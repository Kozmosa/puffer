use crate::auth::OpenAIAuth;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A minimal OpenAI Responses API request payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAIResponsesRequest {
    pub model: String,
    pub input: String,
}

/// A tool-enabled OpenAI Responses API request payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAIResponsesToolRequest {
    pub model: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAIResponsesTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<OpenAIResponsesToolChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
}

/// A tool definition accepted by the OpenAI Responses API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAIResponsesTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// A tool selection directive for the OpenAI Responses API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum OpenAIResponsesToolChoice {
    Mode(OpenAIResponsesToolChoiceMode),
    Named(OpenAIResponsesNamedToolChoice),
}

/// A simple tool-choice mode for the OpenAI Responses API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenAIResponsesToolChoiceMode {
    Auto,
    None,
    Required,
}

/// A named tool-choice directive for the OpenAI Responses API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAIResponsesNamedToolChoice {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
}

/// A tool-result item accepted by the OpenAI Responses API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAIResponsesFunctionCallOutput {
    #[serde(rename = "type")]
    pub kind: String,
    pub call_id: String,
    pub output: String,
}

/// Runtime request configuration for the OpenAI provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAIRequestConfig {
    pub base_url: String,
    pub version: String,
    pub auth: OpenAIAuth,
}

/// An ordered HTTP request representation for tests and execution adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltOpenAIRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Builds a minimal OpenAI Responses API request with ordered headers.
pub(crate) fn build_responses_request(
    config: &OpenAIRequestConfig,
    request: &OpenAIResponsesRequest,
) -> anyhow::Result<BuiltOpenAIRequest> {
    build_request(config, request)
}

/// Builds a tool-enabled OpenAI Responses API request with ordered headers.
pub(crate) fn build_tool_responses_request(
    config: &OpenAIRequestConfig,
    request: &OpenAIResponsesToolRequest,
) -> anyhow::Result<BuiltOpenAIRequest> {
    build_request(config, request)
}

fn build_request<T: Serialize>(
    config: &OpenAIRequestConfig,
    request: &T,
) -> anyhow::Result<BuiltOpenAIRequest> {
    let mut headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        (
            "User-Agent".to_string(),
            format!("puffer-code/{}", config.version),
        ),
    ];
    match &config.auth {
        OpenAIAuth::ApiKey(key) | OpenAIAuth::OAuthBearer(key) => {
            headers.push(("Authorization".to_string(), format!("Bearer {key}")));
        }
    }
    Ok(BuiltOpenAIRequest {
        method: "POST",
        url: format!("{}/v1/responses", config.base_url.trim_end_matches('/')),
        headers,
        body: serde_json::to_string(request)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn api_key_uses_bearer_auth() {
        let request = build_responses_request(
            &OpenAIRequestConfig {
                base_url: "https://api.openai.com".to_string(),
                version: "0.1.0".to_string(),
                auth: OpenAIAuth::ApiKey("sk-test".to_string()),
            },
            &OpenAIResponsesRequest {
                model: "gpt-5".to_string(),
                input: "hello".to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            request.headers[2],
            ("Authorization".to_string(), "Bearer sk-test".to_string())
        );
    }

    #[test]
    fn tool_request_serializes_tools_and_choice() {
        let request = build_tool_responses_request(
            &OpenAIRequestConfig {
                base_url: "https://api.openai.com".to_string(),
                version: "0.1.0".to_string(),
                auth: OpenAIAuth::OAuthBearer("oauth-token".to_string()),
            },
            &OpenAIResponsesToolRequest {
                model: "gpt-5".to_string(),
                input: json!("inspect Cargo.toml"),
                tools: vec![OpenAIResponsesTool {
                    kind: "function".to_string(),
                    name: "read_file".to_string(),
                    description: "Reads a file from disk.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string"
                            }
                        },
                        "required": ["path"]
                    }),
                }],
                tool_choice: Some(OpenAIResponsesToolChoice::Mode(
                    OpenAIResponsesToolChoiceMode::Auto,
                )),
                previous_response_id: Some("resp_123".to_string()),
            },
        )
        .unwrap();

        let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(body["model"], json!("gpt-5"));
        assert_eq!(body["input"], json!("inspect Cargo.toml"));
        assert_eq!(body["tools"][0]["name"], json!("read_file"));
        assert_eq!(body["tool_choice"], json!("auto"));
        assert_eq!(body["previous_response_id"], json!("resp_123"));
        assert_eq!(
            request.headers[2],
            (
                "Authorization".to_string(),
                "Bearer oauth-token".to_string()
            )
        );
    }
}
