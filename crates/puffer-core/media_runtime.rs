use crate::runtime::media::chat_image_output::{
    ChatImageOutputAdapter, ChatImageOutputGenerationRequest,
};
use crate::runtime::media::discovery::TrustedImageDiscoveryClient;
use crate::runtime::media::images_json::{ImagesJsonAdapter, ImagesJsonGenerationRequest};
use crate::runtime::media::minimax_image::{MinimaxImageAdapter, MinimaxImageGenerationRequest};
use crate::runtime::media::resolver::{resolve_media_capabilities, MediaDiscoveryCache};
use crate::runtime::media::{
    MediaArtifact, MediaGenerationService, MediaJob, MediaJobStatus, MediaKind,
};
use anyhow::{bail, Result};
use puffer_provider_registry::{AuthStore, MediaOperation, ProviderRegistry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default TTL for trusted media discovery results.
pub const MEDIA_DISCOVERY_TTL_MS: u64 = 5 * 60 * 1_000;

/// Describes one exact media capability suitable for client display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCapabilityView {
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub kind: String,
    pub operation: String,
    pub adapter: String,
    pub parameters: Vec<MediaCapabilityParameterView>,
    pub defaults: BTreeMap<String, String>,
    pub status: String,
    pub source: String,
    pub reason: Option<String>,
    pub checked_at_ms: u64,
}

/// Describes one select parameter suitable for client rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCapabilityParameterView {
    pub name: String,
    pub label: String,
    pub values: Vec<String>,
    pub default: String,
    pub request_field: Option<String>,
}

/// Carries an exact image generation request from UI or tool configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactImageGenerationRequest {
    pub provider_id: String,
    pub model_id: String,
    pub adapter: String,
    pub prompt: String,
    pub parameters: BTreeMap<String, String>,
}

/// Carries the persisted job and artifact produced by exact image generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactImageGenerationResult {
    pub job_id: String,
    pub artifact_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub status: String,
    pub path: PathBuf,
}

/// Describes the preview-read result for a generated media artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum GeneratedMediaPreviewResult {
    Available {
        #[serde(rename = "mimeType")]
        mime_type: String,
        bytes: Vec<u8>,
    },
    Missing,
    Unsupported,
}

/// Carries generated image attachment metadata without image bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedMediaAttachmentMetadata {
    pub artifact_id: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub state: String,
}

/// Carries trusted media discovery results used by capability resolution.
#[derive(Debug, Clone)]
pub struct ExactMediaDiscoveryCache {
    inner: MediaDiscoveryCache,
    cached_at_ms: u64,
}

/// Loads generated image metadata by artifact id.
pub fn generated_media_attachment_metadata(
    workspace_root: impl AsRef<Path>,
    artifact_id: &str,
) -> Option<GeneratedMediaAttachmentMetadata> {
    let service = MediaGenerationService::new(workspace_root.as_ref());
    let artifact = service.load_artifact(artifact_id).ok()?;
    if artifact.kind != MediaKind::Image {
        return None;
    }
    let image_root = generated_media_image_root(workspace_root.as_ref());
    let canonical_path = match canonical_generated_media_image_path(&image_root, &artifact.path) {
        Ok(path) => Some(path),
        Err(GeneratedMediaPathError::Missing) => None,
        Err(GeneratedMediaPathError::Unsupported) => return None,
    };
    let state = if canonical_path.is_some() {
        "available"
    } else {
        "missing"
    };
    let mime_type = canonical_path
        .as_ref()
        .and_then(|path| canonical_generated_image_mime_type(path, Some(&artifact.mime_type)))
        .or_else(|| {
            canonical_sidecar_image_mime_type(Some(&artifact.mime_type)).map(str::to_string)
        })
        .or_else(|| generated_image_mime_type(&artifact.path).map(str::to_string))
        .unwrap_or_else(|| artifact.mime_type.clone());
    Some(GeneratedMediaAttachmentMetadata {
        artifact_id: artifact.id,
        mime_type,
        byte_count: artifact.byte_count,
        state: state.to_string(),
    })
}

