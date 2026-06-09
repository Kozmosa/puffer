use crate::runtime::media::chat_image_output::{
    ChatImageOutputAdapter, ChatImageOutputGenerationRequest,
};
use crate::runtime::media::discovery::TrustedImageDiscoveryClient;
use crate::runtime::media::images_json::{ImagesJsonAdapter, ImagesJsonGenerationRequest};
use crate::runtime::media::minimax_image::{MinimaxImageAdapter, MinimaxImageGenerationRequest};
use crate::runtime::media::resolver::{resolve_media_capabilities, MediaDiscoveryCache};
use crate::runtime::media::{
    artifacts::MediaArtifactPreviewState, MediaArtifact, MediaGenerationService, MediaJob,
    MediaJobStatus, MediaKind,
};
use anyhow::{bail, Context, Result};
use puffer_provider_registry::{AuthStore, MediaOperation, ProviderRegistry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "media_runtime_video.rs"]
mod media_runtime_video;
use media_runtime_video::generate_exact_video_from_media_request;

/// Default TTL for trusted media discovery results.
pub const MEDIA_DISCOVERY_TTL_MS: u64 = 5 * 60 * 1_000;
const REMOTE_SOURCE_URL_METADATA_KEY: &str = "remoteSourceUrl";

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
    pub count: u8,
}

/// Carries an exact media generation request from UI or tool configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactMediaGenerationRequest {
    pub kind: String,
    pub provider_id: String,
    pub model_id: String,
    pub operation: String,
    pub adapter: String,
    pub prompt: String,
    pub parameters: BTreeMap<String, String>,
    pub count: u8,
}

/// Carries one persisted generated image artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactGeneratedArtifact {
    pub artifact_id: String,
    pub index: usize,
    pub path: PathBuf,
    pub mime_type: String,
    pub byte_count: u64,
    pub remote_source_url: Option<String>,
}

/// Carries the persisted job and artifacts produced by exact image generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactImageGenerationResult {
    pub job_id: String,
    pub requested_count: u8,
    pub artifacts: Vec<ExactGeneratedArtifact>,
    pub provider_id: String,
    pub model_id: String,
    pub status: String,
}

/// Carries the persisted job and artifacts produced by exact media generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactMediaGenerationResult {
    pub job_id: String,
    pub requested_count: u8,
    pub artifacts: Vec<ExactGeneratedArtifact>,
    pub kind: String,
    pub provider_id: String,
    pub model_id: String,
    pub status: String,
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

/// Describes a trusted generated video file that can be served through a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedVideoAccessMetadata {
    pub path: PathBuf,
    pub mime_type: String,
    pub byte_count: u64,
}

/// Describes generated video access metadata lookup state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedVideoAccessMetadataResult {
    Available(GeneratedVideoAccessMetadata),
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
    pub local_path: Option<String>,
    pub remote_source_url: Option<String>,
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
    generated_media_attachment_metadata_from_artifact(workspace_root.as_ref(), artifact)
}

/// Loads generated image metadata or falls back to missing metadata from a tool result.
pub fn generated_media_attachment_metadata_with_fallback(
    workspace_root: impl AsRef<Path>,
    artifact_id: &str,
    fallback_mime_type: &str,
    fallback_byte_count: u64,
) -> Option<GeneratedMediaAttachmentMetadata> {
    let artifact_id = artifact_id.trim();
    if !valid_generated_media_artifact_id(artifact_id) {
        return None;
    }
    let service = MediaGenerationService::new(workspace_root.as_ref());
    match service.load_artifact(artifact_id) {
        Ok(artifact) => {
            generated_media_attachment_metadata_from_artifact(workspace_root.as_ref(), artifact)
        }
        Err(_) => {
            if !generated_media_artifact_sidecar_missing(workspace_root.as_ref(), artifact_id) {
                return None;
            }
            let mime_type = canonical_sidecar_image_mime_type(Some(fallback_mime_type))?;
            Some(GeneratedMediaAttachmentMetadata {
                artifact_id: artifact_id.to_string(),
                mime_type: mime_type.to_string(),
                byte_count: fallback_byte_count,
                state: "missing".to_string(),
                local_path: None,
                remote_source_url: None,
            })
        }
    }
}

fn generated_media_artifact_sidecar_missing(workspace_root: &Path, artifact_id: &str) -> bool {
    let sidecar_path = workspace_root
        .join(".puffer")
        .join("media")
        .join("artifact-sidecars")
        .join(format!("{artifact_id}.json"));
    matches!(
        std::fs::symlink_metadata(sidecar_path),
        Err(error) if error.kind() == ErrorKind::NotFound
    )
}

fn generated_media_attachment_metadata_from_artifact(
    workspace_root: &Path,
    artifact: MediaArtifact,
) -> Option<GeneratedMediaAttachmentMetadata> {
    if artifact.kind != MediaKind::Image {
        return None;
    }
    let image_root = generated_media_image_root(workspace_root);
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
    let local_path = canonical_path
        .as_ref()
        .map(|path| path.display().to_string());
    let remote_source_url = media_artifact_remote_source_url(&artifact);
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
        local_path,
        remote_source_url,
    })
}

