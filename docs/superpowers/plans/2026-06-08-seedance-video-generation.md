# Seedance Video Generation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make BytePlus Seedance video generation appear and work in the desktop "Video generation settings" modal by adding a `seedance_video` execution adapter and a `media.video` section to `byteplus.yaml`.

**Architecture:** Mirror the image path. Declare `media.video` in `byteplus.yaml` (direct ModelArk, same `base_url`). Add a new `seedance_video` execution adapter that reuses `http_support` helpers (`provider_execution_url`, `bearer_token`, `download_image_url`, secret redaction) like `images_json`, and reuses the `replicate_video` async job lifecycle (queued → poll → terminal `MediaJob`). The only genuinely new logic is mapping structured params into ModelArk's prompt-inline `--flag value` request body.

**Tech Stack:** Rust, `reqwest::blocking`, `serde_json`, `anyhow`, existing `MediaJob`/`MediaGenerationService` runtime.

**Spec:** `docs/superpowers/specs/2026-06-08-seedance-video-generation-design.md`

---

## File Structure

- `crates/puffer-provider-registry/src/model.rs` — add `SeedanceVideo` enum variant (modify).
- `crates/puffer-core/runtime/media/resolver.rs` — allow `(Video, SeedanceVideo)`, `adapter_id` mapping, new `resolve_video_execution_descriptor` (modify).
- `crates/puffer-core/runtime/media/seedance_video.rs` — new adapter module (create).
- `crates/puffer-core/runtime/media/mod.rs` — register module (modify).
- `crates/puffer-core/media_runtime.rs` — add `seedance_video` match arm (modify).
- `resources/providers/byteplus.yaml` — add `media.video` (modify).

---

## Task 0: Verify ModelArk facts (gate — do first)

The model id, parameter flags, and response shape below are research defaults. Confirm against the actual account before the YAML is final. If reality differs, update the constants in Task 6 (YAML) and Task 3 (flag mapping) accordingly — the code structure does not change.

- [ ] **Step 1: Confirm and record**

Confirm via ModelArk docs/console for the target account:
- Exact Seedance model id (default assumed: `dreamina-seedance-2-0-260128`).
- That resolution/ratio/duration are prompt-inline `--` flags and their allowed values.
- Submit endpoint `POST {base}/contents/generations/tasks` returns `{ "id": "<task_id>" }`; poll `GET {base}/contents/generations/tasks/{id}` returns `{ "status": "...", "content": { "video_url": "..." } }` with terminal status string `succeeded` and failure `failed`.

No code change in this task. Record any deltas as notes on Tasks 3 and 6.

---

## Task 1: Add `SeedanceVideo` execution kind

**Files:**
- Modify: `crates/puffer-provider-registry/src/model.rs` (`MediaExecutionKind`, ~line 418)
- Test: `crates/puffer-provider-registry/src/model_tests.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/puffer-provider-registry/src/model_tests.rs`:

```rust
#[test]
fn media_execution_kind_parses_seedance_video() {
    let kind: MediaExecutionKind = serde_yaml::from_str("seedance_video").expect("parse");
    assert_eq!(kind, MediaExecutionKind::SeedanceVideo);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p puffer-provider-registry media_execution_kind_parses_seedance_video`
Expected: FAIL — `SeedanceVideo` is not a variant / unknown variant `seedance_video`.

- [ ] **Step 3: Add the variant**

In `crates/puffer-provider-registry/src/model.rs`, add `SeedanceVideo` to the enum:

```rust
pub enum MediaExecutionKind {
    ImagesJson,
    ChatImageOutput,
    MinimaxImage,
    ReplicateVideo,
    SeedanceVideo,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p puffer-provider-registry media_execution_kind_parses_seedance_video`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/puffer-provider-registry/src/model.rs crates/puffer-provider-registry/src/model_tests.rs
