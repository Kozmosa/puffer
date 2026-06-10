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
use anyhow::{bail, Context, Result};
use puffer_provider_registry::{
    AuthStore, MediaOperation, MediaParameterWireType, ProviderRegistry,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "media_runtime_artifacts.rs"]
mod media_runtime_artifacts;
use media_runtime_artifacts::media_artifact_remote_source_url;
pub use media_runtime_artifacts::{
    generated_media_attachment_metadata, generated_media_attachment_metadata_with_fallback,
    generated_media_timeline_attachments, generated_video_access_metadata_by_artifact,
    read_generated_media_preview_by_artifact, GeneratedMediaAttachmentMetadata,
    GeneratedMediaPreviewResult, GeneratedMediaTimelineAttachment,
    GeneratedMediaTimelineAttachmentKind, GeneratedVideoAccessMetadata,
    GeneratedVideoAccessMetadataResult,
};
#[path = "media_runtime_internal_tools.rs"]
mod media_runtime_internal_tools;
pub use media_runtime_internal_tools::{
    generated_media_internal_bash_output, generated_media_internal_command_kind,
    GeneratedMediaInternalCommandKind,
};
#[path = "media_runtime_video.rs"]
mod media_runtime_video;
use media_runtime_video::generate_exact_video_from_media_request;

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
    pub wire_type: MediaParameterWireType,
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

/// Carries trusted media discovery results used by capability resolution.
#[derive(Debug, Clone)]
pub struct ExactMediaDiscoveryCache {
    inner: MediaDiscoveryCache,
    cached_at_ms: u64,
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
            wire_type: parameter.wire_type,
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
