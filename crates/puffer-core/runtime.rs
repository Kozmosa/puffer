use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_provider_openai::{
    build_responses_request, build_tool_responses_request, extract_responses_text,
    extract_responses_tool_calls, parse_responses_response, OpenAIAuth, OpenAIRequestConfig,
    OpenAIResponsesFunctionCallOutput, OpenAIResponsesRequest, OpenAIResponsesTool,
    OpenAIResponsesToolChoice, OpenAIResponsesToolChoiceMode, OpenAIResponsesToolRequest,
};
use puffer_provider_registry::{
    AuthStore, OAuthCredential, ProviderDescriptor, ProviderRegistry, StoredCredential,
};
use puffer_resources::LoadedResources;
use puffer_tools::ToolRegistry;
use puffer_transport_anthropic::{
    build_messages_request, AnthropicAuth, AnthropicMessage, AnthropicModelRequest,
    AnthropicRequestConfig,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};

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
    auth_store: &AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let (provider, model_id) = resolve_provider_and_model(state, providers)?;
    match provider.id.as_str() {
        "anthropic" => execute_anthropic(state, resources, provider, model_id, auth_store, input),
        "openai" => execute_openai(state, resources, provider, model_id, auth_store, input),
        other => bail!("provider {other} is not executable yet"),
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

fn execute_anthropic(
    state: &AppState,
    resources: &LoadedResources,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let auth = anthropic_auth_for_provider(auth_store, &provider.id)?;
    let registry = ToolRegistry::from_resources(resources);
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
            version: "0.1.0".to_string(),
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
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: input.to_string(),
            }],
        },
    )?;

    let mut body = json!({
        "model": model_id,
        "max_tokens": 1024,
        "messages": [
            {
                "role": "user",
                "content": input,
            }
        ],
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
    if let Some(tool_results) = execute_anthropic_tool_calls(&response, &registry, &state.cwd)? {
        let follow_up_body = json!({
            "model": model_id,
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": input,
                },
                {
                    "role": "assistant",
                    "content": response.get("content").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
                },
                {
                    "role": "user",
                    "content": tool_results.results,
                }
            ],
            "system": [
                {
                    "type": "text",
                    "text": request.attribution_prefix_block,
                }
            ],
            "tools": anthropic_tool_definitions(&registry),
        });
        let follow_up = send_http_request(
            &request.url,
            &request.headers,
            &follow_up_body.to_string(),
            true,
        )?;
        return Ok(TurnExecution {
            assistant_text: parse_anthropic_text(&follow_up)?,
            tool_invocations: tool_results.invocations,
        });
    }

    Ok(TurnExecution {
        assistant_text: parse_anthropic_text(&response)?,
        tool_invocations: Vec::new(),
    })
}

fn execute_openai(
    state: &AppState,
    resources: &LoadedResources,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &AuthStore,
    input: &str,
) -> Result<TurnExecution> {
    let auth = openai_auth_for_provider(auth_store, &provider.id)?;
    let request_config = OpenAIRequestConfig {
        base_url: provider.base_url.clone(),
        version: "0.1.0".to_string(),
        auth,
    };
    let registry = ToolRegistry::from_resources(resources);
    let tools = openai_tool_definitions(&registry);
    let response = if tools.is_empty() {
        let request = build_responses_request(
            &request_config,
            &OpenAIResponsesRequest {
                model: model_id.clone(),
                input: input.to_string(),
            },
        )?;
        send_http_request(&request.url, &request.headers, &request.body, false)?
    } else {
        let request = build_tool_responses_request(
            &request_config,
            &OpenAIResponsesToolRequest {
                model: model_id.clone(),
                input: json!(input),
                tools,
                tool_choice: Some(OpenAIResponsesToolChoice::Mode(
                    OpenAIResponsesToolChoiceMode::Auto,
                )),
                previous_response_id: None,
            },
        )?;
        send_http_request(&request.url, &request.headers, &request.body, false)?
    };

    let parsed = parse_responses_response(&serde_json::to_string(&response)?)?;
    let tool_calls = extract_responses_tool_calls(&parsed)?;
    if tool_calls.is_empty() {
        return Ok(TurnExecution {
            assistant_text: parse_openai_text(&response)
                .or_else(|_| parse_openai_text_fallback(&response, state))?,
            tool_invocations: Vec::new(),
        });
    }

    let tool_results = execute_openai_tool_calls(&tool_calls, &registry, &state.cwd)?;
    let follow_up = build_tool_responses_request(
        &request_config,
        &OpenAIResponsesToolRequest {
            model: model_id.clone(),
            input: json!(tool_results.outputs),
            tools: openai_tool_definitions(&registry),
            tool_choice: Some(OpenAIResponsesToolChoice::Mode(
                OpenAIResponsesToolChoiceMode::Auto,
            )),
            previous_response_id: parsed.id.clone(),
        },
    )?;
    let follow_up_response = send_http_request(
        &follow_up.url,
        &follow_up.headers,
        &follow_up.body,
        false,
    )?;
    let follow_up_parsed = parse_responses_response(&serde_json::to_string(&follow_up_response)?)?;
    let assistant_text = {
        let text = extract_responses_text(&follow_up_parsed);
        if text.trim().is_empty() {
            parse_openai_text(&follow_up_response)
                .or_else(|_| parse_openai_text_fallback(&follow_up_response, state))?
        } else {
            text
        }
    };
    Ok(TurnExecution {
        assistant_text,
        tool_invocations: tool_results.invocations,
    })
}