git commit -m "feat(media): add SeedanceVideo execution kind"
```

---

## Task 2: Wire resolver (availability, adapter_id, video execution descriptor)

**Files:**
- Modify: `crates/puffer-core/runtime/media/resolver.rs`
  - `execution_adapter_is_available_for_kind` (~line 301)
  - `adapter_id` (the fn mapping `MediaExecutionKind` → `&str`, used at ~line 267)
  - add `resolve_video_execution_descriptor` (mirror `resolve_image_execution_descriptor`, ~line 129)

- [ ] **Step 1: Write the failing test**

Add to the resolver tests module (bottom of `resolver.rs`, where existing video capability tests live — search for `MediaKind::Video` test helpers, ~line 577):

```rust
#[test]
fn seedance_video_execution_adapter_is_available() {
    assert!(execution_adapter_is_available_for_kind(
        MediaKind::Video,
        MediaExecutionKind::SeedanceVideo
    ));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p puffer-core seedance_video_execution_adapter_is_available`
Expected: FAIL — returns `false`.

- [ ] **Step 3: Allow the pairing + adapter_id mapping**

In `execution_adapter_is_available_for_kind` add the arm:

```rust
fn execution_adapter_is_available_for_kind(kind: MediaKind, adapter: MediaExecutionKind) -> bool {
    matches!(
        (kind, adapter),
        (MediaKind::Image, MediaExecutionKind::ImagesJson)
            | (MediaKind::Image, MediaExecutionKind::ChatImageOutput)
            | (MediaKind::Image, MediaExecutionKind::MinimaxImage)
            | (MediaKind::Video, MediaExecutionKind::ReplicateVideo)
            | (MediaKind::Video, MediaExecutionKind::SeedanceVideo)
    )
}
```

In the `adapter_id` fn (the one returning the wire string per `MediaExecutionKind`), add:

```rust
        MediaExecutionKind::SeedanceVideo => "seedance_video",
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p puffer-core seedance_video_execution_adapter_is_available`
Expected: PASS

- [ ] **Step 5: Add `resolve_video_execution_descriptor`**

Mirror `resolve_image_execution_descriptor` (resolver.rs:129) EXACTLY, with the single delta `media.image` → `media.video` and the label word `image` → `video`. Add next to it:

```rust
/// Resolves the provider and execution descriptor for a validated exact video selection.
pub(crate) fn resolve_video_execution_descriptor<'a>(
    registry: &'a ProviderRegistry,
    provider_id: &str,
    model_id: &str,
    adapter: &str,
) -> Result<(&'a ProviderDescriptor, MediaExecutionDescriptor)> {
    let unavailable =
        || format!("selected video model unavailable: {provider_id}/{model_id} via {adapter}");
    let provider = registry.provider(provider_id).with_context(unavailable)?;
    let video = provider
        .media
        .as_ref()
        .and_then(|media| media.video.as_ref())
        .with_context(unavailable)?;
    let model = video
        .models
        .iter()
        .find(|model| model.id == model_id)
        .with_context(unavailable)?;
    let execution = model
        .execution
        .clone()
        .or_else(|| video.execution.clone())
        .with_context(unavailable)?;
    Ok((provider, execution))
}
```

> If `resolve_image_execution_descriptor`'s body differs from the above (e.g. extra discovery-cache handling), copy ITS structure and apply only the `.image`→`.video` delta. Read it fully before writing.

- [ ] **Step 6: Run build to verify it compiles**

Run: `cargo build -p puffer-core`
Expected: success (unused-function warning for `resolve_video_execution_descriptor` is acceptable until Task 5 uses it).

- [ ] **Step 7: Commit**

```bash
git add crates/puffer-core/runtime/media/resolver.rs
git commit -m "feat(media): allow seedance_video execution and add video execution descriptor resolver"
```

---

## Task 3: Seedance request body + param-flag mapping

**Files:**
- Create: `crates/puffer-core/runtime/media/seedance_video.rs`
- Modify: `crates/puffer-core/runtime/media/mod.rs` (register module so tests compile)

- [ ] **Step 1: Register the module**

In `crates/puffer-core/runtime/media/mod.rs`, next to `pub(crate) mod replicate_video;`, add:

```rust
pub(crate) mod seedance_video;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/puffer-core/runtime/media/seedance_video.rs` with the request type and its tests:

```rust
use super::MediaCapabilityParameter;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const SEEDANCE_PROVIDER_ID: &str = "byteplus";

