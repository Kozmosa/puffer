use crate::hooks::run_resource_hooks;
use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_provider_registry::{
    AuthStore, OAuthCredential, ProviderDescriptor, ProviderRegistry, StoredCredential,
};
use puffer_resources::LoadedResources;
use puffer_tools::ToolRegistry;
use puffer_transport_anthropic::{
    build_messages_request, get_session_ingress_auth, AnthropicAuth, AnthropicMessage,
    AnthropicModelRequest, AnthropicRequestConfig,
};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde_json::{json, Value};

mod agents;
mod claude_tools;
mod local_tools;
mod openai;
mod openai_sse;
mod tool_executor;
#[cfg(test)]
mod agent_runtime_tests;

#[cfg(test)]
use self::openai::{
    build_codex_openai_request_body, execute_openai_tool_calls, openai_tool_definitions,
    parse_openai_sse_response_streaming, resolve_openai_execution_config,
    transcript_to_openai_chat_messages, transcript_to_openai_input,
};
use self::openai::{
    execute_openai, execute_openai_completions, is_event_stream, parse_openai_sse_response,
};
use self::tool_executor::{execute_tool_call, ToolExecutionBackend};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const OPENAI_CHATGPT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

#[derive(Debug)]
struct RawHttpResponse {
    status: StatusCode,
    content_type: Option<String>,
    text: String,
}

/// Describes one tool call executed during a model turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInvocation {
    pub tool_id: String,
    pub input: String,
    pub output: String,
    pub success: bool,
}
/// Stores the visible result of one executed model turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnExecution {
    pub assistant_text: String,
    pub tool_invocations: Vec<ToolInvocation>,
}

/// Describes one incremental event emitted while a model turn is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamEvent {
    TextDelta(String),
    ToolInvocations(Vec<ToolInvocation>),
}

/// Executes one user prompt against the currently selected provider and model.
pub fn execute_user_prompt(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let (provider, model_id) = resolve_provider_and_model(state, providers)?;
    match resolve_model_api(state, providers, provider, &model_id).as_str() {
        "anthropic-messages" => execute_anthropic(
            state, resources, providers, provider, model_id, auth_store, input,
        ),
        "openai-responses" | "azure-openai-responses" | "openai-codex-responses" => execute_openai(
            state, resources, providers, provider, model_id, auth_store, input,
        ),
        "openai-completions" => execute_openai_completions(
            state, resources, providers, provider, model_id, auth_store, input,
        ),
        other => bail!(
            "provider {} with api {other} is not executable yet",
            provider.id
        ),
    }
}

/// Executes one user prompt and emits incremental stream events when the provider supports them.
pub fn execute_user_prompt_streaming<F>(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    input: &str,
    mut on_event: F,
) -> Result<TurnExecution>
where
    F: FnMut(TurnStreamEvent),
{
    let (provider, model_id) = resolve_provider_and_model(state, providers)?;
    match resolve_model_api(state, providers, provider, &model_id).as_str() {
        "openai-responses" | "azure-openai-responses" | "openai-codex-responses" => {
            openai::execute_openai_streaming(
                state,
                resources,
                providers,
                provider,
                model_id,
                auth_store,
                input,
                &mut on_event,
            )
        }
        _ => execute_user_prompt(state, resources, providers, auth_store, input),
    }
}
fn resolve_provider_and_model<'a>(
    state: &AppState,
    providers: &'a ProviderRegistry,
) -> Result<(&'a ProviderDescriptor, String)> {
    if let Some(selected) = &state.current_model {
        if let Some(model) = providers.resolve_model(selected) {
            let provider = providers
                .provider(&model.provider)
                .ok_or_else(|| anyhow!("provider {} not found", model.provider))?;
            return Ok((provider, model.id.clone()));
        }
    }

    if let Some(provider_id) = &state.current_provider {
        let provider = providers
            .provider(provider_id)
            .ok_or_else(|| anyhow!("provider {provider_id} not found"))?;
        let model_id = provider
            .models
            .first()
            .map(|model| model.id.clone())
            .ok_or_else(|| anyhow!("provider {provider_id} has no configured models"))?;
        return Ok((provider, model_id));
    }

    let provider = providers
        .providers()
        .next()
        .ok_or_else(|| anyhow!("no providers are registered"))?;
    let model_id = provider
        .models
        .first()
        .map(|model| model.id.clone())
        .ok_or_else(|| anyhow!("provider {} has no configured models", provider.id))?;
    Ok((provider, model_id))
}

