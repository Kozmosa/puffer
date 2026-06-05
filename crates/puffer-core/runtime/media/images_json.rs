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
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const DEFAULT_IMAGE_REQUEST_TIMEOUT_MS: u64 = 300_000;
const IMAGES_JSON_ALLOWED_REQUEST_FIELDS: &[&str] = &[
    "model",
    "prompt",
    "n",
    "size",
    "quality",
    "output_format",
    "response_format",
    "aspect_ratio",
    "resolution",
];

/// Request shape for OpenAI image generation after media settings resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImagesJsonRequest {
    model: String,
    prompt: String,
    parameters: BTreeMap<String, String>,
}

impl ImagesJsonRequest {
    fn new(
        model: impl Into<String>,
        prompt: impl Into<String>,
        parameters: BTreeMap<String, String>,
    ) -> Self {
        Self {
            model: model.into(),
            prompt: prompt.into(),
            parameters,
        }
    }

    fn to_body(&self) -> Value {
        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model));
        body.insert("prompt".to_string(), json!(self.prompt));
        for (name, value) in &self.parameters {
            body.insert(name.clone(), json!(value));
        }
        Value::Object(body)
    }
}

/// Carries an exact OpenAI Images-compatible generation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImagesJsonGenerationRequest {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) adapter: String,
    pub(crate) prompt: String,
    pub(crate) parameters: BTreeMap<String, String>,
}

/// Carries persisted media records created by the OpenAI Images adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImagesJsonGenerationResult {
    pub(crate) job: MediaJob,
    pub(crate) artifact: MediaArtifact,
}

/// Executes descriptor-driven OpenAI Images-compatible generation.
#[derive(Debug, Clone)]
pub(crate) struct ImagesJsonAdapter {
    client: Client,
}

