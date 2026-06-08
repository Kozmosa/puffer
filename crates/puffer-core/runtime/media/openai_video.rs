use super::capabilities::MediaCapabilityParameter;
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// One OpenAI-compatible video generation request (`POST /v1/video/generations`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenAiVideoRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    /// Ordered (request_field, value) pairs. A `request_field` of `metadata.<k>`
    /// is nested under the body's `metadata` object; otherwise it is top-level.
    pub(crate) params: Vec<(String, String)>,
}

const METADATA_PREFIX: &str = "metadata.";

impl OpenAiVideoRequest {
    fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty() {
            bail!("video model is required");
        }
        if self.prompt.trim().is_empty() {
            bail!("video prompt is required");
        }
        Ok(())
    }

    /// Builds the `POST /v1/video/generations` request body.
    pub(crate) fn request_body(&self) -> Value {
        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model.trim()));
        body.insert("prompt".to_string(), json!(self.prompt.trim()));
        body.insert("n".to_string(), json!(1));

        let mut metadata = Map::new();
        for (field, value) in &self.params {
            let field = field.trim();
            if let Some(key) = field.strip_prefix(METADATA_PREFIX) {
                metadata.insert(key.to_string(), json!(value.trim()));
            } else {
                body.insert(field.to_string(), json!(value.trim()));
            }
        }
        if !metadata.is_empty() {
            body.insert("metadata".to_string(), Value::Object(metadata));
        }
        Value::Object(body)
    }
}

/// Maps a validated selection's parameters into an OpenAI-video request.
///
/// Emits params in capability order using each parameter's `request_field`
/// (only parameters that declare one). The selected value (already defaulted
/// by the caller) is used, falling back to the parameter default.
pub(crate) fn openai_video_request_from_parameters(
    model_id: String,
    prompt: String,
    capability_parameters: &[MediaCapabilityParameter],
    selected: &BTreeMap<String, String>,
) -> Result<OpenAiVideoRequest> {
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
    let request = OpenAiVideoRequest {
        model: model_id,
        prompt,
        params,
    };
    request.validate()?;
    Ok(request)
}

use super::MediaJobStatus;
use anyhow::Context;
use reqwest::blocking::Client;

/// Abstracts OpenAI-compatible video HTTP operations for production and tests.
pub(crate) trait OpenAiVideoTransport {
    /// Submits a video task and returns its JSON response.
    fn submit_task(&self, url: &str, api_token: &str, body: &Value) -> Result<Value>;

    /// Polls a video task URL and returns its JSON response.
    fn poll_task(&self, url: &str, api_token: &str) -> Result<Value>;

    /// Downloads the rendered video bytes from a validated remote URL.
    fn download_bytes(&self, url: &str) -> Result<Vec<u8>>;
}

/// Reqwest-backed transport used by the runtime adapter.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReqwestOpenAiVideoTransport {
    client: Client,
}

impl OpenAiVideoTransport for ReqwestOpenAiVideoTransport {
    fn submit_task(&self, url: &str, api_token: &str, body: &Value) -> Result<Value> {
        let response = self
            .client
            .post(url)
            .bearer_auth(api_token)
            .json(body)
            .send()
            .with_context(|| format!("submit video task {url}"))?;
        openai_video_json_response(response, "submit video task")
    }

    fn poll_task(&self, url: &str, api_token: &str) -> Result<Value> {
        let response = self
            .client
            .get(url)
            .bearer_auth(api_token)
            .send()
            .with_context(|| format!("poll video task {url}"))?;
        openai_video_json_response(response, "poll video task")
    }

    fn download_bytes(&self, url: &str) -> Result<Vec<u8>> {
        // Reuse the shared https/loopback-enforcing downloader (image-path parity).
        super::http_support::download_image_url(&self.client, url, "video output")
    }
}

fn openai_video_json_response(response: reqwest::blocking::Response, label: &str) -> Result<Value> {
    let status = response.status();
    let text = response
        .text()
        .with_context(|| format!("read {label} response body"))?;
    if !status.is_success() {
        bail!("{label} failed with status {}: {text}", status.as_u16());
    }
    serde_json::from_str(&text).with_context(|| format!("parse {label} response JSON"))
}

/// Normalized view of an OpenAI-compatible video task response.
///
/// Envelope confirmed from New API `dto/openai_video.go`: `id`, `status`
/// (`queued|in_progress|completed|failed`), the video URL at `metadata.url`,
/// and `error.{message,code}`.
#[derive(Debug, Clone)]
pub(crate) struct OpenAiVideoTask {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) video_url: Option<String>,
    pub(crate) error: Option<String>,
}