fn resolve_model_api(
    state: &AppState,
    providers: &ProviderRegistry,
    provider: &ProviderDescriptor,
    model_id: &str,
) -> String {
    state
        .current_model
        .as_ref()
        .and_then(|selected| {
            providers
                .resolve_model(selected)
                .map(|model| model.api.clone())
        })
        .or_else(|| {
            provider
                .models
                .iter()
                .find(|model| model.id == model_id)
                .map(|model| model.api.clone())
        })
        .unwrap_or_else(|| provider.default_api.clone())
}
fn execute_anthropic(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &mut AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let auth = anthropic_auth_for_provider(auth_store, provider)?;
    let registry = ToolRegistry::from_resources(resources);
    let mut messages = transcript_to_anthropic_messages(state, input);
    let mut invocations = Vec::new();
    let plan_mode_context = crate::command_helpers::prompt::plan_mode_context_message(state)?;
    let request_config = AnthropicRequestConfig {
        base_url: provider.base_url.clone(),
        session_id: state.session.id.to_string(),
        custom_headers: provider.headers.clone(),
        remote_container_id: None,
        remote_session_id: None,
        client_app: None,
        entrypoint: "cli".to_string(),
        user_type: "external".to_string(),
        version: APP_VERSION.to_string(),
        workload: None,
        additional_protection: false,
        cch_enabled: true,
        auth: auth.clone(),
        beta_header: None,
        client_request_id: None,
    };
    let request = build_messages_request(
        &request_config,
        &AnthropicModelRequest {
            model: model_id.clone(),
            max_tokens: 1024,
            messages: transcript_to_anthropic_request_messages(state, input),
        },
    )?;

    for _ in 0..8 {
        let mut body = json!({
            "model": model_id,
            "max_tokens": 1024,
            "messages": messages,
            "system": anthropic_system_blocks(
                &request.attribution_prefix_block,
                plan_mode_context.as_deref(),
            )
        });

        let tools = anthropic_tool_definitions(&registry);
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }

        let response = send_http_request(&request.url, &request.headers, &body.to_string(), true)?;
        let cwd = state.cwd.clone();
        if let Some(tool_results) = execute_anthropic_tool_calls(
            state,
            resources,
            providers,
            auth_store,
            &response,
            &registry,
            &cwd,
            &request_config,
            &model_id,
        )? {
            invocations.extend(tool_results.invocations);
            messages.push(json!({
                "role": "assistant",
                "content": response
                    .get("content")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
            }));
            messages.push(json!({
                "role": "user",
                "content": tool_results.results,
            }));
            continue;
        }

        let assistant_text = parse_anthropic_text(&response)?;
        run_turn_hooks(resources, &state.cwd, &assistant_text, invocations.len());
        return Ok(TurnExecution {
            assistant_text,
            tool_invocations: invocations,
        });
    }

    bail!("anthropic tool loop exceeded iteration limit")
}
fn send_http_request(
    url: &str,
    headers: &[(String, String)],
    body: &str,
    anthropic: bool,
) -> Result<Value> {
    let response = send_http_request_raw(url, headers, body, anthropic)?;
    parse_http_json_response(url, anthropic, response)
}