fn send_http_request(
    url: &str,
    headers: &[(String, String)],
    body: &str,
    anthropic: bool,
) -> Result<Value> {
    let client = Client::new();
    let mut request = client.post(url);
    for (key, value) in headers {
        request = request.header(key, value);
    }
    request = request.header("content-type", "application/json");
    if anthropic {
        request = request.header("anthropic-version", "2023-06-01");
    }
    let response = request
        .body(body.to_string())
        .send()
        .with_context(|| format!("request to {url} failed"))?;
    let status = response.status();
    let text = response.text()?;
    if !status.is_success() {
        bail!("request failed with status {}: {}", status, text);
    }
    let json = serde_json::from_str::<Value>(&text)
        .with_context(|| format!("response from {url} was not valid JSON"))?;
    Ok(json)
}

fn anthropic_auth_for_provider(auth_store: &AuthStore, provider_id: &str) -> Result<AnthropicAuth> {
    match auth_store.get(provider_id) {
        Some(StoredCredential::ApiKey { key }) => Ok(AnthropicAuth::ApiKey(key.clone())),
        Some(StoredCredential::OAuth(OAuthCredential { access_token, .. })) => {
            Ok(AnthropicAuth::OAuthBearer(access_token.clone()))
        }
        None => bail!(
            "no credentials configured for provider {provider_id}; use `puffer auth set-api-key {provider_id}` first"
        ),
    }
}