impl OpenAiVideoTask {
    pub(crate) fn from_value(value: Value) -> Result<Self> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .context("video task response missing `id`")?
            .to_string();
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .context("video task response missing `status`")?
            .to_string();
        let video_url = value
            .get("metadata")
            .and_then(|metadata| metadata.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let error = value.get("error").and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        Ok(Self {
            id,
            status,
            video_url,
            error,
        })
    }

    /// Maps the OpenAI-video status string onto a media job status.
    pub(crate) fn media_status(&self) -> Result<MediaJobStatus> {
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

use super::{MediaArtifact, MediaGenerationService, MediaJob, MediaKind};
use std::time::Duration;
use uuid::Uuid;

const VIDEO_MIME_TYPE: &str = "video/mp4";
const OPENAI_VIDEO_ADAPTER: &str = "openai_video";

/// Bounded backoff while polling video tasks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenAiVideoPollingConfig {
    pub(crate) max_attempts: usize,
    pub(crate) delay: Duration,
}

impl Default for OpenAiVideoPollingConfig {
    fn default() -> Self {
        // Video renders take minutes; poll every 3s up to ~10 minutes.
        Self {
            max_attempts: 200,
            delay: Duration::from_millis(3_000),
        }
    }
}

/// Submits and polls OpenAI-compatible video tasks into media jobs.
pub(crate) struct OpenAiVideoAdapter<T = ReqwestOpenAiVideoTransport> {
    api_token: String,
    submit_url: String,
    provider_id: String,
    transport: T,
}

impl OpenAiVideoAdapter<ReqwestOpenAiVideoTransport> {
    /// Creates a production adapter. `submit_url` is the absolute task-creation
    /// URL built by the caller via `provider_execution_url`.
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
            transport: ReqwestOpenAiVideoTransport::default(),
        })
    }
}

