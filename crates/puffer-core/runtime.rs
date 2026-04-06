use crate::AppState;
use anyhow::{anyhow, bail, Context, Result};
use puffer_provider_openai::{
    build_responses_request, OpenAIAuth, OpenAIRequestConfig, OpenAIResponsesRequest,
};
use puffer_provider_registry::{
    AuthStore, OAuthCredential, ProviderDescriptor, ProviderRegistry, StoredCredential,
};
use puffer_resources::LoadedResources;
use puffer_tools::{
    BashToolInput, ReadFileToolInput, ToolInput, ToolKind, ToolRegistry, WriteFileToolInput,
};
use puffer_transport_anthropic::{
    build_messages_request, AnthropicAuth, AnthropicMessage, AnthropicModelRequest,
    AnthropicRequestConfig,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};

/// Executes one user prompt against the currently selected provider and model.
pub fn execute_user_prompt(
    state: &AppState,
    resources: &LoadedResources,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    input: &str,
) -> Result<String> {
    let (provider, model_id) = resolve_provider_and_model(state, providers)?;
    match provider.id.as_str() {
        "anthropic" => execute_anthropic(state, resources, provider, model_id, auth_store, input),
        "openai" => execute_openai(state, provider, model_id, auth_store, input),
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
) -> Result<String> {
    let auth = anthropic_auth_for_provider(auth_store, &provider.id)?;
    let tool_registry = ToolRegistry::from_resources(resources);
    let tool_definitions = anthropic_tool_definitions(&tool_registry);
    let mut messages = vec![json!({
        "role": "user",
        "content": input,
    })];

    for _ in 0..8 {
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
            "system": [
                {
                    "type": "text",
                    "text": request.attribution_prefix_block,
                }
            ],
            "messages": messages,
        });
        if !tool_definitions.is_empty() {
            body["tools"] = Value::Array(tool_definitions.clone());
        }

        let response = send_http_request(&request.url, &request.headers, &body.to_string(), true)?;
        let content = response
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| anyhow!("anthropic response missing content array"))?;
        let tool_calls = parse_anthropic_tool_calls(&content, &tool_registry)?;
        if tool_calls.is_empty() {
            return parse_anthropic_text(&response);
        }

        messages.push(json!({
            "role": "assistant",
            "content": content,
        }));
        let tool_results = tool_calls
            .into_iter()
            .map(|call| {
                let result = tool_registry.execute(&call.tool_id, &state.cwd, call.input)?;
                let text = format!("{}{}", result.output.stdout, result.output.stderr);
                Ok::<Value, anyhow::Error>(json!({
                    "type": "tool_result",
                    "tool_use_id": call.tool_use_id,
                    "content": text,
                    "is_error": !result.success,
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        messages.push(json!({
            "role": "user",
            "content": tool_results,
        }));
    }

    bail!("anthropic tool loop exceeded iteration limit")
}

fn execute_openai(
    state: &AppState,
    provider: &ProviderDescriptor,
    model_id: String,
    auth_store: &AuthStore,
    input: &str,
) -> Result<String> {
    let auth = openai_auth_for_provider(auth_store, &provider.id)?;
    let request = build_responses_request(
        &OpenAIRequestConfig {
            base_url: provider.base_url.clone(),
            version: "0.1.0".to_string(),
            auth,
        },
        &OpenAIResponsesRequest {
            model: model_id,
            input: input.to_string(),
        },
    )?;
    let response = send_http_request(&request.url, &request.headers, &request.body, false)?;
    parse_openai_text(&response).or_else(|_| parse_openai_text_fallback(&response, state))
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

fn anthropic_tool_definitions(tool_registry: &ToolRegistry) -> Vec<Value> {
    tool_registry
        .tools()
        .map(|tool| {
            json!({
                "name": tool.spec.name,
                "description": tool.spec.description,
                "input_schema": anthropic_tool_schema(tool.kind),
            })
        })
        .collect()
}

fn anthropic_tool_schema(kind: ToolKind) -> Value {
    match kind {
        ToolKind::Bash => json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"],
        }),
        ToolKind::ReadFile => json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
        }),
        ToolKind::WriteFile => json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "contents": { "type": "string" }
            },
            "required": ["path", "contents"],
        }),
    }
}