fn valid_generated_media_artifact_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn media_artifact_remote_source_url(artifact: &MediaArtifact) -> Option<String> {
    artifact
        .metadata
        .get(REMOTE_SOURCE_URL_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Reads generated artifact preview bytes by artifact id.
pub fn read_generated_media_preview_by_artifact(
    workspace_root: impl AsRef<Path>,
    artifact_id: &str,
) -> GeneratedMediaPreviewResult {
    let workspace_root = workspace_root.as_ref();
    let service = MediaGenerationService::new(workspace_root);
    let artifact = match service.load_artifact(artifact_id) {
        Ok(artifact) => artifact,
        Err(_) => return GeneratedMediaPreviewResult::Missing,
    };
    match artifact.kind {
        MediaKind::Image => {
            let image_root = generated_media_image_root(workspace_root);
            read_generated_media_preview_from_root_with_mime(
                &image_root,
                &artifact.path,
                Some(&artifact.mime_type),
            )
        }
        MediaKind::Video => read_generated_video_poster_preview(workspace_root, &artifact),
    }
}

fn read_generated_video_poster_preview(
    workspace_root: &Path,
    artifact: &MediaArtifact,
) -> GeneratedMediaPreviewResult {
    let Some(preview) = artifact.preview.as_ref() else {
        return GeneratedMediaPreviewResult::Missing;
    };
    let poster = preview.poster();
    if poster.state != MediaArtifactPreviewState::Available {
        return GeneratedMediaPreviewResult::Missing;
    }
    let Some(path) = poster.path.as_ref() else {
        return GeneratedMediaPreviewResult::Missing;
    };
    let Some(expected_mime_type) = poster
        .mime_type
        .as_deref()
        .and_then(|mime_type| canonical_sidecar_image_mime_type(Some(mime_type)))
    else {
        return GeneratedMediaPreviewResult::Unsupported;
    };
    if expected_mime_type != "image/jpeg" {
        return GeneratedMediaPreviewResult::Unsupported;
    }
    let artifact_root = generated_media_artifact_root(workspace_root, &artifact.id);
    let canonical_path = match canonical_generated_media_artifact_path(&artifact_root, path) {
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
    let Some(mime_type) = sniff_generated_image_mime_type(&bytes) else {
        return GeneratedMediaPreviewResult::Unsupported;
    };
    if mime_type != expected_mime_type {
        return GeneratedMediaPreviewResult::Unsupported;
    }
    GeneratedMediaPreviewResult::Available {
        mime_type: mime_type.to_string(),
        bytes,
    }
}

/// Resolves trusted generated video metadata by artifact id.
pub fn generated_video_access_metadata_by_artifact(
    workspace_root: impl AsRef<Path>,
    artifact_id: &str,
) -> GeneratedVideoAccessMetadataResult {
    let workspace_root = workspace_root.as_ref();
    let service = MediaGenerationService::new(workspace_root);
    let artifact = match service.load_artifact(artifact_id) {
        Ok(artifact) => artifact,
        Err(_) => return GeneratedVideoAccessMetadataResult::Missing,
    };
    if artifact.kind != MediaKind::Video {
        return GeneratedVideoAccessMetadataResult::Unsupported;
    }
    let Some(mime_type) = canonical_generated_video_mime_type(&artifact.mime_type) else {
        return GeneratedVideoAccessMetadataResult::Unsupported;
    };
    let canonical_path = match canonical_generated_video_artifact_path(workspace_root, &artifact) {
        Ok(path) => path,
        Err(GeneratedMediaPathError::Missing) => {
            return GeneratedVideoAccessMetadataResult::Missing;
        }
        Err(GeneratedMediaPathError::Unsupported) => {
            return GeneratedVideoAccessMetadataResult::Unsupported;
        }
    };
    let byte_count = std::fs::metadata(&canonical_path)
        .map(|metadata| metadata.len())
        .unwrap_or(artifact.byte_count);
    GeneratedVideoAccessMetadataResult::Available(GeneratedVideoAccessMetadata {
        path: canonical_path,
        mime_type: mime_type.to_string(),
        byte_count,
    })
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

fn generated_media_video_root(workspace_root: &Path, artifact_id: &str) -> PathBuf {
    workspace_root
        .join(".puffer")
        .join("media")
        .join("videos")
        .join(artifact_id)
}

fn generated_media_artifact_root(workspace_root: &Path, artifact_id: &str) -> PathBuf {
    workspace_root
        .join(".puffer")
        .join("media")
        .join("artifacts")
        .join(artifact_id)
}

fn canonical_generated_video_artifact_path(
    workspace_root: &Path,
    artifact: &MediaArtifact,
) -> std::result::Result<PathBuf, GeneratedMediaPathError> {
    let roots = [
        generated_media_video_root(workspace_root, &artifact.id),
        generated_media_artifact_root(workspace_root, &artifact.id),
    ];
    let mut missing = false;
    for root in roots {
        match canonical_generated_media_artifact_path(&root, &artifact.path) {
            Ok(path) => return Ok(path),
            Err(GeneratedMediaPathError::Missing) => missing = true,
            Err(GeneratedMediaPathError::Unsupported) => {}
        }
    }
    if missing {
        Err(GeneratedMediaPathError::Missing)
    } else {
        Err(GeneratedMediaPathError::Unsupported)
    }
}

fn canonical_generated_media_artifact_path(
    artifact_root: &Path,
    path: &Path,
) -> std::result::Result<PathBuf, GeneratedMediaPathError> {
    canonical_generated_media_image_path(artifact_root, path)
}

fn canonical_generated_video_mime_type(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "video/mp4" => Some("video/mp4"),
        "video/webm" => Some("video/webm"),
        _ => None,
    }
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
    let count = validate_image_count(request.count)?;
    request.count = count;
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
                    count: request.count,
                },
            )?;
            Ok(exact_generation_result(result.job, result.artifacts))
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
                    count: request.count,
                },
            )?;
            Ok(exact_generation_result(result.job, result.artifacts))
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
                    count: request.count,
                },
                &discovery_cache.inner,
            )?;
            Ok(exact_generation_result(result.job, result.artifacts))
        }
        adapter => bail!("image media adapter unavailable for {adapter}"),
    }
}

