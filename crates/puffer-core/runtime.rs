use crate::hooks::run_resource_hooks;
use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_provider_openai::{
    build_chat_completions_request, build_json_post_request, build_responses_request,
    build_tool_responses_request, extract_chat_completions_text,
    extract_chat_completions_tool_calls, extract_responses_text, extract_responses_tool_calls,
    parse_chat_completions_response, parse_responses_response, refresh_oauth_token,
    OpenAIAuth, OpenAIChatCompletionTool, OpenAIChatCompletionToolFunction,
    OpenAIChatCompletionsRequest, OpenAIChatFunctionCall, OpenAIChatMessage, OpenAIChatToolCall,
    OpenAIRequestConfig, OpenAIResponsesFunctionCallOutput, OpenAIResponsesRequest,
    OpenAIResponsesTool, OpenAIResponsesToolChoice, OpenAIResponsesToolChoiceMode,
    OpenAIResponsesToolRequest,
};
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

mod mistral;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const OPENAI_CHATGPT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const OPENAI_CODEX_ORIGINATOR: &str = "codex_cli_rs";

#[derive(Debug, Clone)]
struct OpenAIExecutionConfig {
    provider_id: String,
    request_config: OpenAIRequestConfig,
    refresh_token: Option<String>,
    codex_style: bool,
}

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
/// Executes one user prompt against the currently selected provider and model.
pub fn execute_user_prompt(
    state: &AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let (provider, model_id) = resolve_provider_and_model(state, providers)?;
    match resolve_model_api(state, providers, provider, &model_id).as_str() {
        "anthropic-messages" => {
            execute_anthropic(state, resources, provider, model_id, auth_store, input)
        }
        "openai-responses" | "azure-openai-responses" | "openai-codex-responses" => {
            execute_openai(state, resources, provider, model_id, auth_store, input)
        }
        "openai-completions" => {
            execute_openai_completions(state, resources, provider, model_id, auth_store, input)
        }
        "mistral-conversations" => {
            mistral::execute_turn(state, resources, provider, model_id, auth_store, input)
        }
        other => bail!(
            "provider {} with api {other} is not executable yet",
            provider.id
        ),
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
    state: &AppState,
    resources: &LoadedResources,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let auth = anthropic_auth_for_provider(auth_store, provider)?;
    let registry = ToolRegistry::from_resources(resources);
    let mut messages = transcript_to_anthropic_messages(state, input);
    let mut invocations = Vec::new();
    let request = build_messages_request(
        &AnthropicRequestConfig {
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
        },
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
            "system": [
                {
                    "type": "text",
                    "text": request.attribution_prefix_block.clone(),
                }
            ]
        });

        let tools = anthropic_tool_definitions(&registry);
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }

        let response = send_http_request(&request.url, &request.headers, &body.to_string(), true)?;
        if let Some(tool_results) =
            execute_anthropic_tool_calls(resources, &response, &registry, &state.cwd)?
        {
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
fn execute_openai(
    state: &AppState,
    resources: &LoadedResources,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &mut AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let mut execution = resolve_openai_execution_config(state, auth_store, provider)?;
    let registry = ToolRegistry::from_resources(resources);
    let tools = openai_tool_definitions(&registry);
    let mut previous_response_id = None;
    let mut next_input = transcript_to_openai_input(state, input);
    let mut invocations = Vec::new();
    let supports_reasoning = openai_model_supports_reasoning(provider, &model_id);

    for _ in 0..8 {
        let response = if execution.codex_style {
            send_openai_request_with_refresh(auth_store, &mut execution, |request_config| {
                let body = build_codex_openai_request_body(
                    state,
                    &model_id,
                    next_input.clone(),
                    &tools,
                    previous_response_id.as_ref(),
                    supports_reasoning,
                );
                build_json_post_request(
                    request_config,
                    openai_responses_path(&request_config.base_url),
                    &body,
                )
            })?
        } else if tools.is_empty()
            && previous_response_id.is_none()
            && matches!(next_input, Value::String(_))
        {
            send_openai_request_with_refresh(auth_store, &mut execution, |request_config| {
                build_responses_request(
                    request_config,
                    &OpenAIResponsesRequest {
                        model: model_id.clone(),
                        input: next_input.as_str().unwrap_or_default().to_string(),
                    },
                )
            })?
        } else {
            send_openai_request_with_refresh(auth_store, &mut execution, |request_config| {
                build_tool_responses_request(
                    request_config,
                    &OpenAIResponsesToolRequest {
                        model: model_id.clone(),
                        input: next_input.clone(),
                        tools: tools.clone(),
                        tool_choice: if tools.is_empty() {
                            None
                        } else {
                            Some(OpenAIResponsesToolChoice::Mode(
                                OpenAIResponsesToolChoiceMode::Auto,
                            ))
                        },
                        previous_response_id: previous_response_id.clone(),
                    },
                )
            })?
        };

        let parsed = parse_responses_response(&serde_json::to_string(&response)?)?;
        let tool_calls = extract_responses_tool_calls(&parsed)?;
        if tool_calls.is_empty() {
            let assistant_text = parse_openai_assistant_text(&parsed, &response, state)?;
            run_turn_hooks(resources, &state.cwd, &assistant_text, invocations.len());
            return Ok(TurnExecution {
                assistant_text,
                tool_invocations: invocations,
            });
        }

        let response_id = parsed
            .id
            .clone()
            .ok_or_else(|| anyhow!("OpenAI response missing id for tool continuation"))?;
        let tool_results =
            execute_openai_tool_calls(resources, &tool_calls, &registry, &state.cwd)?;
        invocations.extend(tool_results.invocations);
        previous_response_id = Some(response_id);
        next_input = json!(tool_results.outputs);
    }

    bail!("openai tool loop exceeded iteration limit")
}

fn execute_openai_completions(
    state: &AppState,
    resources: &LoadedResources,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &mut AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let mut execution = resolve_openai_execution_config(state, auth_store, provider)?;
    let registry = ToolRegistry::from_resources(resources);
    let tools = openai_chat_completion_tools(&registry);
    let mut messages = transcript_to_openai_chat_messages(state, input);
    let mut invocations = Vec::new();

    for _ in 0..8 {
        let response = send_openai_request_with_refresh(auth_store, &mut execution, |request_config| {
            build_chat_completions_request(
                request_config,
                &OpenAIChatCompletionsRequest {
                    model: model_id.clone(),
                    messages: messages.clone(),
                    tools: tools.clone(),
                    tool_choice: if tools.is_empty() {
                        None
                    } else {
                        Some(OpenAIResponsesToolChoiceMode::Auto)
                    },
                },
            )
        })?;
        let parsed = parse_chat_completions_response(&serde_json::to_string(&response)?)?;
        let tool_calls = extract_chat_completions_tool_calls(&parsed)?;
        let choice = parsed
            .choices
            .first()
            .ok_or_else(|| anyhow!("OpenAI Chat Completions response did not contain choices"))?;
        if tool_calls.is_empty() {
            let text = extract_chat_completions_text(&parsed);
            let assistant_text = if text.trim().is_empty() {
                parse_openai_text(&response)
                    .or_else(|_| parse_openai_text_fallback(&response, state))?
            } else {
                text
            };
            run_turn_hooks(resources, &state.cwd, &assistant_text, invocations.len());
            return Ok(TurnExecution {
                assistant_text,
                tool_invocations: invocations,
            });
        }

        let tool_results =
            execute_openai_tool_calls(resources, &tool_calls, &registry, &state.cwd)?;
        invocations.extend(tool_results.invocations);
        messages.push(OpenAIChatMessage {
            role: choice
                .message
                .role
                .clone()
                .unwrap_or_else(|| "assistant".to_string()),
            content: choice.message.content.clone(),
            tool_call_id: None,
            tool_calls: tool_calls
                .iter()
                .map(|tool_call| OpenAIChatToolCall {
                    id: tool_call.call_id.clone(),
                    kind: "function".to_string(),
                    function: OpenAIChatFunctionCall {
                        name: tool_call.name.clone(),
                        arguments: serde_json::to_string(&tool_call.arguments)
                            .unwrap_or_else(|_| "{}".to_string()),
                    },
                })
                .collect(),
        });
        for output in tool_results.outputs {
            messages.push(OpenAIChatMessage {
                role: "tool".to_string(),
                content: Some(json!(output.output)),
                tool_call_id: Some(output.call_id),
                tool_calls: Vec::new(),
            });
        }
    }

    bail!("openai chat completions tool loop exceeded iteration limit")
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

fn parse_http_json_response(url: &str, anthropic: bool, response: RawHttpResponse) -> Result<Value> {
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
    resources: &LoadedResources,
    response: &Value,
    registry: &ToolRegistry,
    cwd: &std::path::Path,
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
        let execution = registry.execute_json(tool_id, cwd, input.clone())?;
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

struct OpenAIToolResults {
    outputs: Vec<OpenAIResponsesFunctionCallOutput>,
    invocations: Vec<ToolInvocation>,
}
fn openai_tool_definitions(registry: &ToolRegistry) -> Vec<OpenAIResponsesTool> {
    registry
        .definitions()
        .map(|definition| OpenAIResponsesTool {
            kind: "function".to_string(),
            name: definition.id.clone(),
            description: definition.description.clone(),
            parameters: definition.input_schema.as_json_schema(),
        })
        .collect()
}

fn openai_chat_completion_tools(registry: &ToolRegistry) -> Vec<OpenAIChatCompletionTool> {
    registry
        .definitions()
        .map(|definition| OpenAIChatCompletionTool {
            kind: "function".to_string(),
            function: OpenAIChatCompletionToolFunction {
                name: definition.id.clone(),
                description: definition.description.clone(),
                parameters: definition.input_schema.as_json_schema(),
            },
        })
        .collect()
}

fn execute_openai_tool_calls(
    resources: &LoadedResources,
    tool_calls: &[puffer_provider_openai::OpenAIResponseToolCall],
    registry: &ToolRegistry,
    cwd: &std::path::Path,
) -> Result<OpenAIToolResults> {
    let mut outputs = Vec::new();
    let mut invocations = Vec::new();
    for tool_call in tool_calls {
        let execution = registry.execute_json(&tool_call.name, cwd, tool_call.arguments.clone())?;
        run_tool_hooks(
            resources,
            cwd,
            "tool_end",
            &tool_call.name,
            &tool_call.arguments,
            execution.success,
            &execution.output.stdout,
            &execution.output.stderr,
        );
        let output = if execution.output.stderr.is_empty() {
            execution.output.stdout
        } else if execution.output.stdout.is_empty() {
            execution.output.stderr
        } else {
            format!("{}\n{}", execution.output.stdout, execution.output.stderr)
        };
        outputs.push(OpenAIResponsesFunctionCallOutput {
            kind: "function_call_output".to_string(),
            call_id: tool_call.call_id.clone(),
            output: output.clone(),
        });
        invocations.push(ToolInvocation {
            tool_id: tool_call.name.clone(),
            input: serde_json::to_string(&tool_call.arguments)?,
            output,
            success: execution.success,
        });
    }
    Ok(OpenAIToolResults {
        outputs,
        invocations,
    })
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
fn parse_openai_text(response: &Value) -> Result<String> {
    if let Some(text) = response.get("output_text").and_then(Value::as_str) {
        return Ok(text.to_string());
    }

    let mut parts = Vec::new();
    if let Some(items) = response.get("output").and_then(Value::as_array) {
        for item in items {
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for block in content {
                    let block_type = block
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if matches!(block_type, "output_text" | "text") {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            parts.push(text.to_string());
                        }
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        bail!("openai response did not contain output text");
    }
    Ok(parts.join("\n"))
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
fn transcript_to_openai_input(state: &AppState, input: &str) -> Value {
    if state.transcript.is_empty() {
        return Value::String(input.to_string());
    }

    Value::Array(
        state
            .transcript
            .iter()
            .enumerate()
            .map(|(index, message)| match message.role {
                crate::MessageRole::User => json!({
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": message.text,
                        }
                    ],
                }),
                crate::MessageRole::Assistant => json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": message.text,
                            "annotations": [],
                        }
                    ],
                    "status": "completed",
                    "id": format!("msg_{index}"),
                }),
                crate::MessageRole::System => json!({
                    "role": "system",
                    "content": message.text,
                }),
            })
            .collect(),
    )
}

fn transcript_to_openai_chat_messages(state: &AppState, input: &str) -> Vec<OpenAIChatMessage> {
    let mut messages = state
        .transcript
        .iter()
        .map(|message| OpenAIChatMessage {
            role: match message.role {
                crate::MessageRole::User => "user".to_string(),
                crate::MessageRole::Assistant => "assistant".to_string(),
                crate::MessageRole::System => "system".to_string(),
            },
            content: Some(json!(message.text)),
            tool_call_id: None,
            tool_calls: Vec::new(),
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        messages.push(OpenAIChatMessage {
            role: "user".to_string(),
            content: Some(json!(input)),
            tool_call_id: None,
            tool_calls: Vec::new(),
        });
    }
    messages
}

fn parse_openai_assistant_text(
    parsed: &puffer_provider_openai::OpenAIResponsesResponse,
    response: &Value,
    state: &AppState,
) -> Result<String> {
    let text = extract_responses_text(parsed);
    if text.trim().is_empty() {
        parse_openai_text(response).or_else(|_| parse_openai_text_fallback(response, state))
    } else {
        Ok(text)
    }
}
fn parse_openai_text_fallback(response: &Value, state: &AppState) -> Result<String> {
    if let Some(text) = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        return Ok(text);
    }
    bail!(
        "provider {} returned an unsupported response shape for session {}",
        state.current_provider.as_deref().unwrap_or("unknown"),
        state.session.id
    )
}

fn resolve_openai_execution_config(
    state: &AppState,
    auth_store: &AuthStore,
    provider: &ProviderDescriptor,
) -> Result<OpenAIExecutionConfig> {
    let mut custom_headers = provider
        .headers
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();
    append_default_openai_headers(&mut custom_headers, provider.id.as_str());
    let codex_style = is_codex_openai_provider(provider);
    let session_id = Some(state.session.id.to_string());
    let originator = OPENAI_CODEX_ORIGINATOR.to_string();
    match auth_store.get(provider.id.as_str()) {
        Some(StoredCredential::ApiKey { key }) => Ok(OpenAIExecutionConfig {
            provider_id: provider.id.clone(),
            request_config: OpenAIRequestConfig {
                base_url: provider.base_url.clone(),
                version: APP_VERSION.to_string(),
                auth: OpenAIAuth::ApiKey(key.clone()),
                originator,
                session_id,
                account_id: None,
                custom_headers,
                query_params: provider
                    .query_params
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            },
            refresh_token: None,
            codex_style,
        }),
        Some(StoredCredential::OAuth(credential)) => Ok(OpenAIExecutionConfig {
            provider_id: provider.id.clone(),
            request_config: OpenAIRequestConfig {
                base_url: openai_base_url_for_auth(provider, /*oauth*/ true),
                version: APP_VERSION.to_string(),
                auth: OpenAIAuth::OAuthBearer(credential.access_token.clone()),
                originator,
                session_id,
                account_id: credential.account_id.clone(),
                custom_headers,
                query_params: provider
                    .query_params
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            },
            refresh_token: Some(credential.refresh_token.clone()),
            codex_style,
        }),
        None if provider.auth_modes.is_empty() => Ok(OpenAIExecutionConfig {
            provider_id: provider.id.clone(),
            request_config: OpenAIRequestConfig {
                base_url: provider.base_url.clone(),
                version: APP_VERSION.to_string(),
                auth: OpenAIAuth::None,
                originator,
                session_id,
                account_id: None,
                custom_headers,
                query_params: provider
                    .query_params
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            },
            refresh_token: None,
            codex_style,
        }),
        None => bail!(
            "no credentials configured for provider {}; use `puffer auth set-api-key {}` first",
            provider.id,
            provider.id
        ),
    }
}

fn send_openai_request_with_refresh<F>(
    auth_store: &mut AuthStore,
    execution: &mut OpenAIExecutionConfig,
    build_request: F,
) -> Result<Value>
where
    F: Fn(&OpenAIRequestConfig) -> Result<puffer_provider_openai::BuiltOpenAIRequest>,
{
    let request = build_request(&execution.request_config)?;
    let response = send_http_request_raw(&request.url, &request.headers, &request.body, false)?;
    if response.status != StatusCode::UNAUTHORIZED || execution.refresh_token.is_none() {
        return parse_http_json_response(&request.url, false, response);
    }

    let refresh_token = execution
        .refresh_token
        .clone()
        .ok_or_else(|| anyhow!("missing refresh token for OpenAI OAuth retry"))?;
    let refreshed = refresh_oauth_token(&refresh_token)
        .context("failed to refresh OpenAI OAuth credentials after 401")?;
    let stored = openai_registry_credential(refreshed);
    execution.request_config.auth = OpenAIAuth::OAuthBearer(stored.access_token.clone());
    execution.request_config.account_id = stored.account_id.clone();
    execution.refresh_token = Some(stored.refresh_token.clone());
    auth_store.set_oauth(execution.provider_id.clone(), stored);

    let retry = build_request(&execution.request_config)?;
    let retry_response = send_http_request_raw(&retry.url, &retry.headers, &retry.body, false)?;
    parse_http_json_response(&retry.url, false, retry_response)
}

fn append_default_openai_headers(headers: &mut Vec<(String, String)>, provider_id: &str) {
    if provider_id == "openai" && !has_header(headers, "version") {
        headers.push(("version".to_string(), APP_VERSION.to_string()));
    }
    append_env_header(headers, "OpenAI-Organization", "OPENAI_ORGANIZATION");
    append_env_header(headers, "OpenAI-Project", "OPENAI_PROJECT");
}

fn append_env_header(headers: &mut Vec<(String, String)>, header: &str, env_var: &str) {
    if has_header(headers, header) {
        return;
    }
    if let Ok(value) = std::env::var(env_var) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            headers.push((header.to_string(), trimmed.to_string()));
        }
    }
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(header, _)| header.eq_ignore_ascii_case(name))
}

