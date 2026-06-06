use super::artifacts::MediaArtifact;
use super::http_support::{
    bearer_token, download_image_url, provider_error_secrets, provider_execution_url,
    redact_secrets, CredentialAliasMode,
};
use super::jobs::{MediaJob, MediaJobStatus};
use super::resolver::{
    resolve_image_execution_descriptor, validate_image_generate_selection,
    ImageGenerationSelection, MediaDiscoveryCache,
};
use super::{MediaGenerationService, MediaKind};
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use puffer_provider_registry::{
    AuthStore, MediaExecutionDescriptor, ProviderDescriptor, ProviderRegistry,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const DEFAULT_CHAT_IMAGE_REQUEST_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatImageOutputRequest {
    model: String,
    prompt: String,
}

impl ChatImageOutputRequest {
    fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            prompt: prompt.into(),
        }
    }

    fn to_body(&self) -> Value {
        json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": self.prompt
            }],
            "modalities": ["image", "text"]
        })
    }
}

/// Carries an exact chat image-output generation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatImageOutputGenerationRequest {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) adapter: String,
    pub(crate) prompt: String,
    pub(crate) parameters: BTreeMap<String, String>,
    pub(crate) count: u8,
}

/// Carries persisted media records created by the chat image-output adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatImageOutputGenerationResult {
    pub(crate) job: MediaJob,
    pub(crate) artifacts: Vec<MediaArtifact>,
}

/// Executes descriptor-driven chat image-output generation.
#[derive(Debug, Clone)]
pub(crate) struct ChatImageOutputAdapter {
    client: Client,
}