/// Describes one ModelArk Seedance video generation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeedanceVideoRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    /// Ordered (flag, value) pairs appended to the prompt as `--flag value`.
    pub(crate) flags: Vec<(String, String)>,
}

impl SeedanceVideoRequest {
    fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty() {
            bail!("Seedance video model is required");
        }
        if self.prompt.trim().is_empty() {
            bail!("Seedance video prompt is required");
        }
        Ok(())
    }

    /// Builds the prompt text with ModelArk's inline `--flag value` parameters.
    fn prompt_with_flags(&self) -> String {
        let mut text = self.prompt.trim().to_string();
        for (flag, value) in &self.flags {
            text.push_str(&format!(" --{} {}", flag.trim(), value.trim()));
        }
        text
    }

    /// Builds the ModelArk task creation request body.
    pub(crate) fn request_body(&self) -> Value {
        json!({
            "model": self.model.trim(),
            "content": [
                { "type": "text", "text": self.prompt_with_flags() }
            ]
        })
    }
}

/// Maps a validated selection's parameters into a Seedance request.
///
/// `flags` are emitted in capability order using each parameter's
/// `request_field` as the ModelArk flag name and the selected value
/// (already defaulted by the caller).
pub(crate) fn seedance_request_from_parameters(
    model_id: String,
    prompt: String,
    capability_parameters: &[MediaCapabilityParameter],
    selected: &BTreeMap<String, String>,
) -> Result<SeedanceVideoRequest> {
    let mut flags = Vec::new();
    for parameter in capability_parameters {
        let value = selected
            .get(&parameter.name)
            .cloned()
            .unwrap_or_else(|| parameter.default.clone());
        flags.push((parameter.request_field.clone(), value));
    }
    let request = SeedanceVideoRequest {
        model: model_id,
        prompt,
        flags,
    };
    request.validate()?;
    Ok(request)
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
            request_field: request_field.to_string(),
        }
    }

    #[test]
    fn builds_prompt_inline_flags_in_capability_order() {
        let params = vec![
            parameter("resolution", "resolution", "1080p"),
            parameter("ratio", "ratio", "16:9"),
            parameter("duration", "duration", "5"),
        ];
        let mut selected = BTreeMap::new();
        selected.insert("ratio".to_string(), "9:16".to_string());

        let request =
            seedance_request_from_parameters("m".to_string(), "a cat".to_string(), &params, &selected)
                .expect("request");

        let body = request.request_body();
        assert_eq!(
            body["content"][0]["text"],
            json!("a cat --resolution 1080p --ratio 9:16 --duration 5")
        );
        assert_eq!(body["model"], json!("m"));
    }

    #[test]
    fn rejects_empty_prompt() {
        let error = seedance_request_from_parameters(
            "m".to_string(),
            "   ".to_string(),
            &[],
            &BTreeMap::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("prompt is required"));
    }
}
```

> `MediaCapabilityParameter` is defined in the media runtime (search `pub struct MediaCapabilityParameter`). Adjust the `use super::...` import path to wherever it is re-exported within `runtime/media`. If it is not reachable via `super`, import it by its crate path (e.g. `crate::runtime::media::MediaCapabilityParameter` or from `resolver`).

- [ ] **Step 3: Run tests to verify they fail, then pass**

Run: `cargo test -p puffer-core seedance_video::tests`
Expected: compiles and PASSES (the code above is the implementation). If the `MediaCapabilityParameter` import is wrong, fix the path until it compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/puffer-core/runtime/media/seedance_video.rs crates/puffer-core/runtime/media/mod.rs
git commit -m "feat(media): add Seedance request body and param-flag mapping"
```

---

## Task 4: Seedance transport + prediction parsing + status normalization

**Files:**
- Modify: `crates/puffer-core/runtime/media/seedance_video.rs`