fn is_codex_openai_provider(provider: &ProviderDescriptor) -> bool {
    provider.id == "openai" || provider.default_api == "openai-codex-responses"
}

fn openai_base_url_for_auth(provider: &ProviderDescriptor, oauth: bool) -> String {
    if !oauth || provider.id != "openai" {
        return provider.base_url.clone();
    }
    let trimmed = provider.base_url.trim_end_matches('/');
    if trimmed.contains("/backend-api") || trimmed.contains("/api/codex") {
        trimmed.to_string()
    } else {
        OPENAI_CHATGPT_BASE_URL.to_string()
    }
}

fn openai_responses_path(base_url: &str) -> &'static str {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.contains("/backend-api") || trimmed.contains("/api/codex") {
        "/responses"
    } else {
        "/v1/responses"
    }
}

fn openai_model_supports_reasoning(provider: &ProviderDescriptor, model_id: &str) -> bool {
    provider
        .models
        .iter()
        .find(|model| model.id == model_id)
        .map(|model| model.supports_reasoning)
        .unwrap_or(false)
}

fn build_codex_openai_request_body(
    state: &AppState,
    model_id: &str,
    input: Value,
    tools: &[OpenAIResponsesTool],
    previous_response_id: Option<&String>,
    supports_reasoning: bool,
) -> Value {
    let reasoning = codex_reasoning_config(state, supports_reasoning);
    let include = if reasoning.is_some() {
        vec![json!("reasoning.encrypted_content")]
    } else {
        Vec::new()
    };
    let mut body = json!({
        "model": model_id,
        "instructions": "",
        "input": codex_input_items(input),
        "tools": tools,
        "tool_choice": "auto",
        "parallel_tool_calls": !tools.is_empty(),
        "store": false,
        "stream": true,
        "include": include,
        "prompt_cache_key": state.session.id.to_string(),
    });
    if let Some(reasoning) = reasoning {
        body["reasoning"] = reasoning;
    }
    if let Some(previous_response_id) = previous_response_id {
        body["previous_response_id"] = json!(previous_response_id);
    }
    body
}

