use super::{
    execute_tool_call, parse_http_json_response, run_tool_hooks, run_turn_hooks,
    send_http_request_raw, ToolExecutionBackend, ToolInvocation, TurnStreamEvent, APP_VERSION,
    OPENAI_CHATGPT_BASE_URL,
};
use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_provider_openai::{
    build_chat_completions_request, build_json_post_request, build_responses_request,
    build_tool_responses_request, extract_chat_completions_text,
    extract_chat_completions_tool_calls, extract_responses_text, extract_responses_tool_calls,
    parse_chat_completions_response, parse_responses_response, refresh_oauth_token, OpenAIAuth,
    OpenAIChatCompletionTool, OpenAIChatCompletionToolFunction, OpenAIChatCompletionsRequest,
    OpenAIChatFunctionCall, OpenAIChatMessage, OpenAIChatToolCall, OpenAIRequestConfig,
    OpenAIResponseToolCall, OpenAIResponsesFunctionCallOutput, OpenAIResponsesRequest,
    OpenAIResponsesResponse, OpenAIResponsesTool, OpenAIResponsesToolChoice,
    OpenAIResponsesToolChoiceMode, OpenAIResponsesToolRequest,
};
use puffer_provider_registry::{
    AuthStore, OAuthCredential, ProviderDescriptor, ProviderRegistry, StoredCredential,
};
use puffer_resources::LoadedResources;
use puffer_tools::ToolRegistry;
use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use serde_json::{json, Value};

pub(super) use super::openai_sse::{
    is_event_stream, parse_openai_sse_reader, parse_openai_sse_response,
    parse_openai_sse_response_streaming,
};

const OPENAI_CODEX_ORIGINATOR: &str = "codex_cli_rs";

#[derive(Debug, Clone)]
pub(super) struct OpenAIExecutionConfig {
    pub(super) provider_id: String,
    pub(super) request_config: OpenAIRequestConfig,
    pub(super) refresh_token: Option<String>,
    pub(super) codex_style: bool,
}

pub(super) struct OpenAIToolResults {
    pub(super) outputs: Vec<OpenAIResponsesFunctionCallOutput>,
    pub(super) invocations: Vec<ToolInvocation>,
}

pub(super) fn execute_openai(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &mut AuthStore,
    input: &str,
) -> Result<super::TurnExecution> {
    let mut execution = resolve_openai_execution_config(state, auth_store, provider)?;
    let registry = ToolRegistry::from_resources(resources);
    let tools = openai_tool_definitions(&registry);
    let mut previous_response_id = None;
    let mut next_input = transcript_to_openai_input(state, input)?;
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
                        include: Vec::new(),
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
            return Ok(super::TurnExecution {
                assistant_text,
                tool_invocations: invocations,
            });
        }

        let response_id = parsed
            .id
            .clone()
            .ok_or_else(|| anyhow!("OpenAI response missing id for tool continuation"))?;
        let cwd = state.cwd.clone();
        let tool_results = execute_openai_tool_calls(
            state,
            resources,
            providers,
            auth_store,
            &tool_calls,
            &registry,
            &cwd,
            &execution.request_config,
            &model_id,
        )?;
        invocations.extend(tool_results.invocations);
        previous_response_id = Some(response_id);
        next_input = json!(tool_results.outputs);
    }

    bail!("openai tool loop exceeded iteration limit")
}

pub(super) fn execute_openai_streaming<F>(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &mut AuthStore,
    input: &str,
    on_event: &mut F,
) -> Result<super::TurnExecution>
where
    F: FnMut(TurnStreamEvent),
{
    let mut execution = resolve_openai_execution_config(state, auth_store, provider)?;
    if !execution.codex_style {
        return execute_openai(
            state, resources, providers, provider, model_id, auth_store, input,
        );
    }

    let registry = ToolRegistry::from_resources(resources);
    let tools = openai_tool_definitions(&registry);
    let mut previous_response_id = None;
    let mut next_input = transcript_to_openai_input(state, input)?;
    let mut invocations = Vec::new();
    let supports_reasoning = openai_model_supports_reasoning(provider, &model_id);

    for _ in 0..8 {
        let response = send_openai_request_with_refresh_streaming(
            auth_store,
            &mut execution,
            |request_config| {
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
            },
            on_event,
        )?;

        let parsed = parse_responses_response(&serde_json::to_string(&response)?)?;
        let tool_calls = extract_responses_tool_calls(&parsed)?;
        if tool_calls.is_empty() {
            let assistant_text = parse_openai_assistant_text(&parsed, &response, state)?;
            run_turn_hooks(resources, &state.cwd, &assistant_text, invocations.len());
            return Ok(super::TurnExecution {
                assistant_text,
                tool_invocations: invocations,
            });
        }

        let response_id = parsed
            .id
            .clone()
            .ok_or_else(|| anyhow!("OpenAI response missing id for tool continuation"))?;
        let cwd = state.cwd.clone();
        let tool_results = execute_openai_tool_calls(
            state,
            resources,
            providers,
            auth_store,
            &tool_calls,
            &registry,
            &cwd,
            &execution.request_config,
            &model_id,
        )?;
        if !tool_results.invocations.is_empty() {
            on_event(TurnStreamEvent::ToolInvocations(
                tool_results.invocations.clone(),
            ));
        }
        invocations.extend(tool_results.invocations);
        previous_response_id = Some(response_id);
        next_input = json!(tool_results.outputs);
    }

    bail!("openai tool loop exceeded iteration limit")
}