fn send_http_request_raw(
    url: &str,
    headers: &[(String, String)],
    body: &str,
    anthropic: bool,
) -> Result<RawHttpResponse> {
    let client = Client::new();
    let mut request = client.post(url);
    for (key, value) in headers {
        request = request.header(key, value);
    }
    if !headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("content-type"))
    {
        request = request.header("content-type", "application/json");
    }
    if anthropic
        && !headers
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("anthropic-version"))
    {
        request = request.header("anthropic-version", "2023-06-01");
    }
    let response = request
        .body(body.to_string())
        .send()
        .with_context(|| format!("request to {url} failed"))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let text = response.text()?;
    Ok(RawHttpResponse {
        status,
        content_type,
        text,
    })
}

fn parse_http_json_response(
    url: &str,
    anthropic: bool,
    response: RawHttpResponse,
) -> Result<Value> {
    if !response.status.is_success() {
        bail!(
            "request failed with status {}: {}",
            response.status,
            response.text
        );
    }
    if !anthropic && is_event_stream(response.content_type.as_deref(), &response.text) {
        return parse_openai_sse_response(&response.text)
            .with_context(|| format!("failed to parse SSE response from {url}"));
    }
    serde_json::from_str::<Value>(&response.text)
        .with_context(|| format!("response from {url} was not valid JSON"))
}

fn anthropic_auth_for_provider(
    auth_store: &AuthStore,
    provider: &ProviderDescriptor,
) -> Result<AnthropicAuth> {
    match auth_store.get(&provider.id) {
        Some(StoredCredential::ApiKey { key }) => Ok(AnthropicAuth::ApiKey(key.clone())),
        Some(StoredCredential::OAuth(OAuthCredential { access_token, .. })) => {
            Ok(AnthropicAuth::OAuthBearer(access_token.clone()))
        }
        None if provider.auth_modes.is_empty() => Ok(AnthropicAuth::None),
        None => get_session_ingress_auth().ok_or_else(|| {
            anyhow!(
                "no credentials configured for provider {}; use `puffer auth set-api-key {}` first",
                provider.id,
                provider.id
            )
        }),
    }
}
fn parse_anthropic_text(response: &Value) -> Result<String> {
    let parts = response
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("anthropic response missing content array"))?
        .iter()
        .filter_map(|item| {
            let item_type = item.get("type").and_then(Value::as_str)?;
            if item_type == "text" {
                item.get("text").and_then(Value::as_str).map(str::to_string)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        bail!("anthropic response did not contain text content");
    }
    Ok(parts.join("\n"))
}
fn anthropic_tool_definitions(registry: &ToolRegistry) -> Vec<Value> {
    registry
        .tools()
        .map(|tool| {
            json!({
                "name": tool.spec.id,
                "description": tool.spec.description,
                "input_schema": tool.spec.input_schema.as_json_schema(),
            })
        })
        .collect()
}
#[cfg(test)]
fn anthropic_tool_schema(handler: &str) -> Value {
    match handler {
        "bash" => json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" }
            },
            "required": ["command"],
        }),
        "read_file" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"],
        }),
        "write_file" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "contents": { "type": "string" }
            },
            "required": ["path", "contents"],
        }),
        "replace_in_file" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old": { "type": "string" },
                "new": { "type": "string" },
                "replace_all": { "type": "boolean" }
            },
            "required": ["path", "old", "new"],
        }),
        "list_dir" => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": [],
        }),
        "search_text" => json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "path": { "type": "string" }
            },
            "required": ["query"],
        }),
        _ => json!({
            "type": "object",
            "properties": {},
        }),
    }
}
fn execute_anthropic_tool_calls(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    response: &Value,
    registry: &ToolRegistry,
    cwd: &std::path::Path,
    request_config: &AnthropicRequestConfig,
    model_id: &str,
) -> Result<Option<AnthropicToolResults>> {
    let Some(content) = response.get("content").and_then(Value::as_array) else {
        return Ok(None);
    };

    let mut results = Vec::new();
    let mut invocations = Vec::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let tool_id = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("anthropic tool_use block missing name"))?;
        let tool_use_id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("anthropic tool_use block missing id"))?;
        let input = item
            .get("input")
            .ok_or_else(|| anyhow!("anthropic tool_use block missing input"))?;
        let execution = execute_tool_call(
            state,
            resources,
            providers,
            auth_store,
            registry,
            model_id,
            cwd,
            ToolExecutionBackend::Anthropic { request_config },
            tool_id,
            input.clone(),
        )?;
        run_tool_hooks(
            resources,
            cwd,
            "tool_end",
            tool_id,
            input,
            execution.success,
            &execution.output.stdout,
            &execution.output.stderr,
        );
        let output_text = if execution.output.stderr.is_empty() {
            execution.output.stdout
        } else if execution.output.stdout.is_empty() {
            execution.output.stderr
        } else {
            format!("{}\n{}", execution.output.stdout, execution.output.stderr)
        };
        results.push(json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": output_text,
            "is_error": !execution.success,
        }));
        invocations.push(ToolInvocation {
            tool_id: tool_id.to_string(),
            input: serde_json::to_string(input)?,
            output: output_text,
            success: execution.success,
        });
    }

    if results.is_empty() {
        Ok(None)
    } else {
        Ok(Some(AnthropicToolResults {
            results: Value::Array(results),
            invocations,
        }))
    }
}