impl<T> OpenAiVideoAdapter<T>
where
    T: OpenAiVideoTransport,
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

    /// Submits a task and persists the queued job (task id in `provider_job_id`).
    pub(crate) fn submit(
        &self,
        service: &MediaGenerationService,
        request: OpenAiVideoRequest,
        now_ms: u64,
    ) -> Result<MediaJob> {
        let response =
            self.transport
                .submit_task(&self.submit_url, &self.api_token, &request.request_body())?;
        let task = OpenAiVideoTask::from_value(response)?;
        let mut job = MediaJob::new(
            Uuid::new_v4().to_string(),
            MediaKind::Video,
            &self.provider_id,
            request.model.trim(),
            request.prompt.trim(),
            1,
            now_ms,
        );
        job.adapter = Some(OPENAI_VIDEO_ADAPTER.to_string());
        job.provider_job_id = Some(task.id.clone());
        self.apply_task(service, job, task, now_ms)
    }

    fn poll_url(&self, job: &MediaJob) -> Result<String> {
        let id = job
            .provider_job_id
            .as_ref()
            .context("video job is missing a task id")?;
        Ok(format!("{}/{id}", self.submit_url))
    }

    /// Polls a non-terminal job once and persists the resulting state.
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
        let task = OpenAiVideoTask::from_value(response)?;
        self.apply_task(service, job, task, now_ms)
    }

    /// Polls until the job reaches a terminal status.
    pub(crate) fn poll_until_terminal(
        &self,
        service: &MediaGenerationService,
        mut job: MediaJob,
        config: OpenAiVideoPollingConfig,
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
        task: OpenAiVideoTask,
        now_ms: u64,
    ) -> Result<MediaJob> {
        match task.media_status()? {
            MediaJobStatus::Queued | MediaJobStatus::Running => {
                job.transition(task.media_status()?, now_ms)?;
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
        task: &OpenAiVideoTask,
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
            .context("completed video task is missing `metadata.url`")?;
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
            &format!("openai-video-{artifact_id}.mp4"),
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

    fn parameter(name: &str, request_field: &str, default: &str) -> MediaCapabilityParameter {
        MediaCapabilityParameter {
            name: name.to_string(),
            label: name.to_string(),
            values: vec![default.to_string()],
            default: default.to_string(),
            request_field: Some(request_field.to_string()),
        }
    }

    #[test]
    fn splits_top_level_and_metadata_params() {
        let params = vec![
            parameter("duration", "seconds", "5"),
            parameter("resolution", "metadata.resolution", "720p"),
            parameter("ratio", "metadata.ratio", "16:9"),
        ];
        let mut selected = BTreeMap::new();
        selected.insert("resolution".to_string(), "1080p".to_string());

        let request = openai_video_request_from_parameters(
            "m".to_string(),
            "a cat".to_string(),
            &params,
            &selected,
        )
        .expect("request");

        let body = request.request_body();
        assert_eq!(body["model"], json!("m"));
        assert_eq!(body["prompt"], json!("a cat"));
        assert_eq!(body["n"], json!(1));
        assert_eq!(body["seconds"], json!("5"));
        assert_eq!(body["metadata"]["resolution"], json!("1080p"));
        assert_eq!(body["metadata"]["ratio"], json!("16:9"));
    }

    #[test]
    fn omits_metadata_when_no_metadata_params() {
        let params = vec![parameter("duration", "seconds", "5")];
        let request = openai_video_request_from_parameters(
            "m".to_string(),
            "a cat".to_string(),
            &params,
            &BTreeMap::new(),
        )
        .expect("request");
        let body = request.request_body();
        assert!(body.get("metadata").is_none());
    }

    #[test]
    fn rejects_empty_prompt() {
        let error = openai_video_request_from_parameters(
            "m".to_string(),
            "   ".to_string(),
            &[],
            &BTreeMap::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("prompt is required"));
    }

    #[test]
    fn parses_completed_task_with_metadata_url() {
        let value = json!({
            "id": "vid-1",
            "status": "completed",
            "metadata": { "url": "https://cdn.example.com/v.mp4" }
        });
        let task = OpenAiVideoTask::from_value(value).expect("task");
        assert_eq!(task.id, "vid-1");
        assert_eq!(task.media_status().unwrap(), MediaJobStatus::Succeeded);
        assert_eq!(task.video_url.as_deref(), Some("https://cdn.example.com/v.mp4"));
    }

    #[test]
    fn parses_failed_task_error_message() {
        let value = json!({
            "id": "vid-2",
            "status": "failed",
            "error": { "code": "x", "message": "content blocked" }
        });
        let task = OpenAiVideoTask::from_value(value).expect("task");
        assert_eq!(task.media_status().unwrap(), MediaJobStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("content blocked"));
    }

    #[test]
    fn rejects_unknown_status() {
        let value = json!({ "id": "v", "status": "weird" });
        let task = OpenAiVideoTask::from_value(value).expect("task");
        assert!(task
            .media_status()
            .unwrap_err()
            .to_string()
            .contains("unknown video task status"));
    }

    use super::super::MediaGenerationService;
    use std::cell::RefCell;

    struct ScriptedTransport {
        submit: Value,
        polls: RefCell<Vec<Value>>,
    }

    impl OpenAiVideoTransport for ScriptedTransport {
        fn submit_task(&self, _url: &str, _token: &str, _body: &Value) -> Result<Value> {
            Ok(self.submit.clone())
        }
        fn poll_task(&self, _url: &str, _token: &str) -> Result<Value> {
            Ok(self.polls.borrow_mut().remove(0))
        }
        fn download_bytes(&self, _url: &str) -> Result<Vec<u8>> {
            Ok(b"MP4BYTES".to_vec())
        }
    }

    #[test]
    fn submit_then_poll_downloads_video_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(dir.path());
        let transport = ScriptedTransport {
            submit: json!({ "id": "vid-9", "status": "queued" }),
            polls: RefCell::new(vec![
                json!({ "id": "vid-9", "status": "in_progress" }),
                json!({ "id": "vid-9", "status": "completed", "metadata": { "url": "https://cdn.example.com/v.mp4" } }),
            ]),
        };
        let adapter = OpenAiVideoAdapter::with_transport(
            "token",
            "https://relaydance.com/v1/video/generations",
            "relaydance",
            transport,
        );

        let request = OpenAiVideoRequest {
            model: "m".into(),
            prompt: "a cat".into(),
            params: vec![],
        };
        let job = adapter.submit(&service, request, 1).expect("submit");
        let job = adapter
            .poll_until_terminal(
                &service,
                job,
                OpenAiVideoPollingConfig {
                    max_attempts: 5,
                    delay: Duration::from_millis(0),
                },
                |_| {},
                || 2,
            )
            .expect("poll");

        assert_eq!(job.status, MediaJobStatus::Succeeded);
        assert_eq!(job.artifact_ids.len(), 1);
    }
}