pub(super) fn execute_openai_completions(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &mut AuthStore,
    input: &str,
) -> Result<super::TurnExecution> {
    let mut execution = resolve_openai_execution_config(state, auth_store, provider)?;
    let registry = ToolRegistry::from_resources(resources);
    let tools = openai_chat_completion_tools(&registry);
    let mut messages = transcript_to_openai_chat_messages(state, input)?;
    let mut invocations = Vec::new();

    for _ in 0..8 {
        let response =
            send_openai_request_with_refresh(auth_store, &mut execution, |request_config| {
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
            return Ok(super::TurnExecution {
                assistant_text,
                tool_invocations: invocations,
            });
        }

        let cwd = state.cwd.clone();
        let tool_results = execute_openai_tool_calls(
            state,
            resources,
            providers,
            auth_store,
            &tool_calls,
            &registry,
            &cwd,
            &execution.request_config,
            &model_id,
        )?;
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

pub(super) fn openai_tool_definitions(registry: &ToolRegistry) -> Vec<OpenAIResponsesTool> {
    registry
        .definitions()
        .filter(|definition| definition.handler != "runtime:workflow:structured_output")
        .map(|definition| OpenAIResponsesTool {
            kind: "function".to_string(),
            name: definition.id.clone(),
            description: definition.description.clone(),
            parameters: openai_compatible_schema(definition.input_schema.as_json_schema()),
            filters: None,
            user_location: None,
            external_web_access: None,
        })
        .collect()
}

fn openai_chat_completion_tools(registry: &ToolRegistry) -> Vec<OpenAIChatCompletionTool> {
    registry
        .definitions()
        .filter(|definition| definition.handler != "runtime:workflow:structured_output")
        .map(|definition| OpenAIChatCompletionTool {
            kind: "function".to_string(),
            function: OpenAIChatCompletionToolFunction {
                name: definition.id.clone(),
                description: definition.description.clone(),
                parameters: openai_compatible_schema(definition.input_schema.as_json_schema()),
            },
        })
        .collect()
}

fn openai_compatible_schema(schema: Value) -> Value {
    match schema {
        Value::Object(mut object) => {
            if object.get("type").and_then(Value::as_str) == Some("array") {
                let items = object
                    .remove("items")
                    .map(openai_compatible_schema)
                    .unwrap_or_else(|| json!({ "type": "string" }));
                object.insert("items".to_string(), items);
            }

            if let Some(Value::Object(properties)) = object.get_mut("properties") {
                for property in properties.values_mut() {
                    let normalized = openai_compatible_schema(property.take());
                    *property = normalized;
                }
            }

            if let Some(items) = object.get_mut("items") {
                let normalized = openai_compatible_schema(items.take());
                *items = normalized;
            }

            Value::Object(object)
        }
        value => value,
    }
}

pub(super) fn execute_openai_tool_calls(
    state: &mut AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &mut AuthStore,
    tool_calls: &[OpenAIResponseToolCall],
    registry: &ToolRegistry,
    cwd: &std::path::Path,
    request_config: &OpenAIRequestConfig,
    model_id: &str,
) -> Result<OpenAIToolResults> {
    let mut outputs = Vec::new();
    let mut invocations = Vec::new();
    for tool_call in tool_calls {
        let execution = execute_tool_call(
            state,
            resources,
            providers,
            auth_store,
            registry,
            model_id,
            cwd,
            ToolExecutionBackend::OpenAi { request_config },
            &tool_call.name,
            tool_call.arguments.clone(),
        )?;
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

pub(super) fn transcript_to_openai_input(state: &AppState, input: &str) -> Result<Value> {
    let plan_mode_context = crate::command_helpers::prompt::plan_mode_context_message(state)?;
    if state.transcript.is_empty() && plan_mode_context.is_none() {
        return Ok(Value::String(input.to_string()));
    }

    let mut items = Vec::new();
    if let Some(plan_mode_context) = plan_mode_context {
        items.push(json!({
            "role": "system",
            "content": plan_mode_context,
        }));
    }
    if state.transcript.is_empty() {
        items.push(json!({
            "role": "user",
            "content": [
                {
                    "type": "input_text",
                    "text": input,
                }
            ],
        }));
        return Ok(Value::Array(items));
    }

    items.extend(
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
            }),
    );
    Ok(Value::Array(items))
}

pub(super) fn transcript_to_openai_chat_messages(
    state: &AppState,
    input: &str,
) -> Result<Vec<OpenAIChatMessage>> {
    let plan_mode_context = crate::command_helpers::prompt::plan_mode_context_message(state)?;
    let mut messages = Vec::new();
    if let Some(plan_mode_context) = plan_mode_context {
        messages.push(OpenAIChatMessage {
            role: "system".to_string(),
            content: Some(json!(plan_mode_context)),
            tool_call_id: None,
            tool_calls: Vec::new(),
        });
    }
    messages.extend(
        state
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
            .collect::<Vec<_>>(),
    );
    if messages.is_empty() {
        messages.push(OpenAIChatMessage {
            role: "user".to_string(),
            content: Some(json!(input)),
            tool_call_id: None,
            tool_calls: Vec::new(),
        });
    }
    Ok(messages)
}

pub(super) fn parse_openai_assistant_text(
    parsed: &OpenAIResponsesResponse,
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

pub(super) fn parse_openai_text_fallback(response: &Value, state: &AppState) -> Result<String> {
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

pub(super) fn resolve_openai_execution_config(
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
                base_url: openai_base_url_for_auth(provider, true),
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

pub(super) fn send_openai_request_with_refresh<F>(
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

fn send_openai_request_with_refresh_streaming<F, G>(
    auth_store: &mut AuthStore,
    execution: &mut OpenAIExecutionConfig,
    build_request: F,
    on_event: &mut G,
) -> Result<Value>
where
    F: Fn(&OpenAIRequestConfig) -> Result<puffer_provider_openai::BuiltOpenAIRequest>,
    G: FnMut(TurnStreamEvent),
{
    let request = build_request(&execution.request_config)?;
    let response = send_openai_request_stream_raw(&request.url, &request.headers, &request.body)?;
    if response.status() != StatusCode::UNAUTHORIZED || execution.refresh_token.is_none() {
        return parse_openai_stream_response(&request.url, response, on_event);
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
    let retry_response = send_openai_request_stream_raw(&retry.url, &retry.headers, &retry.body)?;
    parse_openai_stream_response(&retry.url, retry_response, on_event)
}

fn send_openai_request_stream_raw(
    url: &str,
    headers: &[(String, String)],
    body: &str,
) -> Result<Response> {
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
    request
        .body(body.to_string())
        .send()
        .with_context(|| format!("request to {url} failed"))
}

fn parse_openai_stream_response<G>(url: &str, response: Response, on_event: &mut G) -> Result<Value>
where
    G: FnMut(TurnStreamEvent),
{
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    if !status.is_success() {
        let text = response.text()?;
        bail!("request failed with status {}: {}", status, text);
    }
    if is_event_stream(content_type.as_deref(), "") {
        return parse_openai_sse_reader(std::io::BufReader::new(response), on_event)
            .with_context(|| format!("failed to parse SSE response from {url}"));
    }
    let text = response.text()?;
    serde_json::from_str::<Value>(&text)
        .with_context(|| format!("response from {url} was not valid JSON"))
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

pub(super) fn openai_responses_path(base_url: &str) -> &'static str {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.contains("/backend-api") || trimmed.contains("/api/codex") {
        "/responses"
    } else {
        "/v1/responses"
    }
}

pub(super) fn openai_model_supports_reasoning(
    provider: &ProviderDescriptor,
    model_id: &str,
) -> bool {
    provider
        .models
        .iter()
        .find(|model| model.id == model_id)
        .map(|model| model.supports_reasoning)
        .unwrap_or(false)
}

pub(super) fn build_codex_openai_request_body(
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
        organization_name: None,
        organization_role: None,
        workspace_role: None,
    }
}