/// Reads generated image preview bytes by artifact id.
pub fn read_generated_media_preview_by_artifact(
    workspace_root: impl AsRef<Path>,
    artifact_id: &str,
) -> GeneratedMediaPreviewResult {
    let service = MediaGenerationService::new(workspace_root.as_ref());
    let artifact = match service.load_artifact(artifact_id) {
        Ok(artifact) => artifact,
        Err(_) => return GeneratedMediaPreviewResult::Missing,
    };
    if artifact.kind != MediaKind::Image {
        return GeneratedMediaPreviewResult::Unsupported;
    }
    let image_root = generated_media_image_root(workspace_root.as_ref());
    read_generated_media_preview_from_root_with_mime(
        &image_root,
        &artifact.path,
        Some(&artifact.mime_type),
    )
}

fn read_generated_media_preview_from_root_with_mime(
    image_root: &Path,
    path: &Path,
    sidecar_mime_type: Option<&str>,
) -> GeneratedMediaPreviewResult {
    let canonical_path = match canonical_generated_media_image_path(image_root, path) {
        Ok(path) => path,
        Err(GeneratedMediaPathError::Missing) => return GeneratedMediaPreviewResult::Missing,
        Err(GeneratedMediaPathError::Unsupported) => {
            return GeneratedMediaPreviewResult::Unsupported
        }
    };
    let bytes = match std::fs::read(&canonical_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return GeneratedMediaPreviewResult::Missing;
        }
        Err(_) => return GeneratedMediaPreviewResult::Missing,
    };
    let Some(mime_type) = sniff_generated_image_mime_type(&bytes)
        .or_else(|| canonical_sidecar_image_mime_type(sidecar_mime_type))
        .or_else(|| generated_image_mime_type(&canonical_path))
    else {
        return GeneratedMediaPreviewResult::Unsupported;
    };
    GeneratedMediaPreviewResult::Available {
        mime_type: mime_type.to_string(),
        bytes,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratedMediaPathError {
    Missing,
    Unsupported,
}

fn canonical_generated_media_image_path(
    image_root: &Path,
    path: &Path,
) -> std::result::Result<PathBuf, GeneratedMediaPathError> {
    let canonical_path = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return if missing_generated_media_path_is_under_root(image_root, path) {
                Err(GeneratedMediaPathError::Missing)
            } else {
                Err(GeneratedMediaPathError::Unsupported)
            };
        }
        Err(_) => return Err(GeneratedMediaPathError::Missing),
    };
    let canonical_root =
        std::fs::canonicalize(image_root).map_err(|_| GeneratedMediaPathError::Unsupported)?;
    if !canonical_path.starts_with(canonical_root) {
        return Err(GeneratedMediaPathError::Unsupported);
    }
    let metadata = match std::fs::metadata(&canonical_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(GeneratedMediaPathError::Missing);
        }
        Err(_) => return Err(GeneratedMediaPathError::Missing),
    };
    if !metadata.is_file() {
        return Err(GeneratedMediaPathError::Unsupported);
    }
    Ok(canonical_path)
}

fn generated_media_image_root(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".puffer").join("media").join("images")
}

fn missing_generated_media_path_is_under_root(image_root: &Path, path: &Path) -> bool {
    if let (Ok(canonical_root), Some(parent)) = (std::fs::canonicalize(image_root), path.parent()) {
        if let Ok(canonical_parent) = std::fs::canonicalize(parent) {
            return canonical_parent.starts_with(canonical_root);
        }
    }
    lexical_path_starts_with(path, image_root)
}

fn lexical_path_starts_with(path: &Path, root: &Path) -> bool {
    lexical_normalize_path(path).starts_with(lexical_normalize_path(root))
}

fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn canonical_sidecar_image_mime_type(value: Option<&str>) -> Option<&'static str> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

fn canonical_generated_image_mime_type(
    path: &Path,
    sidecar_mime_type: Option<&str>,
) -> Option<String> {
    let bytes = generated_image_magic_bytes(path)?;
    sniff_generated_image_mime_type(&bytes)
        .or_else(|| canonical_sidecar_image_mime_type(sidecar_mime_type))
        .or_else(|| generated_image_mime_type(path))
        .map(str::to_string)
}

fn generated_image_magic_bytes(path: &Path) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = [0_u8; 12];
    let count = file.read(&mut buffer).ok()?;
    Some(buffer[..count].to_vec())
}

fn sniff_generated_image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

fn generated_image_mime_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

impl ExactMediaDiscoveryCache {
    /// Creates an empty media discovery cache.
    pub fn empty() -> Self {
        Self {
            inner: MediaDiscoveryCache::default(),
            cached_at_ms: 0,
        }
    }

