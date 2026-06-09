use super::capabilities::MediaCapabilityParameter;
use super::video_jobs::{
    complete_video_job, persist_failed_video_job, poll_video_until_terminal, video_poll_url,
    CompletedVideoTask, VideoPollingConfig,
};
use super::{MediaGenerationService, MediaJob, MediaJobStatus, MediaKind};
use anyhow::{anyhow, bail, Context, Result};
use puffer_provider_registry::MediaParameterWireType;
use reqwest::blocking::Client;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

const BYTEPLUS_VIDEO_ADAPTER: &str = "byteplus_video";

#[derive(Debug)]
pub(crate) struct BytePlusVideoRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) params: Vec<(String, Value)>,
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
        // Default to silent video. BytePlus Seedance models default
        // `generate_audio` to true, and their auto-generated audio reliably
        // trips content moderation ("output audio may contain sensitive
        // information"), failing every job — even benign prompts. Inserted
        // before params so a future `generate_audio` parameter can override.
        body.insert("generate_audio".to_string(), Value::Bool(false));
        for (field, value) in &self.params {
            body.insert(field.trim().to_string(), value.clone());
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

fn byteplus_request_value(parameter: &MediaCapabilityParameter, value: &str) -> Result<Value> {
    let value = value.trim();
    match parameter.wire_type {
        MediaParameterWireType::String => Ok(json!(value)),
        MediaParameterWireType::Number => {
            value.parse::<u64>().map(Value::from).with_context(|| {
                format!(
                    "video generation parameter {}={} must be a number",
                    parameter.name, value
                )
            })
        }
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
        params.push((field, byteplus_request_value(parameter, &value)?));
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

    fn media_status(&self) -> MediaJobStatus {
        super::video_jobs::map_video_task_status(&self.status)
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

fn byteplus_task_error_context(
    provider_id: &str,
    task_id: Option<&str>,
    error: &anyhow::Error,
) -> String {
    format!(
        "{error:#}: provider={provider_id} adapter={BYTEPLUS_VIDEO_ADAPTER} task={}",
        task_id.unwrap_or("unknown")
    )
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

pub(crate) type BytePlusVideoPollingConfig = VideoPollingConfig;

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
        let url = video_poll_url(&self.submit_url, &job)?;
        let response = self.transport.poll_task(&url, &self.api_token)?;
        let task = match BytePlusVideoTask::from_poll_value(response) {
            Ok(task) => task,
            Err(error) => {
                let diagnostic = byteplus_task_error_context(
                    &self.provider_id,
                    job.provider_job_id.as_deref(),
                    &error,
                );
                persist_failed_video_job(service, job, diagnostic.clone(), now_ms)?;
                return Err(anyhow!(diagnostic));
            }
        };
        self.apply_task(service, job, task, now_ms)
    }

    /// Polls until a BytePlus job reaches a terminal status.
    pub(crate) fn poll_until_terminal(
        &self,
        service: &MediaGenerationService,
        job: MediaJob,
        config: BytePlusVideoPollingConfig,
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
        task: BytePlusVideoTask,
        now_ms: u64,
    ) -> Result<MediaJob> {
        let status = task.media_status();
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
        task: &BytePlusVideoTask,
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
                filename_prefix: "byteplus-video",
                missing_url_message: "completed video task is missing content.video_url",
            },
            now_ms,
            |url| self.transport.download_bytes(url),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use puffer_provider_registry::MediaParameterWireType;
    use serde_json::json;
    use std::cell::RefCell;

    fn submit_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("fixtures/byteplus_submit_task.json")).expect("fixture")
    }

    fn poll_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("fixtures/byteplus_poll_task.json")).expect("fixture")
    }

    struct ScriptedTransport {
        submit: Value,
        polls: RefCell<Vec<Value>>,
    }

    impl BytePlusVideoTransport for ScriptedTransport {
        fn submit_task(&self, _url: &str, _api_token: &str, _body: &Value) -> Result<Value> {
            Ok(self.submit.clone())
        }

        fn poll_task(&self, _url: &str, _api_token: &str) -> Result<Value> {
            Ok(self.polls.borrow_mut().remove(0))
        }

        fn download_bytes(&self, _url: &str) -> Result<Vec<u8>> {
            Ok(b"MP4BYTES".to_vec())
        }
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
            model: "dreamina-seedance-2-0-fast-260128".to_string(),
            prompt: "a cat".to_string(),
            params: vec![],
        };
        let body = request.request_body();
        assert_eq!(body["model"], json!("dreamina-seedance-2-0-fast-260128"));
        assert_eq!(body["content"][0]["type"], json!("text"));
        assert_eq!(body["content"][0]["text"], json!("a cat"));
    }

    #[test]
    fn byteplus_request_body_encodes_duration_as_number() {
        let request = BytePlusVideoRequest {
            model: "dreamina-seedance-2-0-fast-260128".to_string(),
            prompt: "a cat".to_string(),
            params: vec![
                ("duration".to_string(), json!(5)),
                ("ratio".to_string(), json!("16:9")),
            ],
        };
        let body = request.request_body();

        assert_eq!(body["duration"], json!(5));
        assert_eq!(body["ratio"], json!("16:9"));
    }

    #[test]
    fn byteplus_request_body_defaults_to_silent_video() {
        let request = BytePlusVideoRequest {
            model: "dreamina-seedance-2-0-fast-260128".to_string(),
            prompt: "a cat".to_string(),
            params: vec![],
        };
        let body = request.request_body();

        // Silent by default; BytePlus audio moderation otherwise fails the job.
        assert_eq!(body["generate_audio"], json!(false));
    }

    #[test]
    fn byteplus_request_body_allows_generate_audio_override() {
        let request = BytePlusVideoRequest {
            model: "dreamina-seedance-2-0-fast-260128".to_string(),
            prompt: "a cat".to_string(),
            params: vec![("generate_audio".to_string(), json!(true))],
        };
        let body = request.request_body();

        assert_eq!(body["generate_audio"], json!(true));
    }

    #[test]
    fn byteplus_request_body_uses_parameter_wire_type_for_numbers() {
        let request = byteplus_video_request_from_parameters(
            "dreamina-seedance-2-0-260128".to_string(),
            "animate a calm lake".to_string(),
            &[MediaCapabilityParameter {
                name: "duration_seconds".to_string(),
                label: "Duration".to_string(),
                values: vec!["4".to_string(), "5".to_string()],
                default: "5".to_string(),
                request_field: Some("duration".to_string()),
                wire_type: MediaParameterWireType::Number,
            }],
            &BTreeMap::from([("duration_seconds".to_string(), "5".to_string())]),
        )
        .expect("request");

        let body = request.request_body();

        assert_eq!(body["duration"], json!(5));
    }

    #[test]
    fn byteplus_rejects_invalid_number_wire_value_before_http() {
        let error = byteplus_video_request_from_parameters(
            "dreamina-seedance-2-0-260128".to_string(),
            "animate a calm lake".to_string(),
            &[MediaCapabilityParameter {
                name: "duration_seconds".to_string(),
                label: "Duration".to_string(),
                values: vec!["fast".to_string()],
                default: "fast".to_string(),
                request_field: Some("duration".to_string()),
                wire_type: MediaParameterWireType::Number,
            }],
            &BTreeMap::new(),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("duration_seconds"));
        assert!(error.contains("number"));
    }

    #[test]
    fn poll_parser_failure_marks_job_failed() {
        let dir = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(dir.path());
        let adapter = BytePlusVideoAdapter::with_transport(
            "token",
            "https://ark.ap-southeast.bytepluses.com/api/v3/contents/generations/tasks",
            "byteplus",
            ScriptedTransport {
                submit: json!({ "id": "task-1" }),
                polls: RefCell::new(vec![json!({ "status": "running" })]),
            },
        );
        let request = BytePlusVideoRequest {
            model: "dreamina-seedance-2-0-260128".to_string(),
            prompt: "a cat".to_string(),
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
            .is_some_and(|value| value.contains("poll video task response missing task id")));
        assert!(saved
            .error
            .as_deref()
            .is_some_and(|value| value.contains("provider=byteplus")));
        assert!(saved
            .error
            .as_deref()
            .is_some_and(|value| value.contains("adapter=byteplus_video")));
        assert!(saved
            .error
            .as_deref()
            .is_some_and(|value| value.contains("task=task-1")));
    }
}
