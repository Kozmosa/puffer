use super::capabilities::MediaCapabilityParameter;
use super::video_jobs::{
    complete_video_job, persist_failed_video_job, poll_video_until_terminal, video_poll_url,
    CompletedVideoTask, VideoPollingConfig,
};
use super::{MediaGenerationService, MediaJob, MediaJobStatus, MediaKind};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

/// One Relaydance video generation request (`POST /v1/video/generations`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelaydanceVideoRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    /// Ordered (request_field, value) pairs. A `request_field` of `metadata.<k>`
    /// is nested under the body's `metadata` object; otherwise it is top-level.
    pub(crate) params: Vec<(String, String)>,
}

const METADATA_PREFIX: &str = "metadata.";

impl RelaydanceVideoRequest {
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

/// Maps a validated selection's parameters into a Relaydance video request.
///
/// Emits params in capability order using each parameter's `request_field`
/// (only parameters that declare one). The selected value (already defaulted
/// by the caller) is used, falling back to the parameter default.
pub(crate) fn relaydance_video_request_from_parameters(
    model_id: String,
    prompt: String,
    capability_parameters: &[MediaCapabilityParameter],
    selected: &BTreeMap<String, String>,
) -> Result<RelaydanceVideoRequest> {
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
    let request = RelaydanceVideoRequest {
        model: model_id,
        prompt,
        params,
    };
    request.validate()?;
    Ok(request)
}

/// Abstracts Relaydance video HTTP operations for production and tests.
pub(crate) trait RelaydanceVideoTransport {
    /// Submits a video task and returns its JSON response.
    fn submit_task(&self, url: &str, api_token: &str, body: &Value) -> Result<Value>;

    /// Polls a video task URL and returns its JSON response.
    fn poll_task(&self, url: &str, api_token: &str) -> Result<Value>;

    /// Downloads the rendered video bytes from a validated remote URL.
    fn download_bytes(&self, url: &str) -> Result<Vec<u8>>;
}

/// Reqwest-backed transport used by the runtime adapter.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReqwestRelaydanceVideoTransport {
    client: Client,
}

impl RelaydanceVideoTransport for ReqwestRelaydanceVideoTransport {
    fn submit_task(&self, url: &str, api_token: &str, body: &Value) -> Result<Value> {
        let response = self
            .client
            .post(url)
            .bearer_auth(api_token)
            .json(body)
            .send()
            .with_context(|| format!("submit video task {url}"))?;
        relaydance_video_json_response(response, "submit video task")
    }

    fn poll_task(&self, url: &str, api_token: &str) -> Result<Value> {
        let response = self
            .client
            .get(url)
            .bearer_auth(api_token)
            .send()
            .with_context(|| format!("poll video task {url}"))?;
        relaydance_video_json_response(response, "poll video task")
    }

    fn download_bytes(&self, url: &str) -> Result<Vec<u8>> {
        // Reuse the shared https/loopback-enforcing downloader (image-path parity).
        super::http_support::download_image_url(&self.client, url, "video output")
    }
}

fn relaydance_video_json_response(
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

/// Normalized view of a Relaydance video task response.
///
/// Envelope confirmed from New API `dto/relaydance_video.go`: `id`, `status`
/// (`queued|in_progress|completed|failed`), the video URL at `metadata.url`,
/// and `error.{message,code}`.
#[derive(Debug, Clone)]
pub(crate) struct RelaydanceVideoTask {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) video_url: Option<String>,
    pub(crate) error: Option<String>,
}