fn openai_auth_for_provider(auth_store: &AuthStore, provider_id: &str) -> Result<OpenAIAuth> {
    match auth_store.get(provider_id) {
        Some(StoredCredential::ApiKey { key }) => Ok(OpenAIAuth::ApiKey(key.clone())),
        Some(StoredCredential::OAuth(OAuthCredential { access_token, .. })) => {
            Ok(OpenAIAuth::OAuthBearer(access_token.clone()))
        }
        None => bail!(
            "no credentials configured for provider {provider_id}; use `puffer auth set-api-key {provider_id}` first"
        ),
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
#[cfg(test)]
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

fn execute_openai_tool_calls(
    tool_calls: &[puffer_provider_openai::OpenAIResponseToolCall],
    registry: &ToolRegistry,
    cwd: &std::path::Path,
) -> Result<OpenAIToolResults> {
    let mut outputs = Vec::new();
    let mut invocations = Vec::new();
    for tool_call in tool_calls {
        let execution = registry.execute_json(&tool_call.name, cwd, tool_call.arguments.clone())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_config::PufferConfig;
    use puffer_provider_openai::OpenAIResponseToolCall;
    use puffer_provider_registry::{AuthMode, ProviderDescriptor};
    use puffer_resources::{LoadedItem, LoadedResources, SourceInfo, SourceKind, ToolSpec};
    use puffer_session_store::SessionMetadata;
    use uuid::Uuid;

    fn provider() -> ProviderDescriptor {
        ProviderDescriptor {
            id: "anthropic".to_string(),
            display_name: "Anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            default_api: "anthropic-messages".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: Default::default(),
            models: vec![puffer_provider_registry::ModelDescriptor {
                id: "claude-sonnet-4-5".to_string(),
                display_name: "Claude Sonnet 4.5".to_string(),
                provider: "anthropic".to_string(),
                api: "anthropic-messages".to_string(),
                context_window: 200_000,
                max_output_tokens: 8192,
                supports_reasoning: true,
            }],
        }
    }

    fn state() -> AppState {
        AppState::new(
            PufferConfig::default(),
            std::env::current_dir().unwrap(),
            SessionMetadata {
                id: Uuid::nil(),
                display_name: None,
                cwd: std::env::current_dir().unwrap(),
                created_at_ms: 0,
                updated_at_ms: 0,
                parent_session_id: None,
                slug: None,
                tags: Vec::new(),
                note: None,
            },
        )
    }

    #[test]
    fn anthropic_tool_schema_lists_expected_fields() {
        let schema = anthropic_tool_schema("write_file");
        let required = schema.get("required").and_then(Value::as_array).unwrap();
        assert!(required.iter().any(|value| value == "path"));
        assert!(required.iter().any(|value| value == "contents"));
    }

    #[test]
    fn resolve_selection_uses_first_provider_model_when_unset() {
        let mut registry = ProviderRegistry::new();
        registry.register(provider());
        let state = state();
        let (provider, model_id) = resolve_provider_and_model(&state, &registry).unwrap();
        assert_eq!(provider.id, "anthropic");
        assert_eq!(model_id, "claude-sonnet-4-5");
    }

    #[test]
    fn execute_anthropic_tool_calls_runs_registered_tools() {
        let resources = LoadedResources {
            tools: vec![LoadedItem {
                value: ToolSpec {
                    id: "bash".to_string(),
                    name: "bash".to_string(),
                    description: "Run shell".to_string(),
                    handler: "bash".to_string(),
                    approval_policy: None,
                    sandbox_policy: None,
                },
                source_info: SourceInfo {
                    path: "bash.yaml".into(),
                    kind: SourceKind::Builtin,
                },
            }],
            ..LoadedResources::default()
        };
        let registry = ToolRegistry::from_resources(&resources);
        let response = json!({
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "bash",
                    "input": {
                        "command": "printf hi"
                    }
                }
            ]
        });
        let result = execute_anthropic_tool_calls(
            &response,
            &registry,
            std::env::current_dir().unwrap().as_path(),
        )
        .unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn openai_tool_definitions_use_registry_schema() {
        let resources = LoadedResources {
            tools: vec![LoadedItem {
                value: ToolSpec {
                    id: "search_text".to_string(),
                    name: "search_text".to_string(),
                    description: "Search".to_string(),
                    handler: "search_text".to_string(),
                    approval_policy: None,
                    sandbox_policy: None,
                },
                source_info: SourceInfo {
                    path: "search_text.yaml".into(),
                    kind: SourceKind::Builtin,
                },
            }],
            ..LoadedResources::default()
        };
        let registry = ToolRegistry::from_resources(&resources);
        let tools = openai_tool_definitions(&registry);
        assert_eq!(tools[0].name, "search_text");
        assert_eq!(tools[0].kind, "function");
        assert_eq!(tools[0].parameters["type"], "object");
    }

    #[test]
    fn execute_openai_tool_calls_serializes_outputs() {
        let resources = LoadedResources {
            tools: vec![LoadedItem {
                value: ToolSpec {
                    id: "bash".to_string(),
                    name: "bash".to_string(),
                    description: "Run shell".to_string(),
                    handler: "bash".to_string(),
                    approval_policy: None,
                    sandbox_policy: None,
                },
                source_info: SourceInfo {
                    path: "bash.yaml".into(),
                    kind: SourceKind::Builtin,
                },
            }],
            ..LoadedResources::default()
        };
        let registry = ToolRegistry::from_resources(&resources);
        let tool_calls = vec![OpenAIResponseToolCall {
            item_id: Some("fc_1".to_string()),
            status: Some("completed".to_string()),
            call_id: "call_1".to_string(),
            name: "bash".to_string(),
            arguments: json!({ "command": "printf hi" }),
        }];
        let result = execute_openai_tool_calls(
            &tool_calls,
            &registry,
            std::env::current_dir().unwrap().as_path(),
        )
        .unwrap();
        assert_eq!(result.outputs[0].kind, "function_call_output");
        assert_eq!(result.outputs[0].call_id, "call_1");
        assert!(result.outputs[0].output.contains("hi"));
        assert_eq!(result.invocations[0].tool_id, "bash");
    }
}
