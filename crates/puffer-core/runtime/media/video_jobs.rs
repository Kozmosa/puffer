use super::{MediaArtifact, MediaGenerationService, MediaJob, MediaJobStatus, MediaKind};
use anyhow::{anyhow, Context, Result};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

const VIDEO_MIME_TYPE: &str = "video/mp4";

/// Shared bounded polling configuration for async video adapters.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VideoPollingConfig {
    pub(crate) max_attempts: usize,
    pub(crate) delay: Duration,
}

impl Default for VideoPollingConfig {
    fn default() -> Self {
        Self {
            max_attempts: 200,
            delay: Duration::from_millis(3_000),
        }
    }
}

/// Describes a completed provider video task ready for local artifact storage.
pub(crate) struct CompletedVideoTask<'a> {
    pub(crate) provider_id: &'a str,
    pub(crate) task_id: &'a str,
    pub(crate) remote_status: &'a str,
    pub(crate) video_url: Option<&'a str>,
    pub(crate) filename_prefix: &'static str,
    pub(crate) missing_url_message: &'static str,
}

/// Builds a task poll URL by appending the provider task id to the submit URL.
pub(crate) fn video_poll_url(submit_url: &str, job: &MediaJob) -> Result<String> {
    let id = job
        .provider_job_id
        .as_ref()
        .context("video job is missing a task id")?;
    let mut url = reqwest::Url::parse(submit_url).context("video submit URL must be absolute")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow!("video submit URL cannot be a base"))?;
        segments.push(id);
    }
    Ok(url.to_string())
}

/// Polls a video job until it reaches a terminal state or exhausts attempts.
pub(crate) fn poll_video_until_terminal(
    mut job: MediaJob,
    config: VideoPollingConfig,
    mut sleep: impl FnMut(Duration),
    mut now_ms: impl FnMut() -> u64,
    mut poll_once: impl FnMut(MediaJob, u64) -> Result<MediaJob>,
) -> Result<MediaJob> {
    for attempt in 0..config.max_attempts {
        job = poll_once(job, now_ms())?;
        if job.status.is_terminal() {
            return Ok(job);
        }
        if attempt + 1 < config.max_attempts {
            sleep(config.delay);
        }
    }
    anyhow::bail!(
        "video job `{}` did not reach a terminal status after {} polls",
        job.id,
        config.max_attempts
    )
}

/// Marks a media job failed, stores its diagnostic, and persists it.
pub(crate) fn persist_failed_video_job(
    service: &MediaGenerationService,
    mut job: MediaJob,
    error: impl Into<String>,
    now_ms: u64,
) -> Result<MediaJob> {
    job.error = Some(error.into());
    job.transition(MediaJobStatus::Failed, now_ms)?;
    service.save_job(&job)?;
    Ok(job)
}

/// Downloads and persists the MP4 artifact for a succeeded video job.
pub(crate) fn complete_video_job(
    service: &MediaGenerationService,
    mut job: MediaJob,
    task: CompletedVideoTask<'_>,
    now_ms: u64,
    download_bytes: impl FnOnce(&str) -> Result<Vec<u8>>,
) -> Result<MediaJob> {
    if !job.artifact_ids.is_empty() {
        job.transition(MediaJobStatus::Succeeded, now_ms)?;
        service.save_job(&job)?;
        return Ok(job);
    }
    let url = task.video_url.context(task.missing_url_message)?;
    let bytes = match download_bytes(url) {
        Ok(bytes) => bytes,
        Err(error) => {
            persist_failed_video_job(service, job, format!("{error:#}"), now_ms)?;
            return Err(error);
        }
    };
    let artifact_id = Uuid::new_v4().to_string();
    let path = service.write_artifact_bytes(
        &artifact_id,
        &format!("{}-{artifact_id}.mp4", task.filename_prefix),
        &bytes,
    )?;
    let artifact = MediaArtifact {
        id: artifact_id.clone(),
        job_id: job.id.clone(),
        kind: MediaKind::Video,
        path,
        mime_type: VIDEO_MIME_TYPE.to_string(),
        byte_count: bytes.len() as u64,
        metadata: json!({
            "provider": task.provider_id,
            "taskId": task.task_id,
            "remoteStatus": task.remote_status,
        }),
        created_at_ms: now_ms,
    };
    service.save_artifact(&artifact)?;
    job.attach_artifact(artifact_id, now_ms);
    job.error = None;
    job.transition(MediaJobStatus::Succeeded, now_ms)?;
    service.save_job(&job)?;
    Ok(job)
}