fn codex_input_items(input: Value) -> Value {
    match input {
        Value::Array(_) => input,
        Value::String(text) => json!([
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": text,
                    }
                ],
            }
        ]),
        other => other,
    }
}

fn codex_reasoning_config(state: &AppState, supports_reasoning: bool) -> Option<Value> {
    if !supports_reasoning {
        return None;
    }
    let mut reasoning = json!({ "summary": "auto" });
    match state.effort_level.as_str() {
        "low" | "medium" | "high" => {
            reasoning["effort"] = json!(state.effort_level);
        }
        "max" => {
            reasoning["effort"] = json!("high");
        }
        _ => {}
    }
    Some(reasoning)
}

fn is_event_stream(content_type: Option<&str>, text: &str) -> bool {
    content_type
        .is_some_and(|value| value.starts_with("text/event-stream"))
        || text.trim_start().starts_with("event:")
}

fn parse_openai_sse_response(stream: &str) -> Result<Value> {
    let mut response_id = None;
    let mut output = Vec::new();

    for chunk in stream.split("\n\n") {
        let data = chunk
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(&data)
            .with_context(|| format!("invalid SSE payload: {data}"))?;
        match event.get("type").and_then(Value::as_str).unwrap_or_default() {
            "response.created" | "response.completed" => {
                if let Some(id) = event.pointer("/response/id").and_then(Value::as_str) {
                    response_id = Some(id.to_string());
                }
            }
            "response.output_item.done" => {
                if let Some(item) = event.get("item") {
                    output.push(item.clone());
                }
            }
            _ => {}
        }
    }

    Ok(json!({
        "id": response_id,
        "output": output,
    }))
}

fn openai_registry_credential(
    credential: puffer_provider_openai::OpenAIOAuthCredentials,
) -> OAuthCredential {
    OAuthCredential {
        access_token: credential.access_token,
        refresh_token: credential.refresh_token,
        expires_at_ms: credential.expires_at_ms,
        account_id: credential.account_id,
        organization_id: None,
        email: credential.email,
        plan_type: credential.plan_type,
        rate_limit_tier: None,
        scopes: Vec::new(),
    }
}
#[cfg(test)]
mod tests;
