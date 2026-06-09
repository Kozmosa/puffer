use super::{
    exact_media_generation_result, load_media_job_artifacts, now_ms, parse_media_operation,
    ExactMediaDiscoveryCache, ExactMediaGenerationRequest, ExactMediaGenerationResult,
};
use crate::runtime::media::byteplus_video::{
    byteplus_video_request_from_parameters, BytePlusVideoAdapter, BytePlusVideoPollingConfig,
};
use crate::runtime::media::capabilities::MediaCapability;
use crate::runtime::media::http_support::{
    bearer_token, provider_error_secrets, provider_execution_url, redact_secrets,
    CredentialAliasMode,
};
use crate::runtime::media::relaydance_video::{
    relaydance_video_request_from_parameters, RelaydanceVideoAdapter, RelaydanceVideoPollingConfig,
    RELAYDANCE_VIDEO_ADAPTER,
};
use crate::runtime::media::replicate_video::{
    ReplicatePollingConfig, ReplicateVideoAdapter, ReplicateVideoRequest,
};
use crate::runtime::media::resolver::{
    resolve_video_execution_descriptor, validate_media_generate_selection, MediaGenerationSelection,
};
use crate::runtime::media::{MediaGenerationService, MediaJob, MediaKind};
use anyhow::{anyhow, bail, Context, Result};
use puffer_provider_registry::{AuthStore, ProviderRegistry};
use std::collections::BTreeMap;
use std::path::Path;

/// Executes an exact text-to-video request through the selected video adapter.
pub(super) fn generate_exact_video_from_media_request(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
    request: ExactMediaGenerationRequest,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Result<ExactMediaGenerationResult> {
    reclaim_relaydance_video_jobs(registry, auth_store, workspace_root);
    let operation = parse_media_operation(&request.operation)?;
    let selection = MediaGenerationSelection {
        kind: MediaKind::Video,
        provider_id: &request.provider_id,
        model_id: &request.model_id,
        operation,
        adapter: &request.adapter,
        parameters: &request.parameters,
    };
    let capability = validate_media_generate_selection(
        registry,
        auth_store,
        &selection,
        now_ms(),
        &discovery_cache.inner,
    )?;
    validate_video_count(request.count)?;
    let parameters = selected_parameters_with_defaults(&capability, &request.parameters);
    match request.adapter.as_str() {
        "replicate_video" => {
            generate_replicate_video(registry, auth_store, workspace_root, &request, parameters)
        }
        "relaydance_video" => generate_relaydance_video(
            registry,
            auth_store,
            workspace_root,
            &request,
            &capability,
            parameters,
        ),
        "byteplus_video" => generate_byteplus_video(
            registry,
            auth_store,
            workspace_root,
            &request,
            &capability,
            parameters,
        ),
        adapter => bail!("video media adapter unavailable for {adapter}"),
    }
}

fn generate_replicate_video(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
    request: &ExactMediaGenerationRequest,
    parameters: BTreeMap<String, String>,
) -> Result<ExactMediaGenerationResult> {
    let provider = registry.provider(&request.provider_id).with_context(|| {
        format!(
            "selected video model unavailable: {}/{} via {}",
            request.provider_id, request.model_id, request.adapter
        )
    })?;
    let api_key = bearer_token(provider, auth_store, CredentialAliasMode::Strict)?
        .context("Replicate API token is required")?;
    let service = MediaGenerationService::new(workspace_root);
    let adapter = ReplicateVideoAdapter::new(api_key)?;
    let video_request = replicate_video_request_from_parameters(
        request.model_id.clone(),
        request.prompt.clone(),
        parameters,
    )?;
    let job = adapter.submit(&service, video_request, now_ms())?;
    let job = adapter.poll_until_terminal(&service, job, ReplicatePollingConfig::default())?;
    finish_exact_video_job(&service, job)
}

/// Resolves provider + execution + token and constructs a Relaydance video
/// adapter, returning it alongside the secrets to redact from error strings.
/// Shared by generation and orphan reclaim.
fn build_relaydance_adapter(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    provider_id: &str,
    model_id: &str,
    adapter: &str,
) -> Result<(RelaydanceVideoAdapter, Vec<String>)> {
    let (provider, execution) =
        resolve_video_execution_descriptor(registry, provider_id, model_id, adapter)?;
    let api_key = bearer_token(provider, auth_store, CredentialAliasMode::Strict)?
        .context("Relaydance API key is required")?;
    let secrets = provider_error_secrets(provider, auth_store, CredentialAliasMode::Strict);
    let submit_url = provider_execution_url(provider, &execution, "video task")?;
    let built =
        RelaydanceVideoAdapter::new(api_key, submit_url.to_string(), provider_id.to_string())?;
    Ok((built, secrets))
}

/// Best-effort: poll each non-terminal relaydance video job once so orphaned
/// tasks (e.g. the app died mid-render) get reclaimed on the next generation.
/// Never propagates errors — a missing token or offline provider must not block
/// the user's new request.
fn reclaim_relaydance_video_jobs(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
) {
    let service = MediaGenerationService::new(workspace_root);
    let Ok(jobs) = service.list_video_jobs() else {
        return;
    };
    for job in jobs {
        if job.status.is_terminal()
            || job.adapter.as_deref() != Some(RELAYDANCE_VIDEO_ADAPTER)
            || job.provider_job_id.is_none()
        {
            continue;
        }
        let Ok((adapter, _secrets)) = build_relaydance_adapter(
            registry,
            auth_store,
            &job.provider_id,
            &job.model_id,
            RELAYDANCE_VIDEO_ADAPTER,
        ) else {
            continue;
        };
        let _ = adapter.poll(&service, job, now_ms());
    }
}

fn generate_relaydance_video(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
    request: &ExactMediaGenerationRequest,
    capability: &MediaCapability,
    parameters: BTreeMap<String, String>,
) -> Result<ExactMediaGenerationResult> {
    let service = MediaGenerationService::new(workspace_root);
    let (adapter, secrets) = build_relaydance_adapter(
        registry,
        auth_store,
        &request.provider_id,
        &request.model_id,
        &request.adapter,
    )?;
    let video_request = relaydance_video_request_from_parameters(
        request.model_id.clone(),
        request.prompt.clone(),
        &capability.parameters,
        &parameters,
    )?;
    let job = adapter
        .submit(&service, video_request, parameters, now_ms())
        .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))?;
    let job = adapter
        .poll_until_terminal(
            &service,
            job,
            RelaydanceVideoPollingConfig::default(),
            std::thread::sleep,
            now_ms,
        )
        .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))?;
    finish_exact_video_job(&service, job)
}