impl ChatImageOutputAdapter {
    /// Creates an adapter with a default blocking HTTP client.
    pub(crate) fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(DEFAULT_CHAT_IMAGE_REQUEST_TIMEOUT_MS))
            .build()
            .context("build chat image-output HTTP client")?;
        Ok(Self { client })
    }

    /// Executes a request using static descriptors plus trusted discovery cache entries.
    pub(crate) fn execute_with_discovery_cache(
        &self,
        registry: &ProviderRegistry,
        auth_store: &AuthStore,
        service: &MediaGenerationService,
        request: ChatImageOutputGenerationRequest,
        discovery_cache: &MediaDiscoveryCache,
    ) -> Result<ChatImageOutputGenerationResult> {
        validate_image_generate_selection(
            registry,
            auth_store,
            &ImageGenerationSelection {
                provider_id: &request.provider_id,
                model_id: &request.model_id,
                adapter: &request.adapter,
                parameters: &request.parameters,
            },
            now_ms(),
            discovery_cache,
        )?;

        let (provider, execution) = resolve_image_execution_descriptor(
            registry,
            &request.provider_id,
            &request.model_id,
            &request.adapter,
            discovery_cache,
        )?;

        let job_id = Uuid::new_v4().to_string();
        let artifact_id = Uuid::new_v4().to_string();
        let created_at_ms = now_ms();
        let mut job = MediaJob::new(
            job_id.clone(),
            MediaKind::Image,
            request.provider_id.clone(),
            request.model_id.clone(),
            request.prompt.clone(),
            1,
            created_at_ms,
        );
        service.save_job(&job)?;
        job.transition(MediaJobStatus::Running, now_ms())?;
        service.save_job(&job)?;

        let output = match self.request_image(provider, auth_store, &execution, &request) {
            Ok(output) => output,
            Err(error) => {
                job.error = Some(format!("{error:#}"));
                job.transition(MediaJobStatus::Failed, now_ms())?;
                service.save_job(&job)?;
                return Err(error);
            }
        };

        let filename = "image.png";
        let artifact_path =
            service.write_image_artifact_bytes(&artifact_id, filename, &output.bytes)?;
        let artifact = MediaArtifact {
            id: artifact_id.clone(),
            job_id: job_id.clone(),
            kind: MediaKind::Image,
            path: artifact_path.clone(),
            mime_type: "image/png".to_string(),
            byte_count: output.bytes.len() as u64,
            metadata: artifact_metadata(&request, &artifact_path, &output, created_at_ms),
            created_at_ms,
        };
        service.save_artifact(&artifact)?;
        job.attach_artifact(artifact_id, now_ms());
        job.transition(MediaJobStatus::Succeeded, now_ms())?;
        service.save_job(&job)?;

        Ok(ChatImageOutputGenerationResult {
            job,
            artifacts: vec![artifact],
        })
    }

    fn request_image(
        &self,
        provider: &ProviderDescriptor,
        auth_store: &AuthStore,
        execution: &MediaExecutionDescriptor,
        request: &ChatImageOutputGenerationRequest,
    ) -> Result<ChatImageOutput> {
        let url = provider_execution_url(provider, execution, "chat image-output")?;
        let secrets = provider_error_secrets(provider, auth_store, CredentialAliasMode::Strict);
        let body = ChatImageOutputRequest::new(&request.model_id, &request.prompt).to_body();
        let mut http = self.client.post(url).json(&body);
        for (name, value) in &provider.headers {
            http = http.header(name.as_str(), value.as_str());
        }
        if let Some(token) = bearer_token(provider, auth_store, CredentialAliasMode::Strict)? {
            http = http.bearer_auth(token);
        }
        let response = http
            .send()
            .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))
            .context("send chat image-output request")?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))
            .context("read chat image-output response")?;
        if !status.is_success() {
            bail!(
                "chat image-output failed with status {}: {}",
                status.as_u16(),
                redact_secrets(&body, &secrets)
            );
        }
        let value: Value =
            serde_json::from_str(&body).context("parse chat image-output response")?;
        chat_output_from_response(&self.client, &value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatImageOutput {
    bytes: Vec<u8>,
    remote_source_url: Option<String>,
}

fn chat_output_from_response(client: &Client, value: &Value) -> Result<ChatImageOutput> {
    if let Some(choices) = value.get("choices").and_then(Value::as_array) {
        for choice in choices {
            if let Some(message) = choice.get("message") {
                if let Some(output) = chat_output_from_message(client, message) {
                    return output;
                }
            }
        }
    }
    if let Some(images) = value.get("images") {
        if let Some(output) = chat_output_from_image_array(client, images) {
            return output;
        }
    }
    bail!("chat image-output response did not contain an image")
}

fn chat_output_from_message(client: &Client, message: &Value) -> Option<Result<ChatImageOutput>> {
    if let Some(images) = message.get("images") {
        if let Some(output) = chat_output_from_image_array(client, images) {
            return Some(output);
        }
    }
    if let Some(parts) = message.get("content").and_then(Value::as_array) {
        for part in parts {
            if let Some(output) = chat_output_from_image_value(client, part) {
                return Some(output);
            }
        }
    }
    None
}

fn chat_output_from_image_array(client: &Client, value: &Value) -> Option<Result<ChatImageOutput>> {
    let images = value.as_array()?;
    for image in images {
        if let Some(output) = chat_output_from_image_value(client, image) {
            return Some(output);
        }
    }
    None
}

fn chat_output_from_image_value(client: &Client, value: &Value) -> Option<Result<ChatImageOutput>> {
    if let Some(encoded) = first_base64_field(value) {
        return Some(
            BASE64_STANDARD
                .decode(encoded.trim())
                .context("decode chat image-output base64")
                .map(|bytes| ChatImageOutput {
                    bytes,
                    remote_source_url: None,
                }),
        );
    }
    let url = first_url_field(value)?;
    Some(bytes_from_image_url(client, url))
}

fn first_base64_field(value: &Value) -> Option<&str> {
    value
        .get("b64_json")
        .or_else(|| value.get("base64"))
        .or_else(|| value.get("image_base64"))
        .or_else(|| value.pointer("/source/data"))
        .and_then(Value::as_str)
}

fn first_url_field(value: &Value) -> Option<&str> {
    value
        .get("url")
        .and_then(Value::as_str)
        .or_else(|| value.get("image_url").and_then(Value::as_str))
        .or_else(|| value.pointer("/image_url/url").and_then(Value::as_str))
        .or_else(|| value.pointer("/imageUrl/url").and_then(Value::as_str))
}

fn bytes_from_image_url(client: &Client, url: &str) -> Result<ChatImageOutput> {
    if let Some(bytes) = bytes_from_data_url(url)? {
        return Ok(ChatImageOutput {
            bytes,
            remote_source_url: None,
        });
    }
    let bytes = download_image_url(client, url, "chat image-output")?;
    Ok(ChatImageOutput {
        bytes,
        remote_source_url: Some(url.to_string()),
    })
}

fn bytes_from_data_url(url: &str) -> Result<Option<Vec<u8>>> {
    if !url.starts_with("data:") {
        return Ok(None);
    }
    let Some((metadata, encoded)) = url.split_once(',') else {
        bail!("invalid chat image-output data URL");
    };
    if !metadata.contains(";base64") {
        bail!("chat image-output data URL must be base64 encoded");
    }
    Ok(Some(
        BASE64_STANDARD
            .decode(encoded.trim())
            .context("decode chat image-output data URL")?,
    ))
}

fn artifact_metadata(
    request: &ChatImageOutputGenerationRequest,
    path: &std::path::Path,
    output: &ChatImageOutput,
    created_at_ms: u64,
) -> Value {
    let mut metadata = json!({
        "providerId": request.provider_id,
        "modelId": request.model_id,
        "adapter": request.adapter,
        "prompt": request.prompt,
        "parameters": request.parameters,
        "mimeType": "image/png",
        "localPath": path,
        "byteCount": output.bytes.len() as u64,
        "createdAtMs": created_at_ms,
    });
    if let Some(remote_source_url) = &output.remote_source_url {
        metadata["remoteSourceUrl"] = json!(remote_source_url);
    }
    metadata
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::media::MediaGenerationService;
    use indexmap::IndexMap;
    use puffer_provider_registry::{
        AuthMode, AuthStore, ImageMediaDescriptor, MediaExecutionDescriptor, MediaExecutionKind,
        MediaModelDescriptor, MediaOperation, ModelDescriptor, ProviderDescriptor,
        ProviderMediaDescriptor, ProviderRegistry,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::tempdir;

    fn registry_with_provider(base_url: String) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: "openrouter".to_string(),
            display_name: "OpenRouter".to_string(),
            base_url,
            default_api: "openai-completions".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::new(),
            query_params: IndexMap::new(),
            chat_completions_path: Some("/chat/completions".to_string()),
            discovery: None,
            media: Some(ProviderMediaDescriptor {
                image: Some(ImageMediaDescriptor {
                    discovery: None,
                    execution: Some(MediaExecutionDescriptor {
                        adapter: MediaExecutionKind::ChatImageOutput,
                        base_url: None,
                        path: "/chat/completions".to_string(),
                    }),
                    models: vec![MediaModelDescriptor {
                        id: "openrouter/image-chat".to_string(),
                        display_name: Some("Image Chat".to_string()),
                        execution: None,
                        operations: vec![MediaOperation::Generate],
                        parameters: Vec::new(),
                    }],
                }),
            }),
            models: Vec::<ModelDescriptor>::new(),
        });
        registry
    }

    fn auth_store() -> AuthStore {
        let mut auth = AuthStore::default();
        auth.set_api_key("openrouter", "sk-openrouter");
        auth
    }

    fn request() -> ChatImageOutputGenerationRequest {
        ChatImageOutputGenerationRequest {
            provider_id: "openrouter".to_string(),
            model_id: "openrouter/image-chat".to_string(),
            adapter: "chat_image_output".to_string(),
            prompt: "draw a precise icon".to_string(),
            parameters: BTreeMap::new(),
            count: 1,
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = [0_u8; 8192];
        let size = stream.read(&mut buffer).expect("read request");
        String::from_utf8_lossy(&buffer[..size]).to_string()
    }

    #[test]
    fn chat_image_output_posts_modalities_and_downloads_returned_image_url() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut chat_stream, _) = listener.accept().expect("chat request");
            let request_text = read_http_request(&mut chat_stream);
            let image_url = format!("http://{address}/generated.png");
            let body = json!({
                "choices": [{
                    "message": {
                        "images": [{
                            "image_url": {"url": image_url}
                        }]
                    }
                }]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            chat_stream
                .write_all(response.as_bytes())
                .expect("chat response");

            let (mut image_stream, _) = listener.accept().expect("image request");
            let _image_request = read_http_request(&mut image_stream);
            let image = b"image-bytes";
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                image.len()
            );
            image_stream
                .write_all(response.as_bytes())
                .expect("image response headers");
            image_stream.write_all(image).expect("image response body");
            request_text
        });
        let registry = registry_with_provider(format!("http://{address}"));
        let service_dir = tempdir().expect("tempdir");

        let result = ChatImageOutputAdapter::new()
            .expect("adapter")
            .execute_with_discovery_cache(
                &registry,
                &auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request(),
                &MediaDiscoveryCache::default(),
            )
            .expect("generation succeeds");

        let request_text = server.join().expect("server");
        assert!(request_text.starts_with("POST /chat/completions HTTP/1.1"));
        assert!(request_text.contains("authorization: Bearer sk-openrouter"));
        assert!(request_text.contains("\"model\":\"openrouter/image-chat\""));
        assert!(request_text.contains("\"modalities\":[\"image\",\"text\"]"));
        assert!(request_text.contains("\"content\":\"draw a precise icon\""));
        assert_eq!(
            std::fs::read(&result.artifacts[0].path).unwrap(),
            b"image-bytes"
        );
        assert_eq!(result.artifacts[0].metadata["adapter"], "chat_image_output");
    }
}