    /// Returns the time at which this cache was refreshed.
    pub fn cached_at_ms(&self) -> u64 {
        self.cached_at_ms
    }

    /// Returns whether this cache is fresh at the given timestamp.
    pub fn is_fresh_at(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.cached_at_ms) <= MEDIA_DISCOVERY_TTL_MS
    }

    #[cfg(test)]
    pub(crate) fn from_inner_for_test(inner: MediaDiscoveryCache, cached_at_ms: u64) -> Self {
        Self {
            inner,
            cached_at_ms,
        }
    }
}

/// Lists exact media capabilities using static descriptors and trusted discovery cache entries.
pub fn list_exact_media_capabilities_with_cache(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    kind_filter: Option<&str>,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Vec<MediaCapabilityView> {
    let checked_at_ms = now_ms();
    let mut capabilities = Vec::new();
    if kind_filter_matches(kind_filter, "image") {
        capabilities.extend(resolve_media_capabilities(
            registry,
            auth_store,
            MediaKind::Image,
            MediaOperation::Generate,
            checked_at_ms,
            &discovery_cache.inner,
        ));
    }
    if kind_filter_matches(kind_filter, "video") {
        capabilities.extend(resolve_media_capabilities(
            registry,
            auth_store,
            MediaKind::Video,
            MediaOperation::Generate,
            checked_at_ms,
            &discovery_cache.inner,
        ));
    }
    capabilities
        .into_iter()
        .map(MediaCapabilityView::from)
        .collect()
}

/// Refreshes trusted media discovery cache entries for connected providers.
pub fn discover_exact_media_capabilities(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
) -> ExactMediaDiscoveryCache {
    let inner = TrustedImageDiscoveryClient::new().discover(registry, auth_store);
    ExactMediaDiscoveryCache {
        inner,
        cached_at_ms: now_ms(),
    }
}

/// Generates one exact image using static descriptors plus trusted discovery cache entries.
pub fn generate_exact_image_with_cache(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
    mut request: ExactImageGenerationRequest,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Result<ExactImageGenerationResult> {
    request.parameters = resolved_exact_image_parameters_with_cache(
        registry,
        auth_store,
        &request,
        discovery_cache,
    )?;
    let service = MediaGenerationService::new(workspace_root);
    match request.adapter.as_str() {
        "images_json" => {
            let result = ImagesJsonAdapter::new()?.execute(
                registry,
                auth_store,
                &service,
                ImagesJsonGenerationRequest {
                    provider_id: request.provider_id,
                    model_id: request.model_id,
                    adapter: request.adapter,
                    prompt: request.prompt,
                    parameters: request.parameters,
                },
            )?;
            Ok(exact_generation_result(result.job, result.artifact))
        }
        "minimax_image" => {
            let result = MinimaxImageAdapter::new()?.execute(
                registry,
                auth_store,
                &service,
                MinimaxImageGenerationRequest {
                    provider_id: request.provider_id,
                    model_id: request.model_id,
                    adapter: request.adapter,
                    prompt: request.prompt,
                    parameters: request.parameters,
                },
            )?;
            Ok(exact_generation_result(result.job, result.artifact))
        }
        "chat_image_output" => {
            let result = ChatImageOutputAdapter::new()?.execute_with_discovery_cache(
                registry,
                auth_store,
                &service,
                ChatImageOutputGenerationRequest {
                    provider_id: request.provider_id,
                    model_id: request.model_id,
                    adapter: request.adapter,
                    prompt: request.prompt,
                    parameters: request.parameters,
                },
                &discovery_cache.inner,
            )?;
            Ok(exact_generation_result(result.job, result.artifact))
        }
        adapter => bail!("image media adapter unavailable for {adapter}"),
    }
}

/// Resolves exact image parameters against the current capability defaults.
pub fn resolved_exact_image_parameters_with_cache(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    selection: &ExactImageGenerationRequest,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Result<BTreeMap<String, String>> {
    let capability = exact_image_capability(registry, auth_store, selection, discovery_cache)?;
    Ok(capability
        .parameters
        .iter()
        .map(|parameter| {
            let value = selection
                .parameters
                .get(&parameter.name)
                .cloned()
                .unwrap_or_else(|| parameter.default.clone());
            (parameter.name.clone(), value)
        })
        .collect())
}

fn exact_image_capability(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    selection: &ExactImageGenerationRequest,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Result<crate::runtime::media::capabilities::MediaCapability> {
    resolve_media_capabilities(
        registry,
        auth_store,
        MediaKind::Image,
        MediaOperation::Generate,
        now_ms(),
        &discovery_cache.inner,
    )
    .into_iter()
    .find(|capability| {
        capability.provider_id == selection.provider_id
            && capability.model_id == selection.model_id
            && capability.adapter == selection.adapter
    })
    .ok_or_else(|| {
        anyhow::anyhow!(
            "selected image model unavailable: {}/{} via {}",
            selection.provider_id,
            selection.model_id,
            selection.adapter
        )
    })
}

fn exact_generation_result(job: MediaJob, artifact: MediaArtifact) -> ExactImageGenerationResult {
    ExactImageGenerationResult {
        job_id: job.id,
        artifact_id: artifact.id,
        provider_id: job.provider_id,
        model_id: job.model_id,
        status: media_job_status_name(job.status).to_string(),
        path: artifact.path,
    }
}

impl From<crate::runtime::media::capabilities::MediaCapability> for MediaCapabilityView {
    fn from(capability: crate::runtime::media::capabilities::MediaCapability) -> Self {
        Self {
            provider_id: capability.provider_id,
            provider_display_name: capability.provider_display_name,
            model_id: capability.model_id,
            model_display_name: capability.model_display_name,
            kind: media_kind_name(capability.kind).to_string(),
            operation: capability.operation,
            adapter: capability.adapter,
            parameters: capability
                .parameters
                .into_iter()
                .map(MediaCapabilityParameterView::from)
                .collect(),
            defaults: capability.defaults,
            status: capability.status,
            source: capability.source,
            reason: capability.reason,
            checked_at_ms: capability.checked_at_ms,
        }
    }
}

impl From<crate::runtime::media::capabilities::MediaCapabilityParameter>
    for MediaCapabilityParameterView
{
    fn from(parameter: crate::runtime::media::capabilities::MediaCapabilityParameter) -> Self {
        Self {
            name: parameter.name,
            label: parameter.label,
            values: parameter.values,
            default: parameter.default,
            request_field: parameter.request_field,
        }
    }
}

fn kind_filter_matches(kind_filter: Option<&str>, kind: &str) -> bool {
    kind_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none_or(|value| value == kind)
}

fn media_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
    }
}

fn media_job_status_name(status: MediaJobStatus) -> &'static str {
    match status {
        MediaJobStatus::Queued => "queued",
        MediaJobStatus::Running => "running",
        MediaJobStatus::Succeeded => "succeeded",
        MediaJobStatus::Failed => "failed",
        MediaJobStatus::Canceled => "canceled",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "media_runtime_generated_preview_tests.rs"]
mod generated_preview_tests;

#[cfg(test)]
mod tests {
    use super::*;
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

    fn minimax_registry(base_url: String) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: "minimax".to_string(),
            display_name: "MiniMax".to_string(),
            base_url: "https://api.minimax.io/anthropic".to_string(),
            default_api: "anthropic-messages".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::new(),
            query_params: IndexMap::new(),
            chat_completions_path: None,
            discovery: None,
            media: Some(ProviderMediaDescriptor {
                image: Some(ImageMediaDescriptor {
                    discovery: None,
                    execution: Some(MediaExecutionDescriptor {
                        adapter: MediaExecutionKind::MinimaxImage,
                        base_url: Some(base_url),
                        path: "/v1/image_generation".to_string(),
                    }),
                    models: vec![MediaModelDescriptor {
                        id: "image-01".to_string(),
                        display_name: Some("Image 01".to_string()),
                        execution: None,
                        operations: vec![MediaOperation::Generate],
                        parameters: vec![
                            MediaParameterSpec {
                                name: "aspect_ratio".to_string(),
                                label: "Aspect ratio".to_string(),
                                values: vec!["1:1".to_string(), "16:9".to_string()],
                                default: "1:1".to_string(),
                                request_field: Some("aspect_ratio".to_string()),
                            },
                            MediaParameterSpec {
                                name: "response_format".to_string(),
                                label: "Response format".to_string(),
                                values: vec!["url".to_string(), "base64".to_string()],
                                default: "base64".to_string(),
                                request_field: Some("response_format".to_string()),
                            },
                        ],
                    }],
                }),
            }),
            models: Vec::<ModelDescriptor>::new(),
        });
        registry
    }

    fn chat_router_registry(base_url: String) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: "openrouter".to_string(),
            display_name: "OpenRouter".to_string(),
            base_url,
            default_api: "openai-completions".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::new(),
            query_params: IndexMap::new(),
            chat_completions_path: None,
            discovery: None,
            media: Some(ProviderMediaDescriptor {
                image: Some(ImageMediaDescriptor {
                    discovery: None,
                    execution: Some(MediaExecutionDescriptor {
                        adapter: MediaExecutionKind::ChatImageOutput,
                        base_url: None,
                        path: "/chat/completions".to_string(),
                    }),
                    models: Vec::new(),
                }),
            }),
            models: Vec::<ModelDescriptor>::new(),
        });
        registry
    }

    fn byteplus_seedream_registry(base_url: String) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderDescriptor {
            id: "byteplus".to_string(),
            display_name: "BytePlus".to_string(),
            base_url,
            default_api: "openai-completions".to_string(),
            auth_modes: vec![AuthMode::ApiKey],
            headers: IndexMap::new(),
            query_params: IndexMap::new(),
            chat_completions_path: None,
            discovery: None,
            media: Some(ProviderMediaDescriptor {
                image: Some(ImageMediaDescriptor {
                    discovery: None,
                    execution: Some(MediaExecutionDescriptor {
                        adapter: MediaExecutionKind::ImagesJson,
                        base_url: None,
                        path: "/images/generations".to_string(),
                    }),
                    models: vec![MediaModelDescriptor {
                        id: "seedream-4-5-251128".to_string(),
                        display_name: Some("Seedream 4.5".to_string()),
                        execution: None,
                        operations: vec![MediaOperation::Generate],
                        parameters: vec![
                            MediaParameterSpec {
                                name: "size".to_string(),
                                label: "Size".to_string(),
                                values: vec!["2K".to_string()],
                                default: "2K".to_string(),
                                request_field: Some("size".to_string()),
                            },
                            MediaParameterSpec {
                                name: "response_format".to_string(),
                                label: "Response format".to_string(),
                                values: vec!["b64_json".to_string(), "url".to_string()],
                                default: "b64_json".to_string(),
                                request_field: Some("response_format".to_string()),
                            },
                        ],
                    }],
                }),
            }),
            models: Vec::<ModelDescriptor>::new(),
        });
        registry
    }

    fn discovered_chat_image_cache() -> ExactMediaDiscoveryCache {
        ExactMediaDiscoveryCache::from_inner_for_test(
            crate::runtime::media::resolver::MediaDiscoveryCache {
                image_models: vec![crate::runtime::media::resolver::CachedImageMediaModel {
                    provider_id: "openrouter".to_string(),
                    model: MediaModelDescriptor {
                        id: "openrouter/image-chat".to_string(),
                        display_name: Some("Image Chat".to_string()),
                        execution: None,
                        operations: vec![MediaOperation::Generate],
                        parameters: Vec::new(),
                    },
                    source: "provider_discovery".to_string(),
                }],
            },
            1_000,
        )
    }

    fn auth_store() -> AuthStore {
        let mut auth = AuthStore::default();
        auth.set_api_key("minimax", "sk-minimax");
        auth
    }

    fn auth_store_for(provider_id: &str) -> AuthStore {
        let mut auth = AuthStore::default();
        auth.set_api_key(provider_id, "sk-test");
        auth
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = [0_u8; 8192];
        let size = stream.read(&mut buffer).expect("read request");
        String::from_utf8_lossy(&buffer[..size]).to_string()
    }

    #[test]
    fn generate_exact_image_dispatches_to_minimax_adapter() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let request_text = read_http_request(&mut stream);
            let body = json!({
                "data": {"image_base64": ["aW1hZ2UtYnl0ZXM="]},
                "base_resp": {"status_code": 0, "status_msg": "success"}
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
        let registry = minimax_registry(format!("http://{address}"));
        let workspace = tempdir().expect("tempdir");

        let result = generate_exact_image_with_cache(
            &registry,
            &auth_store(),
            workspace.path(),
            ExactImageGenerationRequest {
                provider_id: "minimax".to_string(),
                model_id: "image-01".to_string(),
                adapter: "minimax_image".to_string(),
                prompt: "draw a precise icon".to_string(),
                parameters: BTreeMap::from([
                    ("aspect_ratio".to_string(), "16:9".to_string()),
                    ("response_format".to_string(), "base64".to_string()),
                ]),
            },
            &ExactMediaDiscoveryCache::empty(),
        )
        .expect("generation succeeds");

        let request_text = server.join().expect("server");
        assert!(request_text.starts_with("POST /v1/image_generation HTTP/1.1"));
        assert!(request_text.contains("\"aspect_ratio\":\"16:9\""));
        assert_eq!(result.provider_id, "minimax");
        assert_eq!(result.model_id, "image-01");
        assert_eq!(std::fs::read(result.path).unwrap(), b"image-bytes");
    }

    #[test]
    fn generate_exact_image_with_cache_executes_discovered_chat_image_model() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let request_text = read_http_request(&mut stream);
            let body = json!({
                "choices": [{
                    "message": {
                        "images": [{"b64_json": "aW1hZ2UtYnl0ZXM="}]
                    }
                }]
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
        let registry = chat_router_registry(format!("http://{address}"));
        let workspace = tempdir().expect("tempdir");
        let cache = discovered_chat_image_cache();

        let result = generate_exact_image_with_cache(
            &registry,
            &auth_store_for("openrouter"),
            workspace.path(),
            ExactImageGenerationRequest {
                provider_id: "openrouter".to_string(),
                model_id: "openrouter/image-chat".to_string(),
                adapter: "chat_image_output".to_string(),
                prompt: "draw a precise icon".to_string(),
                parameters: BTreeMap::new(),
            },
            &cache,
        )
        .expect("generation succeeds");

        let request_text = server.join().expect("server");
        assert!(request_text.starts_with("POST /chat/completions HTTP/1.1"));
        assert!(request_text.contains("\"model\":\"openrouter/image-chat\""));
        assert_eq!(result.provider_id, "openrouter");
        assert_eq!(result.model_id, "openrouter/image-chat");
        assert_eq!(std::fs::read(result.path).unwrap(), b"image-bytes");
    }

    #[test]
    fn generate_exact_image_prunes_stale_undeclared_parameters_before_http() {
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
        let registry = byteplus_seedream_registry(format!("http://{address}"));
        let workspace = tempdir().expect("tempdir");

        generate_exact_image_with_cache(
            &registry,
            &auth_store_for("byteplus"),
            workspace.path(),
            ExactImageGenerationRequest {
                provider_id: "byteplus".to_string(),
                model_id: "seedream-4-5-251128".to_string(),
                adapter: "images_json".to_string(),
                prompt: "draw a precise icon".to_string(),
                parameters: BTreeMap::from([
                    ("size".to_string(), "2K".to_string()),
                    ("output_format".to_string(), "png".to_string()),
                ]),
            },
            &ExactMediaDiscoveryCache::empty(),
        )
        .expect("generation succeeds");

        let request_text = server.join().expect("server");
        assert!(request_text.contains("\"size\":\"2K\""));
        assert!(request_text.contains("\"response_format\":\"b64_json\""));
        assert!(!request_text.contains("output_format"));
    }

    #[test]
    fn generate_exact_image_with_cache_rejects_discovered_model_missing_from_cache_before_http() {
        let registry = chat_router_registry("http://127.0.0.1:9".to_string());
        let workspace = tempdir().expect("tempdir");

        let error = generate_exact_image_with_cache(
            &registry,
            &auth_store_for("openrouter"),
            workspace.path(),
            ExactImageGenerationRequest {
                provider_id: "openrouter".to_string(),
                model_id: "openrouter/image-chat".to_string(),
                adapter: "chat_image_output".to_string(),
                prompt: "draw a precise icon".to_string(),
                parameters: BTreeMap::new(),
            },
            &ExactMediaDiscoveryCache::empty(),
        )
        .expect_err("missing discovery cache should fail");

        assert_eq!(
            error.to_string(),
            "selected image model unavailable: openrouter/openrouter/image-chat via chat_image_output"
        );
    }

    #[test]
    fn exact_media_discovery_cache_uses_ttl_boundary() {
        let cache = ExactMediaDiscoveryCache::from_inner_for_test(
            crate::runtime::media::resolver::MediaDiscoveryCache::default(),
            1_000,
        );

        assert!(cache.is_fresh_at(1_000 + MEDIA_DISCOVERY_TTL_MS - 1));
        assert!(!cache.is_fresh_at(1_000 + MEDIA_DISCOVERY_TTL_MS + 1));
    }
}