impl ImagesJsonAdapter {
    /// Creates an adapter with a default blocking HTTP client.
    pub(crate) fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(DEFAULT_IMAGE_REQUEST_TIMEOUT_MS))
            .build()
            .context("build image generation HTTP client")?;
        Ok(Self { client })
    }

    /// Executes an exact image generation request and persists job/artifact sidecars.
    pub(crate) fn execute(
        &self,
        registry: &ProviderRegistry,
        auth_store: &AuthStore,
        service: &MediaGenerationService,
        request: ImagesJsonGenerationRequest,
    ) -> Result<ImagesJsonGenerationResult> {
        let capability = validate_image_generate_selection(
            registry,
            auth_store,
            &ImageGenerationSelection {
                provider_id: &request.provider_id,
                model_id: &request.model_id,
                adapter: &request.adapter,
                parameters: &request.parameters,
            },
            now_ms(),
            &MediaDiscoveryCache::default(),
        )?;
        let selected_parameters =
            selected_parameters_with_defaults(&capability, &request.parameters)?;

        let discovery_cache = MediaDiscoveryCache::default();
        let (provider, execution) = resolve_image_execution_descriptor(
            registry,
            &request.provider_id,
            &request.model_id,
            &request.adapter,
            &discovery_cache,
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
            created_at_ms,
        );
        service.save_job(&job)?;
        job.transition(MediaJobStatus::Running, now_ms())?;
        service.save_job(&job)?;

        let output = match self.request_image(
            provider,
            auth_store,
            &request,
            selected_parameters,
            &execution,
        ) {
            Ok(output) => output,
            Err(error) => {
                job.error = Some(format!("{error:#}"));
                job.transition(MediaJobStatus::Failed, now_ms())?;
                service.save_job(&job)?;
                return Err(error);
            }
        };

        let output_format = output_format_for_parameters(&request.parameters);
        let filename = format!("image.{}", extension_for_output_format(&output_format));
        let artifact_path = service.write_artifact_bytes(&artifact_id, &filename, &output.bytes)?;
        let artifact = MediaArtifact {
            id: artifact_id.clone(),
            job_id: job_id.clone(),
            kind: MediaKind::Image,
            path: artifact_path.clone(),
            mime_type: mime_type_for_output_format(&output_format).to_string(),
            byte_count: output.bytes.len() as u64,
            metadata: artifact_metadata(&request, &artifact_path, &output, created_at_ms),
            created_at_ms,
        };
        service.save_artifact(&artifact)?;
        job.attach_artifact(artifact_id, now_ms());
        job.transition(MediaJobStatus::Succeeded, now_ms())?;
        service.save_job(&job)?;

        Ok(ImagesJsonGenerationResult { job, artifact })
    }

    fn request_image(
        &self,
        provider: &ProviderDescriptor,
        auth_store: &AuthStore,
        request: &ImagesJsonGenerationRequest,
        parameters: BTreeMap<String, String>,
        execution: &MediaExecutionDescriptor,
    ) -> Result<ImageOutput> {
        let url = provider_execution_url(provider, execution, "image generation")?;
        let secrets =
            provider_error_secrets(provider, auth_store, CredentialAliasMode::OpenAiCodexAlias);
        let body = ImagesJsonRequest::new(&request.model_id, &request.prompt, parameters).to_body();
        let mut http = self.client.post(url).json(&body);
        for (name, value) in &provider.headers {
            http = http.header(name.as_str(), value.as_str());
        }
        if let Some(token) =
            bearer_token(provider, auth_store, CredentialAliasMode::OpenAiCodexAlias)?
        {
            http = http.bearer_auth(token);
        }
        let response = http
            .send()
            .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))
            .context("send image generation request")?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))
            .context("read image generation response")?;
        if !status.is_success() {
            bail!(
                "image generation failed with status {}: {}",
                status.as_u16(),
                redact_secrets(&body, &secrets)
            );
        }
        let value: Value =
            serde_json::from_str(&body).context("parse image generation response")?;
        image_output_from_response(&self.client, &value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageOutput {
    bytes: Vec<u8>,
    revised_prompt: Option<String>,
    remote_source_url: Option<String>,
}

fn selected_parameters_with_defaults(
    capability: &crate::runtime::media::capabilities::MediaCapability,
    selected: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut request_parameters = BTreeMap::new();
    for parameter in &capability.parameters {
        let request_field = parameter
            .request_field
            .as_deref()
            .unwrap_or(parameter.name.as_str());
        if !IMAGES_JSON_ALLOWED_REQUEST_FIELDS.contains(&request_field) {
            bail!("image generation request field unsupported: {request_field}");
        }
        let value = selected
            .get(&parameter.name)
            .cloned()
            .unwrap_or_else(|| parameter.default.clone());
        request_parameters.insert(request_field.to_string(), value);
    }
    Ok(request_parameters)
}

fn image_output_from_response(client: &Client, value: &Value) -> Result<ImageOutput> {
    let Some(first) = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
    else {
        bail!("image generation response did not contain an image");
    };
    let revised_prompt = first
        .get("revised_prompt")
        .or_else(|| first.get("revisedPrompt"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if let Some(encoded) = first.get("b64_json").and_then(Value::as_str) {
        let bytes = BASE64_STANDARD
            .decode(encoded.trim())
            .context("decode image b64_json")?;
        return Ok(ImageOutput {
            bytes,
            revised_prompt,
            remote_source_url: None,
        });
    }
    if let Some(url) = first.get("url").and_then(Value::as_str) {
        let bytes = download_image_url(client, url, "image response")?;
        return Ok(ImageOutput {
            bytes,
            revised_prompt,
            remote_source_url: Some(url.to_string()),
        });
    }
    bail!("image generation response did not contain an image")
}

fn artifact_metadata(
    request: &ImagesJsonGenerationRequest,
    path: &std::path::Path,
    output: &ImageOutput,
    created_at_ms: u64,
) -> Value {
    let output_format = output_format_for_parameters(&request.parameters);
    let mut metadata = json!({
        "providerId": request.provider_id,
        "modelId": request.model_id,
        "adapter": request.adapter,
        "prompt": request.prompt,
        "parameters": request.parameters,
        "mimeType": mime_type_for_output_format(&output_format),
        "localPath": path,
        "byteCount": output.bytes.len() as u64,
        "createdAtMs": created_at_ms,
    });
    if let Some(revised_prompt) = &output.revised_prompt {
        metadata["revisedPrompt"] = json!(revised_prompt);
    }
    if let Some(remote_source_url) = &output.remote_source_url {
        metadata["remoteSourceUrl"] = json!(remote_source_url);
    }
    metadata
}

fn output_format_for_parameters(parameters: &BTreeMap<String, String>) -> String {
    parameters
        .get("output_format")
        .cloned()
        .unwrap_or_else(|| "png".to_string())
}

fn mime_type_for_output_format(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn extension_for_output_format(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "jpeg",
        "webp" => "webp",
        _ => "png",
    }
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
        MediaModelDescriptor, MediaOperation, MediaParameterSpec, ModelDescriptor,
        ProviderDescriptor, ProviderMediaDescriptor, ProviderRegistry,
    };
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::tempdir;

    fn registry_with_provider(base_url: String) -> ProviderRegistry {
        registry_with_provider_id("exact-provider", base_url)
    }

    fn registry_with_provider_id(provider_id: &str, base_url: String) -> ProviderRegistry {
        registry_with_provider_parameters(provider_id, base_url, image_parameters())
    }

    fn registry_with_provider_parameters(
        provider_id: &str,
        base_url: String,
        parameters: Vec<MediaParameterSpec>,
    ) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: provider_id.to_string(),
            display_name: "Exact Provider".to_string(),
            base_url,
            default_api: "openai-responses".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::from([("x-provider-header".to_string(), "present".to_string())]),
            query_params: IndexMap::from([("api-version".to_string(), "2026-06-05".to_string())]),
            chat_completions_path: None,
            discovery: None,
            media: Some(ProviderMediaDescriptor {
                image: Some(ImageMediaDescriptor {
                    discovery: None,
                    execution: Some(MediaExecutionDescriptor {
                        adapter: MediaExecutionKind::ImagesJson,
                        base_url: None,
                        path: "/custom/images".to_string(),
                    }),
                    models: vec![MediaModelDescriptor {
                        id: "exact-image-model".to_string(),
                        display_name: Some("Exact Image Model".to_string()),
                        execution: None,
                        operations: vec![MediaOperation::Generate],
                        parameters,
                    }],
                }),
            }),
            models: Vec::<ModelDescriptor>::new(),
        });
        registry
    }

    fn image_parameters() -> Vec<MediaParameterSpec> {
        vec![
            MediaParameterSpec {
                name: "size".to_string(),
                label: "Size".to_string(),
                values: vec!["1024x1024".to_string()],
                default: "1024x1024".to_string(),
                request_field: Some("size".to_string()),
            },
            MediaParameterSpec {
                name: "quality".to_string(),
                label: "Quality".to_string(),
                values: vec!["auto".to_string()],
                default: "auto".to_string(),
                request_field: Some("quality".to_string()),
            },
            MediaParameterSpec {
                name: "output_format".to_string(),
                label: "Output format".to_string(),
                values: vec!["png".to_string(), "webp".to_string()],
                default: "png".to_string(),
                request_field: Some("output_format".to_string()),
            },
        ]
    }

    fn auth_store() -> AuthStore {
        let mut auth = AuthStore::default();
        auth.set_api_key("exact-provider", "sk-secret");
        auth
    }

    fn codex_auth_store() -> AuthStore {
        let mut auth = AuthStore::default();
        auth.set_api_key("codex", "sk-codex-secret");
        auth
    }

    fn request() -> ImagesJsonGenerationRequest {
        ImagesJsonGenerationRequest {
            provider_id: "exact-provider".to_string(),
            model_id: "exact-image-model".to_string(),
            adapter: "images_json".to_string(),
            prompt: "draw a precise icon".to_string(),
            parameters: BTreeMap::from([
                ("size".to_string(), "1024x1024".to_string()),
                ("quality".to_string(), "auto".to_string()),
                ("output_format".to_string(), "png".to_string()),
            ]),
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = [0_u8; 8192];
        let size = stream.read(&mut buffer).expect("read request");
        String::from_utf8_lossy(&buffer[..size]).to_string()
    }

    #[test]
    fn request_body_uses_selected_model_and_descriptor_path() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let request_text = read_http_request(&mut stream);
            let body = json!({
                "data": [{"b64_json": "aW1hZ2UtYnl0ZXM="}]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
            request_text
        });
        let registry = registry_with_provider(format!("http://{address}"));
        let service_dir = tempdir().expect("tempdir");

        let result = ImagesJsonAdapter::new()
            .expect("adapter")
            .execute(
                &registry,
                &auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request(),
            )
            .expect("generation succeeds");

        let request_text = server.join().expect("server");
        assert!(request_text.starts_with("POST /custom/images?api-version=2026-06-05 HTTP/1.1"));
        assert!(request_text.contains("authorization: Bearer sk-secret"));
        assert!(request_text.contains("x-provider-header: present"));
        assert!(request_text.contains("\"model\":\"exact-image-model\""));
        assert!(request_text.contains("\"size\":\"1024x1024\""));
        assert_eq!(
            std::fs::read(&result.artifact.path).unwrap(),
            b"image-bytes"
        );
        assert_eq!(result.artifact.metadata["adapter"], "images_json");
    }

    #[test]
    fn request_body_uses_descriptor_request_field_mapping() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let request_text = read_http_request(&mut stream);
            let body = json!({
                "data": [{"b64_json": "aW1hZ2UtYnl0ZXM="}]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
            request_text
        });
        let mut parameters = image_parameters();
        parameters[0].name = "resolution_choice".to_string();
        parameters[0].request_field = Some("resolution".to_string());
        parameters[0].values = vec!["2k".to_string()];
        parameters[0].default = "2k".to_string();
        let registry = registry_with_provider_parameters(
            "exact-provider",
            format!("http://{address}"),
            parameters,
        );
        let service_dir = tempdir().expect("tempdir");
        let mut request = request();
        request.parameters.remove("size");
        request
            .parameters
            .insert("resolution_choice".to_string(), "2k".to_string());

        ImagesJsonAdapter::new()
            .expect("adapter")
            .execute(
                &registry,
                &auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request,
            )
            .expect("generation succeeds");

        let request_text = server.join().expect("server");
        assert!(request_text.contains("\"resolution\":\"2k\""));
        assert!(!request_text.contains("\"resolution_choice\""));
    }

    #[test]
    fn url_response_is_downloaded_before_success() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("generation request");
            let generation_request = read_http_request(&mut stream);
            let body = json!({
                "data": [{
                    "url": format!("http://{address}/generated.png"),
                    "revised_prompt": "draw a more precise icon"
                }]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("generation response");

            let (mut stream, _) = listener.accept().expect("download request");
            let download_request = read_http_request(&mut stream);
            let response = "HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ncontent-length: 12\r\nconnection: close\r\n\r\ndownloaded!!";
            stream
                .write_all(response.as_bytes())
                .expect("download response");
            (generation_request, download_request)
        });
        let registry = registry_with_provider(format!("http://{address}"));
        let service_dir = tempdir().expect("tempdir");

        let result = ImagesJsonAdapter::new()
            .expect("adapter")
            .execute(
                &registry,
                &auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request(),
            )
            .expect("generation succeeds");

        let (_, download_request) = server.join().expect("server");
        assert!(download_request.starts_with("GET /generated.png HTTP/1.1"));
        assert_eq!(
            std::fs::read(&result.artifact.path).unwrap(),
            b"downloaded!!"
        );
        assert_eq!(
            result.job.status,
            crate::runtime::media::MediaJobStatus::Succeeded
        );
        assert_eq!(
            result.artifact.metadata["revisedPrompt"],
            "draw a more precise icon"
        );
        assert_eq!(
            result.artifact.metadata["remoteSourceUrl"],
            format!("http://{address}/generated.png")
        );
    }

    #[test]
    fn missing_image_data_returns_stable_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let body = json!({"data": [{}]}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
        });
        let registry = registry_with_provider(format!("http://{address}"));
        let service_dir = tempdir().expect("tempdir");

        let error = ImagesJsonAdapter::new()
            .expect("adapter")
            .execute(
                &registry,
                &auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request(),
            )
            .expect_err("missing image data should fail");

        server.join().expect("server");
        assert_eq!(
            error.to_string(),
            "image generation response did not contain an image"
        );
    }

    #[test]
    fn external_http_image_url_is_rejected_before_download() {
        let value = json!({
            "data": [{"url": "http://example.com/generated.png"}]
        });

        let error = image_output_from_response(&Client::new(), &value)
            .expect_err("external http URL should fail before download");

        assert_eq!(
            error.to_string(),
            "unsupported image response URL scheme `http`"
        );
    }

    #[test]
    fn unsupported_parameter_fails_before_http_request() {
        let registry = registry_with_provider("http://127.0.0.1:9".to_string());
        let service_dir = tempdir().expect("tempdir");
        let mut request = request();
        request
            .parameters
            .insert("size".to_string(), "2048x2048".to_string());

        let error = ImagesJsonAdapter::new()
            .expect("adapter")
            .execute(
                &registry,
                &auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request,
            )
            .expect_err("unsupported parameter should fail");

        assert_eq!(
            error.to_string(),
            "image generation parameter unsupported: size=2048x2048"
        );
    }

    #[test]
    fn unsupported_request_field_fails_before_http_request() {
        let mut parameters = image_parameters();
        parameters[0].request_field = Some("watermark".to_string());
        let registry = registry_with_provider_parameters(
            "exact-provider",
            "http://127.0.0.1:9".to_string(),
            parameters,
        );
        let service_dir = tempdir().expect("tempdir");

        let error = ImagesJsonAdapter::new()
            .expect("adapter")
            .execute(
                &registry,
                &auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request(),
            )
            .expect_err("unsupported request field should fail before HTTP");

        assert_eq!(
            error.to_string(),
            "image generation request field unsupported: watermark"
        );
    }

    #[test]
    fn provider_secrets_are_redacted_from_error_responses() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let _request_text = read_http_request(&mut stream);
            let body = "bad sk-secret 2026-06-05";
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
        });
        let registry = registry_with_provider(format!("http://{address}"));
        let service_dir = tempdir().expect("tempdir");

        let error = ImagesJsonAdapter::new()
            .expect("adapter")
            .execute(
                &registry,
                &auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request(),
            )
            .expect_err("provider error should fail");

        server.join().expect("server");
        let message = error.to_string();
        assert!(message.contains("[redacted]"), "{message}");
        assert!(!message.contains("sk-secret"), "{message}");
        assert!(!message.contains("2026-06-05"), "{message}");
    }

    #[test]
    fn openai_provider_uses_codex_credentials_for_generation() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let request_text = read_http_request(&mut stream);
            let body = json!({
                "data": [{"b64_json": "aW1hZ2UtYnl0ZXM="}]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
            request_text
        });
        let registry = registry_with_provider_id("openai", format!("http://{address}"));
        let service_dir = tempdir().expect("tempdir");
        let request = ImagesJsonGenerationRequest {
            provider_id: "openai".to_string(),
            model_id: "exact-image-model".to_string(),
            ..request()
        };

        ImagesJsonAdapter::new()
            .expect("adapter")
            .execute(
                &registry,
                &codex_auth_store(),
                &MediaGenerationService::new(service_dir.path()),
                request,
            )
            .expect("generation succeeds");

        let request_text = server.join().expect("server");
        assert!(request_text.contains("authorization: Bearer sk-codex-secret"));
    }
}