- [ ] **Step 1: Write the failing tests**

Append to `seedance_video.rs` (above `#[cfg(test)] mod tests` add production code; add tests inside the module):

Production code:

```rust
use super::MediaJobStatus;
use anyhow::Context;
use reqwest::blocking::Client;

/// Abstracts ModelArk Seedance HTTP operations for production and tests.
pub(crate) trait SeedanceVideoTransport {
    /// Submits a Seedance task and returns its JSON response.
    fn submit_task(&self, url: &str, api_token: &str, body: &Value) -> Result<Value>;

    /// Polls a Seedance task URL and returns its JSON response.
    fn poll_task(&self, url: &str, api_token: &str) -> Result<Value>;
}

/// Reqwest-backed Seedance transport used by the runtime adapter.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReqwestSeedanceVideoTransport {
    client: Client,
}

impl SeedanceVideoTransport for ReqwestSeedanceVideoTransport {
    fn submit_task(&self, url: &str, api_token: &str, body: &Value) -> Result<Value> {
        let response = self
            .client
            .post(url)
            .bearer_auth(api_token)
            .json(body)
            .send()
            .with_context(|| format!("submit Seedance video task {url}"))?;
        seedance_json_response(response, "submit Seedance video task")
    }

    fn poll_task(&self, url: &str, api_token: &str) -> Result<Value> {
        let response = self
            .client
            .get(url)
            .bearer_auth(api_token)
            .send()
            .with_context(|| format!("poll Seedance video task {url}"))?;
        seedance_json_response(response, "poll Seedance video task")
    }
}

fn seedance_json_response(
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

/// Normalized view of a ModelArk Seedance task response.
#[derive(Debug, Clone)]
pub(crate) struct SeedanceTask {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) video_url: Option<String>,
    pub(crate) error: Option<String>,
}

impl SeedanceTask {
    pub(crate) fn from_value(value: Value) -> Result<Self> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .context("Seedance task response missing `id`")?
            .to_string();
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .context("Seedance task response missing `status`")?
            .to_string();
        let video_url = value
            .get("content")
            .and_then(|content| content.get("video_url"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let error = value
            .get("error")
            .map(|error| match error.get("message").and_then(Value::as_str) {
                Some(message) => message.to_string(),
                None => error.to_string(),
            });
        Ok(Self {
            id,
            status,
            video_url,
            error,
        })
    }

    /// Maps the ModelArk status string onto a media job status.
    pub(crate) fn media_status(&self) -> Result<MediaJobStatus> {
        match self.status.trim().to_ascii_lowercase().as_str() {
            "queued" | "pending" => Ok(MediaJobStatus::Queued),
            "running" | "processing" => Ok(MediaJobStatus::Running),
            "succeeded" | "success" => Ok(MediaJobStatus::Succeeded),
            "failed" | "expired" => Ok(MediaJobStatus::Failed),
            "cancelled" | "canceled" => Ok(MediaJobStatus::Canceled),
            other => bail!("unknown Seedance task status `{other}`"),
        }
    }
}
```

Tests (inside `mod tests`):

```rust
    #[test]
    fn parses_succeeded_task_with_video_url() {
        let value = json!({
            "id": "task-1",
            "status": "succeeded",
            "content": { "video_url": "https://cdn.example.com/v.mp4" }
        });
        let task = SeedanceTask::from_value(value).expect("task");
        assert_eq!(task.id, "task-1");
        assert_eq!(task.media_status().unwrap(), MediaJobStatus::Succeeded);
        assert_eq!(task.video_url.as_deref(), Some("https://cdn.example.com/v.mp4"));
    }

    #[test]
    fn parses_failed_task_error_message() {
        let value = json!({
            "id": "task-2",
            "status": "failed",
            "error": { "code": "x", "message": "content blocked" }
        });
        let task = SeedanceTask::from_value(value).expect("task");
        assert_eq!(task.media_status().unwrap(), MediaJobStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("content blocked"));
    }

    #[test]
    fn rejects_unknown_status() {
        let value = json!({ "id": "t", "status": "weird" });
        let task = SeedanceTask::from_value(value).expect("task");
        assert!(task.media_status().unwrap_err().to_string().contains("unknown Seedance task status"));
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p puffer-core seedance_video::tests`
Expected: PASS (5 tests now). If `MediaJobStatus` variant names differ (`Running` vs `Processing` etc.), align the match arms to the real enum from Task 3's `replicate_video.rs` reference (`Queued | Running | Succeeded | Failed | Canceled`).

