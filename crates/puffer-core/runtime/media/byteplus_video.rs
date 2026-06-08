use super::capabilities::MediaCapabilityParameter;
use super::{MediaArtifact, MediaGenerationService, MediaJob, MediaJobStatus, MediaKind};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

const VIDEO_MIME_TYPE: &str = "video/mp4";
const BYTEPLUS_VIDEO_ADAPTER: &str = "byteplus_video";

pub(crate) struct BytePlusVideoRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) params: Vec<(String, String)>,
}

impl BytePlusVideoRequest {
    /// Builds a BytePlus ModelArk text-to-video task request.
    pub(crate) fn request_body(&self) -> Value {
        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model.trim()));
        body.insert(
            "content".to_string(),
            json!([
                {
                    "type": "text",
                    "text": self.prompt.trim()
                }
            ]),
        );
        for (field, value) in &self.params {
            body.insert(field.trim().to_string(), json!(value.trim()));
        }
        Value::Object(body)
    }

    fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty() {
            bail!("video model is required");
        }
        if self.prompt.trim().is_empty() {
            bail!("video prompt is required");
        }
        Ok(())
    }
}

/// Maps a validated selection into a BytePlus text-to-video request.
pub(crate) fn byteplus_video_request_from_parameters(
    model_id: String,
    prompt: String,
    capability_parameters: &[MediaCapabilityParameter],
    selected: &BTreeMap<String, String>,
) -> Result<BytePlusVideoRequest> {
    let mut params = Vec::new();
    for parameter in capability_parameters {
        let Some(field) = parameter.request_field.clone() else {
            continue;
        };
        let value = selected
            .get(&parameter.name)
            .cloned()
            .unwrap_or_else(|| parameter.default.clone());
        params.push((field, value));
    }
    let request = BytePlusVideoRequest {
        model: model_id,
        prompt,
        params,
    };
    request.validate()?;
    Ok(request)
}

pub(crate) struct BytePlusVideoTask {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) video_url: Option<String>,
    pub(crate) error: Option<String>,
}

impl BytePlusVideoTask {
    /// Parses the BytePlus task creation response.
    pub(crate) fn from_submit_value(value: Value) -> Result<Self> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .context("submit video task response missing task id")?
            .to_string();
        Ok(Self {
            id,
            status: "queued".to_string(),
            video_url: None,
            error: byteplus_error_message(&value),
        })
    }

    /// Parses the BytePlus task retrieval response.
    pub(crate) fn from_poll_value(value: Value) -> Result<Self> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .context("poll video task response missing task id")?
            .to_string();
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .context("poll video task response missing status")?
            .to_string();
        Ok(Self {
            id,
            status,
            video_url: byteplus_video_url(&value),
            error: byteplus_error_message(&value),
        })
    }

    fn media_status(&self) -> Result<MediaJobStatus> {
        match self.status.trim().to_ascii_lowercase().as_str() {
            "queued" | "pending" => Ok(MediaJobStatus::Queued),
            "in_progress" | "running" | "processing" => Ok(MediaJobStatus::Running),
            "completed" | "succeeded" | "success" => Ok(MediaJobStatus::Succeeded),
            "failed" | "error" | "expired" => Ok(MediaJobStatus::Failed),
            "cancelled" | "canceled" => Ok(MediaJobStatus::Canceled),
            other => bail!("unknown video task status `{other}`"),
        }
    }
}