fn parse_anthropic_tool_calls(
    content: &[Value],
    tool_registry: &ToolRegistry,
) -> Result<Vec<AnthropicToolCall>> {
    let mut calls = Vec::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("anthropic tool_use missing name"))?;
        let tool_use_id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("anthropic tool_use missing id"))?
            .to_string();
        let tool = tool_registry
            .tools()
            .find(|tool| tool.spec.name == name || tool.spec.id == name)
            .ok_or_else(|| anyhow!("unknown anthropic tool {name}"))?;
        let input = tool_input_from_json(
            tool.kind,
            item.get("input")
                .ok_or_else(|| anyhow!("anthropic tool_use missing input"))?,
        )?;
        calls.push(AnthropicToolCall {
            tool_id: tool.spec.id.clone(),
            tool_use_id,
            input,
        });
    }
    Ok(calls)
}

fn tool_input_from_json(kind: ToolKind, value: &Value) -> Result<ToolInput> {
    match kind {
        ToolKind::Bash => Ok(ToolInput::Bash(BashToolInput {
            command: value
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("bash tool input missing command"))?
                .to_string(),
        })),
        ToolKind::ReadFile => Ok(ToolInput::ReadFile(ReadFileToolInput {
            path: value
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("read_file tool input missing path"))?
                .into(),
        })),
        ToolKind::WriteFile => Ok(ToolInput::WriteFile(WriteFileToolInput {
            path: value
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("write_file tool input missing path"))?
                .into(),
            contents: value
                .get("contents")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("write_file tool input missing contents"))?
                .to_string(),
        })),
    }
}

struct AnthropicToolCall {
    tool_id: String,
    tool_use_id: String,
    input: ToolInput,
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

fn anthropic_tool_definitions(registry: &ToolRegistry) -> Vec<Value> {
    registry
        .tools()
        .map(|tool| {
            let input_schema = match tool.spec.handler.as_str() {
                "bash" => json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute",
                        }
                    },
                    "required": ["command"],
                }),
                "read_file" => json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to read",
                        }
                    },
                    "required": ["path"],
                }),
                "write_file" => json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to write",
                        },
                        "contents": {
                            "type": "string",
                            "description": "Text to write to the file",
                        }
                    },
                    "required": ["path", "contents"],
                }),
                _ => json!({
                    "type": "object",
                    "properties": {},
                }),
            };
            json!({
                "name": tool.spec.id,
                "description": tool.spec.description,
                "input_schema": input_schema,
            })
        })
        .collect()
}

fn parse_anthropic_tool_calls(
    content: &[Value],
    registry: &ToolRegistry,
) -> Result<Vec<ParsedAnthropicToolCall>> {
    let mut calls = Vec::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let tool_id = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("anthropic tool_use block missing name"))?;
        if registry.tool(tool_id).is_none() {
            bail!("anthropic requested unknown tool {tool_id}");
        }
        let tool_use_id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("anthropic tool_use block missing id"))?
            .to_string();
        let input = item
            .get("input")
            .ok_or_else(|| anyhow!("anthropic tool_use block missing input"))?;
        calls.push(ParsedAnthropicToolCall {
            tool_id: tool_id.to_string(),
            tool_use_id,
            input: parse_tool_input(tool_id, input)?,
        });
    }
    Ok(calls)
}

fn parse_tool_input(tool_id: &str, input: &Value) -> Result<ToolInput> {
    match tool_id {
        "bash" => {
            let command = input
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("bash tool input missing command"))?;
            Ok(ToolInput::Bash(BashToolInput {
                command: command.to_string(),
            }))
        }
        "read_file" => {
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("read_file tool input missing path"))?;
            Ok(ToolInput::ReadFile(ReadFileToolInput {
                path: path.into(),
            }))
        }
        "write_file" => {
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("write_file tool input missing path"))?;
            let contents = input
                .get("contents")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("write_file tool input missing contents"))?;
            Ok(ToolInput::WriteFile(WriteFileToolInput {
                path: path.into(),
                contents: contents.to_string(),
            }))
        }
        other => bail!("tool input parsing is not implemented for {other}"),
    }
}

struct ParsedAnthropicToolCall {
    tool_id: String,
    tool_use_id: String,
    input: ToolInput,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_tool_schema_lists_expected_fields() {
        let schema = anthropic_tool_schema(ToolKind::WriteFile);
        let required = schema.get("required").and_then(Value::as_array).unwrap();
        assert!(required.iter().any(|value| value == "path"));
        assert!(required.iter().any(|value| value == "contents"));
    }
}