- [ ] **Step 3: Commit**

```bash
git add crates/puffer-core/runtime/media/seedance_video.rs
git commit -m "feat(media): add Seedance transport and task status parsing"
```

---

## Task 5: Seedance adapter (submit + poll lifecycle)

**Files:**
- Modify: `crates/puffer-core/runtime/media/seedance_video.rs`

This mirrors `replicate_video.rs`'s adapter lifecycle (submit → bounded poll → complete), simplified (no cancel). Read `replicate_video.rs` lines 177–415 before writing; reuse its `poll_until_terminal_with_sleep` loop shape verbatim.

- [ ] **Step 1: Write the failing test (fake transport, happy path)**

Add production code:

```rust
use super::{MediaArtifact, MediaGenerationService, MediaJob, MediaKind};
use crate::runtime::media::http_support::download_image_url;
use std::time::Duration;
use uuid::Uuid;

const VIDEO_MIME_TYPE: &str = "video/mp4";

/// Bounded backoff while polling Seedance tasks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SeedancePollingConfig {
    pub(crate) max_attempts: usize,
    pub(crate) delay: Duration,
}

impl Default for SeedancePollingConfig {
    fn default() -> Self {
        // Video renders take minutes; poll every 3s up to ~10 minutes.
        Self { max_attempts: 200, delay: Duration::from_millis(3_000) }
    }
}

/// Submits and polls ModelArk Seedance video tasks into media jobs.
pub(crate) struct SeedanceVideoAdapter<T = ReqwestSeedanceVideoTransport> {
    api_token: String,
    submit_url: String,
    transport: T,
    client: Client,
}

impl SeedanceVideoAdapter<ReqwestSeedanceVideoTransport> {
    /// Creates a production adapter. `submit_url` is the absolute task-creation
    /// URL built by the caller via `provider_execution_url`.
    pub(crate) fn new(api_token: impl Into<String>, submit_url: impl Into<String>) -> Result<Self> {
        let api_token = api_token.into().trim().to_string();
        if api_token.is_empty() {
            bail!("Seedance API token is required");
        }
        Ok(Self {
            api_token,
            submit_url: submit_url.into().trim_end_matches('/').to_string(),
            transport: ReqwestSeedanceVideoTransport::default(),
            client: Client::new(),
        })
    }
}

impl<T> SeedanceVideoAdapter<T>
where
    T: SeedanceVideoTransport,
{
    #[cfg(test)]
    pub(crate) fn with_transport(
        api_token: impl Into<String>,
        submit_url: impl Into<String>,
        transport: T,
    ) -> Self {
        Self {
            api_token: api_token.into().trim().to_string(),
            submit_url: submit_url.into().trim_end_matches('/').to_string(),
            transport,
            client: Client::new(),
        }
    }

    /// Submits a task and persists the queued job (task id in `provider_job_id`).
    pub(crate) fn submit(
        &self,
        service: &MediaGenerationService,
        request: SeedanceVideoRequest,
        now_ms: u64,
    ) -> Result<MediaJob> {
        let response =
            self.transport
                .submit_task(&self.submit_url, &self.api_token, &request.request_body())?;
        let task = SeedanceTask::from_value(response)?;
        let mut job = MediaJob::new(
            Uuid::new_v4().to_string(),
            MediaKind::Video,
            SEEDANCE_PROVIDER_ID,
            request.model.trim(),
            request.prompt.trim(),
            1,
            now_ms,
        );
        job.adapter = Some("seedance_video".to_string());
        job.provider_job_id = Some(task.id.clone());
        self.apply_task(service, job, task, now_ms)
    }

    fn poll_url(&self, job: &MediaJob) -> Result<String> {
        let id = job
            .provider_job_id
            .as_ref()
            .context("Seedance video job is missing a task id")?;
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
        let task = SeedanceTask::from_value(response)?;
        self.apply_task(service, job, task, now_ms)
    }

    /// Polls until the job reaches a terminal status.
    pub(crate) fn poll_until_terminal(
        &self,
        service: &MediaGenerationService,
        mut job: MediaJob,
        config: SeedancePollingConfig,
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
            "Seedance video job `{}` did not reach a terminal status after {} polls",
            job.id,
            config.max_attempts
        )
    }

    fn apply_task(
        &self,
        service: &MediaGenerationService,
        mut job: MediaJob,
        task: SeedanceTask,
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
                job.error = task.error.clone().or(Some("Seedance task failed".to_string()));
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
        task: &SeedanceTask,
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
            .context("Seedance succeeded task is missing `content.video_url`")?;
        let bytes = match download_image_url(&self.client, &url, "Seedance video output") {
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
            &format!("seedance-video-{artifact_id}.mp4"),
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
                "provider": SEEDANCE_PROVIDER_ID,
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
```