fn generate_byteplus_video(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
    request: &ExactMediaGenerationRequest,
    capability: &MediaCapability,
    parameters: BTreeMap<String, String>,
) -> Result<ExactMediaGenerationResult> {
    let (provider, execution) = resolve_video_execution_descriptor(
        registry,
        &request.provider_id,
        &request.model_id,
        &request.adapter,
    )?;
    let api_key = bearer_token(provider, auth_store, CredentialAliasMode::Strict)?
        .context("BytePlus API key is required")?;
    let secrets = provider_error_secrets(provider, auth_store, CredentialAliasMode::Strict);
    let service = MediaGenerationService::new(workspace_root);
    let submit_url = provider_execution_url(provider, &execution, "video task")?;
    let adapter =
        BytePlusVideoAdapter::new(api_key, submit_url.to_string(), request.provider_id.clone())?;
    let video_request = byteplus_video_request_from_parameters(
        request.model_id.clone(),
        request.prompt.clone(),
        &capability.parameters,
        &parameters,
    )?;
    let job = adapter
        .submit(&service, video_request, parameters, now_ms())
        .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))?;
    let job = adapter
        .poll_until_terminal(
            &service,
            job,
            BytePlusVideoPollingConfig::default(),
            std::thread::sleep,
            now_ms,
        )
        .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))?;
    finish_exact_video_job(&service, job)
}

fn finish_exact_video_job(
    service: &MediaGenerationService,
    job: MediaJob,
) -> Result<ExactMediaGenerationResult> {
    let artifacts = load_media_job_artifacts(service, &job)?;
    Ok(exact_media_generation_result(job, artifacts))
}

fn replicate_video_request_from_parameters(
    model_id: String,
    prompt: String,
    parameters: BTreeMap<String, String>,
) -> Result<ReplicateVideoRequest> {
    let aspect_ratio = parameters
        .get("aspect_ratio")
        .cloned()
        .context("video generation parameter unsupported: aspect_ratio=")?;
    let duration_seconds = parameters
        .get("duration")
        .context("video generation parameter unsupported: duration=")?
        .parse::<u32>()
        .context("video generation parameter unsupported: duration")?;
    Ok(ReplicateVideoRequest {
        model: model_id,
        prompt,
        aspect_ratio,
        duration_seconds,
    })
}

fn selected_parameters_with_defaults(
    capability: &MediaCapability,
    selected: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    capability
        .parameters
        .iter()
        .map(|parameter| {
            let value = selected
                .get(&parameter.name)
                .cloned()
                .unwrap_or_else(|| parameter.default.clone());
            (parameter.name.clone(), value)
        })
        .collect()
}

fn validate_video_count(count: u8) -> Result<u8> {
    if count == 1 {
        Ok(count)
    } else {
        bail!("video generation count must be 1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::media::relaydance_video::tests_support::scripted;
    use crate::runtime::media::MediaJobStatus;
    use serde_json::json;

    #[test]
    fn poll_once_reclaims_completed_relaydance_job() {
        let dir = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(dir.path());

        // Pre-seed a video job frozen in queued, carrying a provider_job_id.
        let mut job = MediaJob::new(
            "frozen-1",
            MediaKind::Video,
            "relaydance",
            "seedance-nsfw",
            "p",
            1,
            1,
        );
        job.adapter = Some("relaydance_video".to_string());
        job.provider_job_id = Some("task_done".to_string());
        service.save_job(&job).unwrap();

        // Inject a gateway response that already reports SUCCESS.
        let adapter = RelaydanceVideoAdapter::with_transport(
            "token",
            "https://relaydance.com/v1/video/generations",
            "relaydance",
            scripted(
                json!({}),
                vec![json!({ "data": { "id": "task_done", "status": "SUCCESS",
                    "result_url": "https://cdn.example.com/v.mp4" } })],
            ),
        );
        let loaded = service.load_job("frozen-1").unwrap();
        let reclaimed = adapter.poll(&service, loaded, 2).expect("reclaim poll");
        assert_eq!(reclaimed.status, MediaJobStatus::Succeeded);
        assert_eq!(reclaimed.artifact_ids.len(), 1);
    }
}