fn byteplus_video_url(value: &Value) -> Option<String> {
    value
        .get("content")
        .and_then(|content| content.get("video_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn byteplus_error_message(value: &Value) -> Option<String> {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) trait BytePlusVideoTransport {
    /// Submits a BytePlus video task and returns its JSON response.
    fn submit_task(&self, url: &str, api_token: &str, body: &Value) -> Result<Value>;

    /// Polls a BytePlus video task URL and returns its JSON response.
    fn poll_task(&self, url: &str, api_token: &str) -> Result<Value>;

    /// Downloads rendered video bytes from a validated remote URL.
    fn download_bytes(&self, url: &str) -> Result<Vec<u8>>;
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ReqwestBytePlusVideoTransport {
    client: Client,
}

impl BytePlusVideoTransport for ReqwestBytePlusVideoTransport {
    fn submit_task(&self, url: &str, api_token: &str, body: &Value) -> Result<Value> {
        let response = self
            .client
            .post(url)
            .bearer_auth(api_token)
            .json(body)
            .send()
            .with_context(|| format!("submit video task {url}"))?;
        byteplus_video_json_response(response, "submit video task")
    }

    fn poll_task(&self, url: &str, api_token: &str) -> Result<Value> {
        let response = self
            .client
            .get(url)
            .bearer_auth(api_token)
            .send()
            .with_context(|| format!("poll video task {url}"))?;
        byteplus_video_json_response(response, "poll video task")
    }

    fn download_bytes(&self, url: &str) -> Result<Vec<u8>> {
        super::http_support::download_image_url(&self.client, url, "video output")
    }
}

fn byteplus_video_json_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<Value> {
    let status = response.status();
    let text = response
        .text()
        .with_context(|| format!("read {label} response body"))?;
    if !status.is_success() {
        bail!("{label} failed with status {}: {text}", status.as_u16());
    }
    serde_json::from_str(&text).with_context(|| format!("parse {label} response JSON"))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BytePlusVideoPollingConfig {
    pub(crate) max_attempts: usize,
    pub(crate) delay: Duration,
}

impl Default for BytePlusVideoPollingConfig {
    fn default() -> Self {
        Self {
            max_attempts: 200,
            delay: Duration::from_millis(3_000),
        }
    }
}

pub(crate) struct BytePlusVideoAdapter<T = ReqwestBytePlusVideoTransport> {
    api_token: String,
    submit_url: String,
    provider_id: String,
    transport: T,
}

impl BytePlusVideoAdapter<ReqwestBytePlusVideoTransport> {
    /// Creates a production BytePlus video adapter.
    pub(crate) fn new(
        api_token: impl Into<String>,
        submit_url: impl Into<String>,
        provider_id: impl Into<String>,
    ) -> Result<Self> {
        let api_token = api_token.into().trim().to_string();
        if api_token.is_empty() {
            bail!("video API token is required");
        }
        Ok(Self {
            api_token,
            submit_url: submit_url.into().trim_end_matches('/').to_string(),
            provider_id: provider_id.into(),
            transport: ReqwestBytePlusVideoTransport::default(),
        })
    }
}

impl<T> BytePlusVideoAdapter<T>
where
    T: BytePlusVideoTransport,
{
    #[cfg(test)]
    pub(crate) fn with_transport(
        api_token: impl Into<String>,
        submit_url: impl Into<String>,
        provider_id: impl Into<String>,
        transport: T,
    ) -> Self {
        Self {
            api_token: api_token.into().trim().to_string(),
            submit_url: submit_url.into().trim_end_matches('/').to_string(),
            provider_id: provider_id.into(),
            transport,
        }
    }

    /// Submits a BytePlus task and persists the queued job.
    pub(crate) fn submit(
        &self,
        service: &MediaGenerationService,
        request: BytePlusVideoRequest,
        selected_parameters: BTreeMap<String, String>,
        now_ms: u64,
    ) -> Result<MediaJob> {
        let response = self.transport.submit_task(
            &self.submit_url,
            &self.api_token,
            &request.request_body(),
        )?;
        let task = BytePlusVideoTask::from_submit_value(response).with_context(|| {
            format!(
                "provider={} adapter={BYTEPLUS_VIDEO_ADAPTER} task=unknown",
                self.provider_id
            )
        })?;
        let mut job = MediaJob::new(
            Uuid::new_v4().to_string(),
            MediaKind::Video,
            &self.provider_id,
            request.model.trim(),
            request.prompt.trim(),
            1,
            now_ms,
        );
        job.adapter = Some(BYTEPLUS_VIDEO_ADAPTER.to_string());
        job.parameters = selected_parameters;
        job.provider_job_id = Some(task.id.clone());
        self.apply_task(service, job, task, now_ms)
    }

    fn poll_url(&self, job: &MediaJob) -> Result<String> {
        let id = job
            .provider_job_id
            .as_ref()
            .context("video job is missing a task id")?;
        let mut url =
            reqwest::Url::parse(&self.submit_url).context("video submit URL must be absolute")?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow!("video submit URL cannot be a base"))?;
            segments.push(id);
        }
        Ok(url.to_string())
    }

    /// Polls a non-terminal BytePlus job once and persists the resulting state.
    pub(crate) fn poll(
        &self,
        service: &MediaGenerationService,
        job: MediaJob,
        now_ms: u64,
    ) -> Result<MediaJob> {
        if job.status.is_terminal() {
            return Ok(job);
        }
        let url = self.poll_url(&job)?;
        let response = self.transport.poll_task(&url, &self.api_token)?;
        let task = BytePlusVideoTask::from_poll_value(response)?;
        self.apply_task(service, job, task, now_ms)
    }

    /// Polls until a BytePlus job reaches a terminal status.
    pub(crate) fn poll_until_terminal(
        &self,
        service: &MediaGenerationService,
        mut job: MediaJob,
        config: BytePlusVideoPollingConfig,
        mut sleep: impl FnMut(Duration),
        mut now_ms: impl FnMut() -> u64,
    ) -> Result<MediaJob> {
        for attempt in 0..config.max_attempts {
            job = self.poll(service, job, now_ms())?;
            if job.status.is_terminal() {
                return Ok(job);
            }
            if attempt + 1 < config.max_attempts {
                sleep(config.delay);
            }
        }
        bail!(
            "video job `{}` did not reach a terminal status after {} polls",
            job.id,
            config.max_attempts
        )
    }

    fn apply_task(
        &self,
        service: &MediaGenerationService,
        mut job: MediaJob,
        task: BytePlusVideoTask,
        now_ms: u64,
    ) -> Result<MediaJob> {
        let status = task.media_status()?;
        match status {
            MediaJobStatus::Queued | MediaJobStatus::Running => {
                job.transition(status, now_ms)?;
                service.save_job(&job)?;
                Ok(job)
            }
            MediaJobStatus::Succeeded => self.complete_succeeded(service, job, &task, now_ms),
            MediaJobStatus::Failed => {
                job.error = task.error.clone().or(Some("video task failed".to_string()));
                job.transition(MediaJobStatus::Failed, now_ms)?;
                service.save_job(&job)?;
                Ok(job)
            }
            MediaJobStatus::Canceled => {
                job.transition(MediaJobStatus::Canceled, now_ms)?;
                service.save_job(&job)?;
                Ok(job)
            }
        }
    }

    fn complete_succeeded(
        &self,
        service: &MediaGenerationService,
        mut job: MediaJob,
        task: &BytePlusVideoTask,
        now_ms: u64,
    ) -> Result<MediaJob> {
        if !job.artifact_ids.is_empty() {
            job.transition(MediaJobStatus::Succeeded, now_ms)?;
            service.save_job(&job)?;
            return Ok(job);
        }
        let url = task
            .video_url
            .clone()
            .context("completed video task is missing content.video_url")?;
        let bytes = match self.transport.download_bytes(&url) {
            Ok(bytes) => bytes,
            Err(error) => {
                job.error = Some(format!("{error:#}"));
                job.transition(MediaJobStatus::Failed, now_ms)?;
                service.save_job(&job)?;
                return Err(error);
            }
        };
        let artifact_id = Uuid::new_v4().to_string();
        let path = service.write_artifact_bytes(
            &artifact_id,
            &format!("byteplus-video-{artifact_id}.mp4"),
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
                "provider": self.provider_id,
                "taskId": task.id,
                "remoteStatus": task.status,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn submit_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("fixtures/byteplus_submit_task.json")).expect("fixture")
    }

    fn poll_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("fixtures/byteplus_poll_task.json")).expect("fixture")
    }

    #[test]
    fn parses_byteplus_submit_fixture() {
        let task = BytePlusVideoTask::from_submit_value(submit_fixture()).expect("task");
        assert!(!task.id.trim().is_empty());
    }

    #[test]
    fn parses_byteplus_poll_fixture() {
        let task = BytePlusVideoTask::from_poll_value(poll_fixture()).expect("task");
        assert!(!task.status.trim().is_empty());
    }

    #[test]
    fn byteplus_request_body_contains_model_and_prompt() {
        let request = BytePlusVideoRequest {
            model: "dreamina-seedance-2-0-260128".to_string(),
            prompt: "a cat".to_string(),
            params: vec![],
        };
        let body = request.request_body();
        assert_eq!(body["model"], json!("dreamina-seedance-2-0-260128"));
        assert_eq!(body["content"][0]["type"], json!("text"));
        assert_eq!(body["content"][0]["text"], json!("a cat"));
    }
}