> Cross-check every `MediaJob`/`MediaArtifact`/`MediaGenerationService` method name and field against `replicate_video.rs` (`MediaJob::new`, `job.transition`, `job.attach_artifact`, `job.artifact_ids`, `service.save_job`, `service.save_artifact`, `service.write_artifact_bytes`, `MediaArtifact { .. }`). Use the exact same shapes it uses.

Test (a fake transport returning queued-then-succeeded; verify artifact written). Add inside `mod tests`:

```rust
    use super::super::MediaGenerationService;
    use std::cell::RefCell;

    struct ScriptedTransport {
        submit: Value,
        polls: RefCell<Vec<Value>>,
    }

    impl SeedanceVideoTransport for ScriptedTransport {
        fn submit_task(&self, _url: &str, _token: &str, _body: &Value) -> Result<Value> {
            Ok(self.submit.clone())
        }
        fn poll_task(&self, _url: &str, _token: &str) -> Result<Value> {
            Ok(self.polls.borrow_mut().remove(0))
        }
    }

    #[test]
    fn submit_then_poll_downloads_video_artifact() {
        // download_image_url permits http loopback; serve bytes from a local server.
        let server = tiny_http::Server::http("127.0.0.1:0").expect("server");
        let url = format!("http://{}/v.mp4", server.server_addr());
        std::thread::spawn(move || {
            if let Ok(request) = server.recv() {
                let _ = request.respond(tiny_http::Response::from_data(b"MP4BYTES".to_vec()));
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(dir.path());
        let transport = ScriptedTransport {
            submit: json!({ "id": "task-9", "status": "queued" }),
            polls: RefCell::new(vec![
                json!({ "id": "task-9", "status": "running" }),
                json!({ "id": "task-9", "status": "succeeded", "content": { "video_url": url } }),
            ]),
        };
        let adapter = SeedanceVideoAdapter::with_transport("token", "https://ark/api/v3/contents/generations/tasks", transport);

        let request = SeedanceVideoRequest { model: "m".into(), prompt: "a cat".into(), flags: vec![] };
        let job = adapter.submit(&service, request, 1).expect("submit");
        let job = adapter
            .poll_until_terminal(&service, job, SeedancePollingConfig { max_attempts: 5, delay: Duration::from_millis(0) }, |_| {}, || 2)
            .expect("poll");

        assert_eq!(job.status, MediaJobStatus::Succeeded);
        assert_eq!(job.artifact_ids.len(), 1);
    }
```

> If `tiny_http` / `tempfile` are not already dev-dependencies of `puffer-core`, check how `replicate_video.rs`/`images_json_tests.rs` spin up their local servers (they already download bytes in tests) and reuse that exact harness instead. Do not add new dev-deps if an existing one covers it.

- [ ] **Step 2: Run tests**