fn validate_image_count(count: u8) -> Result<u8> {
    if (1..=4).contains(&count) {
        Ok(count)
    } else {
        bail!("image generation count must be between 1 and 4")
    }
}

/// Generates exact media using static descriptors plus trusted discovery cache entries.
pub fn generate_exact_media_with_cache(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
    request: ExactMediaGenerationRequest,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Result<ExactMediaGenerationResult> {
    match request.kind.trim() {
        "image" => generate_exact_image_from_media_request(
            registry,
            auth_store,
            workspace_root,
            request,
            discovery_cache,
        ),
        "video" => generate_exact_video_from_media_request(
            registry,
            auth_store,
            workspace_root,
            request,
            discovery_cache,
        ),
        kind => bail!("unsupported media kind `{kind}`"),
    }
}

fn generate_exact_image_from_media_request(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
    request: ExactMediaGenerationRequest,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Result<ExactMediaGenerationResult> {
    parse_media_operation(&request.operation)?;
    let result = generate_exact_image_with_cache(
        registry,
        auth_store,
        workspace_root,
        ExactImageGenerationRequest {
            provider_id: request.provider_id,
            model_id: request.model_id,
            adapter: request.adapter,
            prompt: request.prompt,
            parameters: request.parameters,
            count: request.count,
        },
        discovery_cache,
    )?;
    Ok(ExactMediaGenerationResult {
        job_id: result.job_id,
        requested_count: result.requested_count,
        artifacts: result.artifacts,
        kind: "image".to_string(),
        provider_id: result.provider_id,
        model_id: result.model_id,
        status: result.status,
    })
}

fn parse_media_operation(operation: &str) -> Result<MediaOperation> {
    match operation.trim() {
        "generate" => Ok(MediaOperation::Generate),
        operation => bail!("unsupported media operation `{operation}`"),
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

fn exact_generation_result(
    job: MediaJob,
    artifacts: Vec<MediaArtifact>,
) -> ExactImageGenerationResult {
    let artifacts = exact_generated_artifacts(artifacts);
    ExactImageGenerationResult {
        job_id: job.id,
        requested_count: job.requested_count,
        artifacts,
        provider_id: job.provider_id,
        model_id: job.model_id,
        status: media_job_status_name(job.status).to_string(),
    }
}

fn exact_media_generation_result(
    job: MediaJob,
    artifacts: Vec<MediaArtifact>,
) -> ExactMediaGenerationResult {
    let artifacts = exact_generated_artifacts(artifacts);
    ExactMediaGenerationResult {
        job_id: job.id,
        requested_count: job.requested_count,
        artifacts,
        kind: media_kind_name(job.kind).to_string(),
        provider_id: job.provider_id,
        model_id: job.model_id,
        status: media_job_status_name(job.status).to_string(),
    }
}

fn exact_generated_artifacts(artifacts: Vec<MediaArtifact>) -> Vec<ExactGeneratedArtifact> {
    artifacts
        .into_iter()
        .enumerate()
        .map(|(index, artifact)| {
            let remote_source_url = media_artifact_remote_source_url(&artifact);
            ExactGeneratedArtifact {
                artifact_id: artifact.id,
                index,
                path: artifact.path,
                mime_type: artifact.mime_type,
                byte_count: artifact.byte_count,
                remote_source_url,
            }
        })
        .collect()
}

fn load_media_job_artifacts(
    service: &MediaGenerationService,
    job: &MediaJob,
) -> Result<Vec<MediaArtifact>> {
    job.artifact_ids
        .iter()
        .map(|artifact_id| {
            service
                .load_artifact(artifact_id)
                .with_context(|| format!("load generated media artifact {artifact_id}"))
        })
        .collect()
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
#[path = "media_runtime_tests.rs"]
mod tests;