impl RelaydanceVideoTask {
    pub(crate) fn from_value(value: Value, phase: &str) -> Result<Self> {
        let body = value.get("data").unwrap_or(&value);
        let id = string_field(body, &["id", "task_id"])
            .or_else(|| string_field(&value, &["id", "task_id"]))
            .ok_or_else(|| {
                anyhow!(
                    "{phase} response missing task id: {}",
                    relaydance_response_shape_summary(&value)
                )
            })?;
        let status = string_field(body, &["status"])
            .or_else(|| string_field(&value, &["status"]))
            .ok_or_else(|| {
                anyhow!(
                    "{phase} response missing status: {}",
                    relaydance_response_shape_summary(&value)
                )
            })?;
        let video_url = relaydance_video_url(body).or_else(|| relaydance_video_url(&value));
        let error = relaydance_error_message(body).or_else(|| relaydance_error_message(&value));
        Ok(Self {
            id,
            status,
            video_url,
            error,
        })
    }

    /// Maps the Relaydance status string onto a media job status.
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

fn relaydance_response_shape_summary(value: &serde_json::Value) -> String {
    let keys = value
        .as_object()
        .map(|object| {
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            keys.join(",")
        })
        .unwrap_or_else(|| value_type_name(value).to_string());
    format!("keys=[{keys}]")
}

fn value_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn relaydance_error_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .or_else(|| value.get("message"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

fn relaydance_task_error_context(
    provider_id: &str,
    task_id: Option<&str>,
    error: &anyhow::Error,
) -> String {
    format!(
        "{error:#}: provider={provider_id} adapter={RELAYDANCE_VIDEO_ADAPTER} task={}",
        task_id.unwrap_or("unknown")
    )
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value
            .get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn relaydance_video_url(value: &Value) -> Option<String> {
    value
        .get("metadata")
        .and_then(|metadata| metadata.get("url"))
        .or_else(|| value.get("url"))
        .or_else(|| value.get("video_url"))
        .or_else(|| value.get("result_url"))
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| data.get("content"))
                .and_then(|content| content.get("video_url"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

const RELAYDANCE_VIDEO_ADAPTER: &str = "relaydance_video";

pub(crate) type RelaydanceVideoPollingConfig = VideoPollingConfig;

/// Submits and polls Relaydance video tasks into media jobs.
pub(crate) struct RelaydanceVideoAdapter<T = ReqwestRelaydanceVideoTransport> {
    api_token: String,
    submit_url: String,
    provider_id: String,
    transport: T,
}

impl RelaydanceVideoAdapter<ReqwestRelaydanceVideoTransport> {
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
            transport: ReqwestRelaydanceVideoTransport::default(),
        })
    }
}

impl<T> RelaydanceVideoAdapter<T>
where
    T: RelaydanceVideoTransport,
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
    ///
    /// `selected_parameters` are the user-facing name->value pairs (display
    /// parity with `replicate_video`, so duration/resolution/ratio show in UI).
    pub(crate) fn submit(
        &self,
        service: &MediaGenerationService,
        request: RelaydanceVideoRequest,
        selected_parameters: BTreeMap<String, String>,
        now_ms: u64,
    ) -> Result<MediaJob> {
        let response = self.transport.submit_task(
            &self.submit_url,
            &self.api_token,
            &request.request_body(),
        )?;
        let task =
            RelaydanceVideoTask::from_value(response, "submit video task").with_context(|| {
                format!(
                    "provider={} adapter={RELAYDANCE_VIDEO_ADAPTER} task=unknown",
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
        job.adapter = Some(RELAYDANCE_VIDEO_ADAPTER.to_string());
        job.parameters = selected_parameters;
        job.provider_job_id = Some(task.id.clone());
        self.apply_task(service, job, task, now_ms)
    }

    fn poll_url(&self, job: &MediaJob) -> Result<String> {
        video_poll_url(&self.submit_url, job)
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
        let task = match RelaydanceVideoTask::from_value(response, "poll video task") {
            Ok(task) => task,
            Err(error) => {
                let mut failed = job;
                let diagnostic = relaydance_task_error_context(
                    &self.provider_id,
                    failed.provider_job_id.as_deref(),
                    &error,
                );
                failed.error = Some(diagnostic.clone());
                failed.transition(MediaJobStatus::Failed, now_ms)?;
                service.save_job(&failed)?;
                return Err(anyhow!(diagnostic));
            }
        };
        self.apply_task(service, job, task, now_ms)
    }

    /// Polls until the job reaches a terminal status.
    pub(crate) fn poll_until_terminal(
        &self,
        service: &MediaGenerationService,
        job: MediaJob,
        config: RelaydanceVideoPollingConfig,
        sleep: impl FnMut(Duration),
        now_ms: impl FnMut() -> u64,
    ) -> Result<MediaJob> {
        poll_video_until_terminal(job, config, sleep, now_ms, |job, now_ms| {
            self.poll(service, job, now_ms)
        })
    }

    fn apply_task(
        &self,
        service: &MediaGenerationService,
        mut job: MediaJob,
        task: RelaydanceVideoTask,
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
            MediaJobStatus::Failed => persist_failed_video_job(
                service,
                job,
                task.error
                    .clone()
                    .unwrap_or_else(|| "video task failed".to_string()),
                now_ms,
            ),
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
        job: MediaJob,
        task: &RelaydanceVideoTask,
        now_ms: u64,
    ) -> Result<MediaJob> {
        complete_video_job(
            service,
            job,
            CompletedVideoTask {
                provider_id: &self.provider_id,
                task_id: &task.id,
                remote_status: &task.status,
                video_url: task.video_url.as_deref(),
                filename_prefix: "relaydance-video",
                missing_url_message: "completed video task is missing `metadata.url`",
            },
            now_ms,
            |url| self.transport.download_bytes(url),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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

        let request = relaydance_video_request_from_parameters(
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
        let request = relaydance_video_request_from_parameters(
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
        let error = relaydance_video_request_from_parameters(
            "m".to_string(),
            "   ".to_string(),
            &[],
            &BTreeMap::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("prompt is required"));
    }

    fn relaydance_poll_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("fixtures/relaydance_poll_task.json")).expect("fixture")
    }

    #[test]
    fn parses_relaydance_poll_fixture() {
        let task = RelaydanceVideoTask::from_value(relaydance_poll_fixture(), "poll video task")
            .expect("task");
        assert!(!task.id.trim().is_empty());
        assert!(!task.status.trim().is_empty());
    }

    #[test]
    fn relaydance_shape_summary_lists_top_level_keys() {
        let summary = relaydance_response_shape_summary(&relaydance_poll_fixture());
        assert!(summary.contains("keys=["));
    }

    #[test]
    fn parses_relaydance_completed_task_with_task_id_and_url() {
        let task = RelaydanceVideoTask::from_value(
            json!({
                "task_id": "task-1",
                "status": "succeeded",
                "url": "https://example.com/video.mp4"
            }),
            "poll video task",
        )
        .expect("task");

        assert_eq!(task.id, "task-1");
        assert_eq!(task.status, "succeeded");
        assert_eq!(
            task.video_url.as_deref(),
            Some("https://example.com/video.mp4")
        );
    }

    #[test]
    fn relaydance_missing_task_id_reports_phase_and_keys() {
        let error = RelaydanceVideoTask::from_value(
            json!({
                "code": "ok",
                "message": "accepted",
                "data": { "status": "running" }
            }),
            "poll video task",
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("poll video task response missing task id"));
        assert!(error.contains("keys=[code,data,message]"));
    }

    #[test]
    fn parses_completed_task_with_metadata_url() {
        let value = json!({
            "id": "vid-1",
            "status": "completed",
            "metadata": { "url": "https://cdn.example.com/v.mp4" }
        });
        let task = RelaydanceVideoTask::from_value(value, "poll video task").expect("task");
        assert_eq!(task.id, "vid-1");
        assert_eq!(task.media_status().unwrap(), MediaJobStatus::Succeeded);
        assert_eq!(
            task.video_url.as_deref(),
            Some("https://cdn.example.com/v.mp4")
        );
    }

    #[test]
    fn parses_failed_task_error_message() {
        let value = json!({
            "id": "vid-2",
            "status": "failed",
            "error": { "code": "x", "message": "content blocked" }
        });
        let task = RelaydanceVideoTask::from_value(value, "poll video task").expect("task");
        assert_eq!(task.media_status().unwrap(), MediaJobStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("content blocked"));
    }

    #[test]
    fn rejects_unknown_status() {
        let value = json!({ "id": "v", "status": "weird" });
        let task = RelaydanceVideoTask::from_value(value, "poll video task").expect("task");
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

    impl RelaydanceVideoTransport for ScriptedTransport {
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
        let adapter = RelaydanceVideoAdapter::with_transport(
            "token",
            "https://relaydance.com/v1/video/generations",
            "relaydance",
            transport,
        );

        let request = RelaydanceVideoRequest {
            model: "m".into(),
            prompt: "a cat".into(),
            params: vec![],
        };
        let job = adapter
            .submit(&service, request, BTreeMap::new(), 1)
            .expect("submit");
        let job = adapter
            .poll_until_terminal(
                &service,
                job,
                RelaydanceVideoPollingConfig {
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

    #[test]
    fn poll_url_preserves_submit_url_query() {
        let adapter = RelaydanceVideoAdapter::with_transport(
            "token",
            "https://relaydance.com/v1/video/generations?token=x",
            "relaydance",
            ScriptedTransport {
                submit: json!({}),
                polls: RefCell::new(vec![]),
            },
        );
        let mut job = MediaJob::new(
            "job-1".to_string(),
            MediaKind::Video,
            "relaydance",
            "m",
            "a cat",
            1,
            1,
        );
        job.provider_job_id = Some("vid-9".to_string());
        assert_eq!(
            adapter.poll_url(&job).expect("poll url"),
            "https://relaydance.com/v1/video/generations/vid-9?token=x"
        );
    }

    #[test]
    fn submit_persists_selected_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(dir.path());
        let adapter = RelaydanceVideoAdapter::with_transport(
            "token",
            "https://relaydance.com/v1/video/generations",
            "relaydance",
            ScriptedTransport {
                submit: json!({ "id": "vid-9", "status": "queued" }),
                polls: RefCell::new(vec![]),
            },
        );
        let request = RelaydanceVideoRequest {
            model: "m".into(),
            prompt: "a cat".into(),
            params: vec![],
        };
        let selected = BTreeMap::from([
            ("duration".to_string(), "5".to_string()),
            ("resolution".to_string(), "1080p".to_string()),
        ]);
        let job = adapter
            .submit(&service, request, selected.clone(), 1)
            .expect("submit");
        assert_eq!(job.parameters, selected);
    }

    #[test]
    fn poll_parser_failure_marks_job_failed() {
        let dir = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(dir.path());
        let adapter = RelaydanceVideoAdapter::with_transport(
            "token",
            "https://relaydance.com/v1/video/generations",
            "relaydance",
            ScriptedTransport {
                submit: json!({ "id": "task-1", "status": "queued" }),
                polls: RefCell::new(vec![json!({ "data": { "status": "running" } })]),
            },
        );
        let request = RelaydanceVideoRequest {
            model: "m".into(),
            prompt: "a cat".into(),
            params: vec![],
        };
        let job = adapter
            .submit(&service, request, BTreeMap::new(), 1)
            .expect("submit");

        let error = adapter
            .poll(&service, job.clone(), 2)
            .unwrap_err()
            .to_string();
        assert!(error.contains("poll video task response missing task id"));

        let saved = service.load_job(&job.id).expect("saved job");
        assert_eq!(saved.status, MediaJobStatus::Failed);
        assert!(saved
            .error
            .as_deref()
            .is_some_and(|value| { value.contains("poll video task response missing task id") }));
        assert!(saved
            .error
            .as_deref()
            .is_some_and(|value| value.contains("provider=relaydance")));
        assert!(saved
            .error
            .as_deref()
            .is_some_and(|value| value.contains("adapter=relaydance_video")));
        assert!(saved
            .error
            .as_deref()
            .is_some_and(|value| value.contains("task=task-1")));
    }
}