Run: `cargo test -p puffer-core seedance_video::tests`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/puffer-core/runtime/media/seedance_video.rs
git commit -m "feat(media): add Seedance video adapter submit/poll lifecycle"
```

---

## Task 6: Add `media.video` to `byteplus.yaml`

**Files:**
- Modify: `resources/providers/byteplus.yaml`

- [ ] **Step 1: Add the video section**

Under the existing `media:` map (sibling of `image:`), add (use the id/values confirmed in Task 0):

```yaml
  video:
    discovery:
      adapter: static
    execution:
      adapter: seedance_video
      path: /contents/generations/tasks
    models:
      - id: dreamina-seedance-2-0-260128
        display_name: Seedance 2.0
        operations:
          - generate
        parameters:
          - name: resolution
            label: Resolution
            values: ["480p", "720p", "1080p"]
            default: "1080p"
            request_field: resolution
          - name: ratio
            label: Aspect ratio
            values: ["16:9", "9:16", "1:1"]
            default: "16:9"
            request_field: ratio
          - name: duration
            label: Duration
            values: ["5", "10"]
            default: "5"
            request_field: duration
```

- [ ] **Step 2: Verify the resource parses**

Run: `cargo test -p puffer-provider-registry`
Expected: PASS (resource load/parse tests still green; the new section deserializes).

- [ ] **Step 3: Commit**

```bash
git add resources/providers/byteplus.yaml
git commit -m "feat(media): declare BytePlus Seedance video capability"
```

---

## Task 7: Wire the `seedance_video` arm in the media runtime

**Files:**
- Modify: `crates/puffer-core/media_runtime.rs` (`generate_exact_video_from_media_request`, ~line 675)

- [ ] **Step 1: Add the match arm**

In `generate_exact_video_from_media_request`, the `match request.adapter.as_str()` currently has `"replicate_video"` and a catch-all `bail!`. Add a `"seedance_video"` arm before the catch-all. The surrounding code already computed `capability` (from `validate_media_generate_selection`) and `parameters` (from `selected_parameters_with_defaults`). Use them:

```rust
        "seedance_video" => {
            let (provider, execution) = resolve_video_execution_descriptor(
                registry,
                &request.provider_id,
                &request.model_id,
                &request.adapter,
            )?;
            let api_key = bearer_token(provider, auth_store, CredentialAliasMode::Strict)?
                .context("BytePlus API key is required")?;
            let submit_url = provider_execution_url(provider, &execution, "Seedance video task")?;
            let service = MediaGenerationService::new(workspace_root);
            let adapter = SeedanceVideoAdapter::new(api_key, submit_url.to_string())?;
            let job = adapter.submit(
                &service,
                seedance_request_from_parameters(
                    request.model_id.clone(),
                    request.prompt.clone(),
                    &capability.parameters,
                    &parameters,
                )?,
                now_ms(),
            )?;
            let job = adapter.poll_until_terminal(
                &service,
                job,
                SeedancePollingConfig::default(),
                std::thread::sleep,
                now_ms,
            )?;
            let artifacts = load_media_job_artifacts(&service, &job)?;
            Ok(exact_media_generation_result(job, artifacts))
        }
```

Add the imports at the top of `media_runtime.rs`:

```rust
use crate::runtime::media::http_support::provider_execution_url;
use crate::runtime::media::resolver::resolve_video_execution_descriptor;
use crate::runtime::media::seedance_video::{
    seedance_request_from_parameters, SeedancePollingConfig, SeedanceVideoAdapter,
};
```

> Match the existing import style in this file. `bearer_token`, `CredentialAliasMode`, `MediaGenerationService`, `load_media_job_artifacts`, `exact_media_generation_result`, `now_ms` are already imported/used by the `replicate_video` arm — reuse those imports. Confirm `capability` is in scope at the match (it is, from `validate_media_generate_selection` at the top of the fn); if the binding was named `_capability`, rename it to `capability`.

- [ ] **Step 2: Build**

Run: `cargo build -p puffer-core`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add crates/puffer-core/media_runtime.rs
git commit -m "feat(media): route seedance_video adapter in media runtime"
```