struct AnthropicToolResults {
    results: Value,
    invocations: Vec<ToolInvocation>,
}

fn run_tool_hooks(
    resources: &LoadedResources,
    cwd: &std::path::Path,
    event: &str,
    tool_id: &str,
    input: &Value,
    success: bool,
    stdout: &str,
    stderr: &str,
) {
    run_resource_hooks(
        resources,
        cwd,
        event,
        &[
            ("PUFFER_TOOL_ID", tool_id.to_string()),
            ("PUFFER_TOOL_INPUT", input.to_string()),
            (
                "PUFFER_TOOL_SUCCESS",
                if success { "true" } else { "false" }.to_string(),
            ),
            ("PUFFER_TOOL_STDOUT", stdout.to_string()),
            ("PUFFER_TOOL_STDERR", stderr.to_string()),
        ],
    );
}
fn run_turn_hooks(
    resources: &LoadedResources,
    cwd: &std::path::Path,
    text: &str,
    tool_count: usize,
) {
    run_resource_hooks(
        resources,
        cwd,
        "turn_end",
        &[
            ("PUFFER_TURN_TEXT", text.to_string()),
            ("PUFFER_TURN_TOOL_COUNT", tool_count.to_string()),
        ],
    );
}
fn transcript_to_anthropic_messages(state: &AppState, input: &str) -> Vec<Value> {
    let mut messages = state
        .transcript
        .iter()
        .map(|message| match message.role {
            crate::MessageRole::User => json!({
                "role": "user",
                "content": message.text,
            }),
            crate::MessageRole::Assistant => json!({
                "role": "assistant",
                "content": message.text,
            }),
            crate::MessageRole::System => json!({
                "role": "user",
                "content": format!("[system]\n{}", message.text),
            }),
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        messages.push(json!({
            "role": "user",
            "content": input,
        }));
    }
    messages
}
fn transcript_to_anthropic_request_messages(
    state: &AppState,
    input: &str,
) -> Vec<AnthropicMessage> {
    let mut messages = state
        .transcript
        .iter()
        .map(|message| AnthropicMessage {
            role: match message.role {
                crate::MessageRole::Assistant => "assistant".to_string(),
                crate::MessageRole::User | crate::MessageRole::System => "user".to_string(),
            },
            content: match message.role {
                crate::MessageRole::System => format!("[system]\n{}", message.text),
                _ => message.text.clone(),
            },
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        messages.push(AnthropicMessage {
            role: "user".to_string(),
            content: input.to_string(),
        });
    }
    messages
}
fn anthropic_system_blocks(
    attribution_prefix_block: &str,
    plan_mode_context: Option<&str>,
) -> Vec<Value> {
    let mut blocks = vec![json!({
        "type": "text",
        "text": attribution_prefix_block,
    })];
    if let Some(plan_mode_context) = plan_mode_context {
        blocks.push(json!({
            "type": "text",
            "text": plan_mode_context,
        }));
    }
    blocks
}

#[cfg(test)]
mod tests;
