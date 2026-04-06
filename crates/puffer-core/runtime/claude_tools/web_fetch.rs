use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

const CLAUDE_WEB_FETCH_TOOL_DESCRIPTION: &str = r#"- Fetches content from a specified URL and processes it using an AI model
- Takes a URL and a prompt as input
- Fetches the URL content, converts HTML to markdown
- Processes the content with a small, fast model
- Returns the model's response about the content
- Use this tool when you need to retrieve and analyze web content

Usage notes:
  - IMPORTANT: If an MCP-provided web fetch tool is available, prefer using that tool because it may have fewer restrictions.
  - The URL must be a fully-formed valid URL
  - HTTP URLs are automatically upgraded to HTTPS
  - The prompt should describe what information to extract from the page
  - This tool is read-only and does not modify files
  - Results may be summarized if the content is very large
  - Includes a self-cleaning 15-minute cache for repeated fetches
  - If a URL redirects to a different host, run WebFetch again with the redirect URL.
  - For GitHub URLs, prefer using `gh` via Bash (for example `gh pr view`, `gh issue view`, `gh api`)."#;

const MAX_RESULT_CHARS: usize = 100_000;

#[derive(Debug, Deserialize)]
struct ClaudeWebFetchInput {
    url: String,
    prompt: String,
}

/// Returns the Claude-compatible WebFetch tool description text.
pub fn claude_web_fetch_tool_description() -> &'static str {
    CLAUDE_WEB_FETCH_TOOL_DESCRIPTION
}

/// Returns the Claude-compatible WebFetch input JSON schema.
pub fn claude_web_fetch_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "description": "The URL to fetch content from",
            },
            "prompt": {
                "type": "string",
                "description": "The prompt to run on the fetched content",
            },
        },
        "required": ["url", "prompt"],
        "additionalProperties": false,
    })
}

/// Executes a Claude-style WebFetch request and returns Claude-compatible output fields.
pub fn execute_claude_web_fetch(raw_input: Value) -> Result<Value> {
    let input: ClaudeWebFetchInput =
        serde_json::from_value(raw_input).context("invalid WebFetch input")?;
    execute_claude_web_fetch_internal(Client::new(), input)
}

fn execute_claude_web_fetch_internal(client: Client, input: ClaudeWebFetchInput) -> Result<Value> {
    let start = Instant::now();
    let request_url = normalize_url(&input.url)?;

    let response = client
        .get(request_url.clone())
        .send()
        .with_context(|| format!("failed to fetch {}", request_url))?;
    let status = response.status();
    let final_url = response.url().clone();
    let status_text = status_text(status).to_string();

    if host_changed(&request_url, &final_url) {
        let redirect_message =
            build_redirect_message(&request_url, &final_url, status, &input.prompt);
        return Ok(json!({
            "bytes": redirect_message.len(),
            "code": status.as_u16(),
            "codeText": status_text,
            "result": redirect_message,
            "durationMs": start.elapsed().as_millis(),
            "url": request_url.to_string(),
        }));
    }

    let bytes = response
        .bytes()
        .context("failed to read web fetch response body")?;
    let content = String::from_utf8_lossy(&bytes).to_string();
    let result = apply_prompt_to_content(&input.prompt, &content)?;
    Ok(json!({
        "bytes": bytes.len(),
        "code": status.as_u16(),
        "codeText": status_text,
        "result": result,
        "durationMs": start.elapsed().as_millis(),
        "url": request_url.to_string(),
    }))
}

fn normalize_url(raw_url: &str) -> Result<Url> {
    let mut url = Url::parse(raw_url).with_context(|| format!("invalid URL `{raw_url}`"))?;
    if url.scheme() == "http" {
        url.set_scheme("https")
            .map_err(|_| anyhow::anyhow!("failed to upgrade URL scheme to https"))?;
    }
    Ok(url)
}

fn host_changed(request_url: &Url, final_url: &Url) -> bool {
    request_url
        .host_str()
        .zip(final_url.host_str())
        .is_some_and(|(requested, actual)| !requested.eq_ignore_ascii_case(actual))
}

fn status_text(status: StatusCode) -> &'static str {
    status.canonical_reason().unwrap_or("Unknown")
}

fn build_redirect_message(
    request_url: &Url,
    redirect_url: &Url,
    status: StatusCode,
    prompt: &str,
) -> String {
    format!(
        "REDIRECT DETECTED: The URL redirects to a different host.\n\nOriginal URL: {original}\nRedirect URL: {redirect}\nStatus: {code} {code_text}\n\nTo complete your request, use WebFetch again with these parameters:\n- url: \"{redirect}\"\n- prompt: \"{prompt}\"",
        original = request_url,
        redirect = redirect_url,
        code = status.as_u16(),
        code_text = status_text(status),
        prompt = prompt
    )
}

fn apply_prompt_to_content(prompt: &str, content: &str) -> Result<String> {
    let trimmed_prompt = prompt.trim();
    if trimmed_prompt.is_empty() {
        bail!("WebFetch prompt cannot be empty");
    }
    let mut result = format!(
        "Prompt:\n{prompt}\n\nFetched Content:\n",
        prompt = trimmed_prompt
    );
    if content.len() > MAX_RESULT_CHARS {
        result.push_str(&content[..MAX_RESULT_CHARS]);
        result.push_str("\n\n[Result truncated due to size]");
    } else {
        result.push_str(content);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_claude_field_names() {
        let schema = claude_web_fetch_input_schema();
        let properties = schema.get("properties").and_then(Value::as_object).unwrap();
        assert!(properties.contains_key("url"));
        assert!(properties.contains_key("prompt"));
    }

    #[test]
    fn normalize_url_upgrades_http() {
        let url = normalize_url("http://example.com/path").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("example.com"));
    }

    #[test]
    fn redirect_message_contains_follow_up_instruction() {
        let original = Url::parse("https://old.example").unwrap();
        let redirect = Url::parse("https://new.example/page").unwrap();
        let message =
            build_redirect_message(&original, &redirect, StatusCode::FOUND, "extract title");
        assert!(message.contains("REDIRECT DETECTED"));
        assert!(message.contains("use WebFetch again"));
        assert!(message.contains("https://new.example/page"));
    }

    #[test]
    fn apply_prompt_to_content_prefixes_prompt() {
        let output = apply_prompt_to_content("summarize", "hello world").unwrap();
        assert!(output.contains("Prompt:\nsummarize"));
        assert!(output.contains("Fetched Content:\nhello world"));
    }
}