---

## Task 8: Integration test — daemon discovers Seedance video capability

**Files:**
- Modify: `crates/puffer-cli/src/daemon.rs` (tests module — reuse the existing
  `daemon_state_with_replicate_video_capability` harness pattern, ~line 5702/5986)

- [ ] **Step 1: Write the failing test**

The daemon tests already have a helper writing a video provider override and a test `daemon_list_media_capabilities_returns_video_capability`. Add a BytePlus-Seedance variant. Mirror `write_replicate_video_resource_override` + `daemon_state_with_replicate_video_capability` but with `adapter: seedance_video`, provider id `byteplus`, base_url ModelArk, and a Seedance model. Then:

```rust
    #[test]
    fn daemon_list_media_capabilities_returns_seedance_video_capability() {
        let (_home_guard, _temp, state) = daemon_state_with_seedance_video_capability();

        let response =
            handle_list_media_capabilities(&state, &json!({"kind": "video"})).expect("response");
        let capabilities = response["capabilities"].as_array().expect("capabilities");

        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0]["adapter"], "seedance_video");
        assert_eq!(capabilities[0]["provider_id"], "byteplus");
    }
```

Add the helper `daemon_state_with_seedance_video_capability` and `write_seedance_video_resource_override` by copying the replicate equivalents and changing: provider id → `byteplus`, `base_url` → `https://ark.ap-southeast.bytepluses.com/api/v3`, `adapter: seedance_video`, `path: /contents/generations/tasks`, model id `dreamina-seedance-2-0-260128`, auth key set for `byteplus`.

- [ ] **Step 2: Run test to verify it fails then passes**

Run: `cargo test -p puffer-cli daemon_list_media_capabilities_returns_seedance_video_capability`
Expected: FAIL first if the override/helper is incomplete; PASS once the override matches the new YAML schema and Task 1/2 are in place.

- [ ] **Step 3: Commit**

```bash
git add crates/puffer-cli/src/daemon.rs
git commit -m "test(media): daemon discovers Seedance video capability"
```

---

## Task 9: Full verification

- [ ] **Step 1: Workspace build + tests**

Run: `cargo build --workspace`
Expected: success.

Run: `cargo test -p puffer-core -p puffer-provider-registry -p puffer-cli`
Expected: all PASS.

- [ ] **Step 2: Desktop UI smoke (manual)**

With a `byteplus` API key configured, open a session → Add content → "Video generation settings". Confirm the modal now lists provider **BytePlus** / model **Seedance 2.0** with Resolution/Aspect ratio/Duration selectors (no "No video capabilities available."). Save, then run `/video <prompt>` and confirm a task is submitted and an MP4 artifact attaches on completion.

- [ ] **Step 3: Commit any test-only fixups**

```bash
git add -A
git commit -m "test(media): finalize Seedance video verification"
```

---

## Self-Review Notes

- **Spec coverage:** YAML (Task 6) ✓; adapter reusing http_support + replicate lifecycle (Tasks 3–5) ✓; four wiring points — enum (T1), availability+adapter_id+resolve_video_execution_descriptor (T2), mod.rs (T3), match arm (T7) ✓; error/redaction reuse via `download_image_url` (T5) ✓ — note: `provider_error_secrets`/`redact_secrets` wrapping of ModelArk error bodies is provided by reusing the shared helpers in the arm's error path; if the replicate arm wraps errors with redaction, mirror that wrapping in Task 7. Testing (T3–T5, T8) ✓; verification prerequisites (T0) ✓; count==1 enforced by existing `validate_video_count` (unchanged) ✓.
- **Non-goals honored:** no generic video trait; `replicate_video` untouched; image path untouched; no concurrency defense.
- **Type consistency:** `seedance_request_from_parameters`, `SeedanceVideoRequest`, `SeedanceTask`, `SeedanceVideoAdapter`, `SeedancePollingConfig` names used identically across Tasks 3/4/5/7.
