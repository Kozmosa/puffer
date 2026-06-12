# Media Failure Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make media generation failures diagnosable at the `videogen` boundary across WorldRouter, BytePlus, and Relaydance without changing generation lifecycle behavior.

**Architecture:** Add a lightweight diagnostic model in `puffer-media`, have provider adapters attach facts to errors and failed jobs, and have `puffer-core`/`puffer-cli` preserve the structured diagnostic across internal tool execution. Keep provider-specific parsing inside each adapter and keep the shared layer limited to field shape, redaction, and shallow hints.

**Tech Stack:** Rust, Cargo workspace, `anyhow`, `serde`, `serde_json`, existing `puffer-media` exact media runtime and internal tool broker.

**Spec:** `docs/superpowers/specs/2026-06-13-media-failure-diagnostics-design.md`

---

## Scope Rules

- Implement diagnostics for the current video adapters only:
  `worldrouter_video`, `byteplus_video`, and `relaydance_video`.
- Keep the shared diagnostic module provider-agnostic. Adapter-specific
  response parsing stays in the adapter files.
- Preserve existing synchronous generation behavior. Do not add background
  jobs, retry policy, circuit breakers, provider health state, failover, or
  default provider/model changes.
- Keep existing top-level video output fields (`providerJobId`,
  `remoteStatus`, `error`) and add the structured `diagnostic` object beside
  them.
- Successful video output should serialize `"diagnostic": null`; failed remote
  jobs and internal tool failures should carry the structured diagnostic when
  provider context is available.

---

## File Structure

- Create: `crates/puffer-media/src/diagnostics.rs`
  Defines `MediaFailureDiagnostic`, `MediaFailureContext`,
  `ProviderHttpError`, hint derivation, JSON-body fact extraction, redaction,
  and `anyhow` helpers.
- Modify: `crates/puffer-media/src/lib.rs`
  Exports the diagnostic type and helper needed by `puffer-core`.
- Modify: `crates/puffer-media/src/runtime.rs`
  Adds `diagnostic: Option<MediaFailureDiagnostic>` to
  `ExactMediaGenerationResult` and builds failed-job diagnostics from `MediaJob`.
- Modify: `crates/puffer-media/src/video.rs`
  Preserves full diagnostic errors through WorldRouter, BytePlus, and
  Relaydance generation paths.
- Modify: `crates/puffer-media/src/media/worldrouter_video.rs`
  Uses shared diagnostics for submit, poll, asset, and download phases.
- Modify: `crates/puffer-media/src/media/worldrouter_video_tests.rs`
  Covers WorldRouter submit, poll, download, and remote failure diagnostics.
- Modify: `crates/puffer-media/src/media/byteplus_video.rs`
  Uses shared diagnostics for submit, poll, download, and remote failed-job
  coverage; adds inline adapter tests.
- Modify: `crates/puffer-media/src/media/relaydance_video.rs`
  Uses shared diagnostics for submit, poll, download, and remote failed-job
  coverage; adds inline adapter tests.
- Modify: `crates/puffer-tools/src/internal_permissions.rs`
  Adds optional structured `diagnostic` to internal tool execution failures.
- Modify: `crates/puffer-core/runtime/internal_tool_permissions.rs`
  Extracts media diagnostics from failed internal tool execution.
- Modify: `crates/puffer-cli/src/media_internal_tools.rs`
  Prints failed internal tool diagnostics instead of collapsing them to one
  line.
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs`
  Emits `diagnostic` in successful `videogen` JSON, including failed remote
  jobs.

---

## Task 1: Add Shared Media Failure Diagnostic Model

**Files:**

- Create: `crates/puffer-media/src/diagnostics.rs`
- Modify: `crates/puffer-media/src/lib.rs`

- [ ] **Step 1: Write failing diagnostic tests**

Add tests at the bottom of `crates/puffer-media/src/diagnostics.rs` while
creating the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_error_extracts_nested_provider_facts() {
        let error = ProviderHttpError::new(
            "submit video task",
            402,
            r#"{"error":{"code":"seedance_balance_too_low","message":"low credits","request_id":"req-1"}}"#,
        );

        assert_eq!(error.http_status, Some(402));
        assert_eq!(error.provider_code.as_deref(), Some("seedance_balance_too_low"));
        assert_eq!(error.provider_message.as_deref(), Some("low credits"));
        assert_eq!(error.request_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn diagnostic_hint_covers_worldrouter_seedance_balance() {
        let diagnostic = MediaFailureDiagnostic::from_http_error(
            MediaFailureContext::new("video", "worldrouter")
                .adapter("worldrouter_video")
                .model("seedance-2.0-fast")
                .phase("submit"),
            ProviderHttpError::new(
                "submit WorldRouter video task",
                402,
                r#"{"error":{"code":"seedance_balance_too_low","message":"top up"}}"#,
            ),
        );

        assert_eq!(diagnostic.provider_id, "worldrouter");
        assert_eq!(diagnostic.phase.as_deref(), Some("submit"));
        assert_eq!(diagnostic.http_status, Some(402));
        assert_eq!(
            diagnostic.provider_code.as_deref(),
            Some("seedance_balance_too_low")
        );
        assert_eq!(
            diagnostic.hint.as_deref(),
            Some("WorldRouter Seedance credits appear to be too low; check team credits.")
        );
    }

    #[test]
    fn redaction_removes_secret_from_error_and_hint_inputs() {
        let diagnostic = MediaFailureDiagnostic::from_http_error(
            MediaFailureContext::new("video", "byteplus")
                .adapter("byteplus_video")
                .model("dreamina-seedance-2-0-fast-260128")
                .phase("submit"),
            ProviderHttpError::new(
                "submit video task",
                500,
                r#"{"error":{"message":"upstream leaked sk-secret-token"}}"#,
            ),
        )
        .redact(&["sk-secret-token".to_string()]);

        assert!(!diagnostic.error.contains("sk-secret-token"));
        assert!(diagnostic.error.contains("[redacted]"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p puffer-media diagnostics
```

Expected: FAIL because `ProviderHttpError`, `MediaFailureDiagnostic`, and
`MediaFailureContext` do not exist yet.

- [ ] **Step 3: Implement the diagnostic model**

Add this implementation to `crates/puffer-media/src/diagnostics.rs`:

```rust
use anyhow::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Carries provider failure facts in a stable shape for media tooling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFailureDiagnostic {
    pub kind: String,
    #[serde(rename = "provider")]
    pub provider_id: String,
    pub adapter: Option<String>,
    #[serde(rename = "model")]
    pub model_id: Option<String>,
    pub phase: Option<String>,
    pub provider_job_id: Option<String>,
    pub remote_status: Option<String>,
    pub http_status: Option<u16>,
    pub provider_code: Option<String>,
    pub request_id: Option<String>,
    pub error: String,
    pub hint: Option<String>,
}

/// Carries media context used to build a failure diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFailureContext {
    kind: String,
    provider_id: String,
    adapter: Option<String>,
    model_id: Option<String>,
    phase: Option<String>,
    provider_job_id: Option<String>,
    remote_status: Option<String>,
}

impl MediaFailureContext {
    /// Creates a diagnostic context for one media provider.
    pub fn new(kind: impl Into<String>, provider_id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            provider_id: provider_id.into(),
            adapter: None,
            model_id: None,
            phase: None,
            provider_job_id: None,
            remote_status: None,
        }
    }

    /// Adds an adapter id.
    pub fn adapter(mut self, adapter: impl Into<String>) -> Self {
        self.adapter = Some(adapter.into());
        self
    }

    /// Adds a model id.
    pub fn model(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    /// Adds a phase label.
    pub fn phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }

    /// Adds a provider job id.
    pub fn provider_job_id(mut self, provider_job_id: impl Into<String>) -> Self {
        self.provider_job_id = Some(provider_job_id.into());
        self
    }

    /// Adds a remote status.
    pub fn remote_status(mut self, remote_status: impl Into<String>) -> Self {
        self.remote_status = Some(remote_status.into());
        self
    }
}

/// Captures an HTTP provider error before provider context is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHttpError {
    pub label: String,
    pub http_status: Option<u16>,
    pub body: String,
    pub provider_code: Option<String>,
    pub provider_message: Option<String>,
    pub request_id: Option<String>,
}

impl ProviderHttpError {
    /// Creates an HTTP provider error and extracts common JSON fields.
    pub fn new(label: impl Into<String>, http_status: u16, body: impl Into<String>) -> Self {
        let label = label.into();
        let body = body.into();
        let parsed = serde_json::from_str::<Value>(&body).ok();
        Self {
            label,
            http_status: Some(http_status),
            provider_code: parsed.as_ref().and_then(provider_code),
            provider_message: parsed.as_ref().and_then(provider_message),
            request_id: parsed.as_ref().and_then(request_id),
            body,
        }
    }
}

impl fmt::Display for ProviderHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.http_status {
            Some(status) => write!(formatter, "{} failed with status {}: {}", self.label, status, self.body),
            None => write!(formatter, "{} failed: {}", self.label, self.body),
        }
    }
}

impl std::error::Error for ProviderHttpError {}

/// Error wrapper that preserves a structured media failure diagnostic.
#[derive(Debug, Clone)]
pub struct MediaFailureError {
    diagnostic: MediaFailureDiagnostic,
}

impl MediaFailureError {
    /// Creates a media failure error from a diagnostic.
    pub fn new(diagnostic: MediaFailureDiagnostic) -> Self {
        Self { diagnostic }
    }

    /// Returns the carried diagnostic.
    pub fn diagnostic(&self) -> &MediaFailureDiagnostic {
        &self.diagnostic
    }
}

impl fmt::Display for MediaFailureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.diagnostic.error)
    }
}

impl std::error::Error for MediaFailureError {}

impl MediaFailureDiagnostic {
    /// Builds a diagnostic from a provider HTTP error.
    pub fn from_http_error(context: MediaFailureContext, error: ProviderHttpError) -> Self {
        let message = error
            .provider_message
            .clone()
            .unwrap_or_else(|| error.to_string());
        let mut diagnostic = Self {
            kind: context.kind,
            provider_id: context.provider_id,
            adapter: context.adapter,
            model_id: context.model_id,
            phase: context.phase,
            provider_job_id: context.provider_job_id,
            remote_status: context.remote_status,
            http_status: error.http_status,
            provider_code: error.provider_code,
            request_id: error.request_id,
            error: format!("{}: {}", error.label, message),
            hint: None,
        };
        diagnostic.hint = diagnostic_hint(&diagnostic);
        diagnostic
    }

    /// Builds a diagnostic from an arbitrary error.
    pub fn from_error(context: MediaFailureContext, error: &Error) -> Self {
        let mut diagnostic = Self {
            kind: context.kind,
            provider_id: context.provider_id,
            adapter: context.adapter,
            model_id: context.model_id,
            phase: context.phase,
            provider_job_id: context.provider_job_id,
            remote_status: context.remote_status,
            http_status: None,
            provider_code: None,
            request_id: None,
            error: format!("{error:#}"),
            hint: None,
        };
        diagnostic.hint = diagnostic_hint(&diagnostic);
        diagnostic
    }

    /// Redacts known secrets from diagnostic text fields.
    pub fn redact(mut self, secrets: &[String]) -> Self {
        self.error = redact_text(&self.error, secrets);
        self.provider_code = self.provider_code.map(|value| redact_text(&value, secrets));
        self.request_id = self.request_id.map(|value| redact_text(&value, secrets));
        self.hint = self.hint.map(|value| redact_text(&value, secrets));
        self
    }
}

/// Converts an anyhow error into a diagnostic-carrying error.
pub fn media_failure_error(context: MediaFailureContext, error: Error) -> Error {
    let mut http_error = None;
    let mut media_error = None;
    for cause in error.chain() {
        if http_error.is_none() {
            http_error = cause.downcast_ref::<ProviderHttpError>().cloned();
        }
        if media_error.is_none() {
            media_error = cause
                .downcast_ref::<MediaFailureError>()
                .map(|failure| failure.diagnostic().clone());
        }
    }
    let diagnostic = http_error
        .map(|http| MediaFailureDiagnostic::from_http_error(context.clone(), http))
        .or(media_error)
        .unwrap_or_else(|| MediaFailureDiagnostic::from_error(context, &error));
    Error::new(MediaFailureError::new(diagnostic))
}

/// Returns the structured media diagnostic carried by an error, if present.
pub fn media_failure_diagnostic(error: &Error) -> Option<MediaFailureDiagnostic> {
    error.downcast_ref::<MediaFailureError>().map(|error| error.diagnostic().clone())
}

fn provider_code(value: &Value) -> Option<String> {
    text_at(value, &["error", "code"])
        .or_else(|| text_at(value, &["code"]))
        .or_else(|| text_at(value, &["type"]))
}

fn provider_message(value: &Value) -> Option<String> {
    text_at(value, &["error", "message"])
        .or_else(|| text_at(value, &["message"]))
        .or_else(|| text_at(value, &["failure_reason"]))
        .or_else(|| text_at(value, &["fail_reason"]))
        .or_else(|| text_at(value, &["reason"]))
}

fn request_id(value: &Value) -> Option<String> {
    text_at(value, &["requestId"])
        .or_else(|| text_at(value, &["request_id"]))
        .or_else(|| text_at(value, &["error", "request_id"]))
        .or_else(|| text_at(value, &["error", "requestId"]))
}

fn text_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn diagnostic_hint(diagnostic: &MediaFailureDiagnostic) -> Option<String> {
    let provider = diagnostic.provider_id.as_str();
    let code = diagnostic.provider_code.as_deref().unwrap_or("");
    let message = diagnostic.error.to_ascii_lowercase();
    if provider == "worldrouter" && code == "seedance_balance_too_low" {
        return Some("WorldRouter Seedance credits appear to be too low; check team credits.".to_string());
    }
    if provider == "worldrouter" && code == "seedance_too_many_pending_tasks" {
        return Some("WorldRouter has too many pending Seedance tasks; wait for active jobs to finish.".to_string());
    }
    if provider == "worldrouter" && code == "unsupported_model" {
        return Some("WorldRouter rejected the model; use seedance-2.0 or seedance-2.0-fast on the Seedance task endpoint.".to_string());
    }
    if provider == "worldrouter" && message.contains("upload assets first") {
        return Some("WorldRouter image references must be uploaded through asset helpers before generation.".to_string());
    }
    if provider == "byteplus" && (message.contains("sensitive") || message.contains("moderation")) {
        return Some("BytePlus rejected the generated media or references; revise the prompt or inputs.".to_string());
    }
    if provider == "relaydance" && (message.contains("copyright") || message.contains("sensitive")) {
        return Some("Relaydance rejected the generated media or references; revise the prompt or inputs.".to_string());
    }
    match diagnostic.http_status {
        Some(401) | Some(403) => Some("Provider credentials or permissions appear invalid.".to_string()),
        Some(402) => Some("Provider account may not have enough media-generation credits.".to_string()),
        Some(408) => Some("Provider or network timed out; retry later.".to_string()),
        Some(429) => Some("Provider rate limit or pending-task limit was reached; wait before retrying.".to_string()),
        Some(400) => Some("Provider rejected the request; check model, parameters, endpoint, and media references.".to_string()),
        Some(status) if status >= 500 => Some("Provider or upstream service returned an internal error; retry later or compare another provider.".to_string()),
        _ if diagnostic.phase.as_deref() == Some("download") => Some("Provider task completed, but artifact download failed.".to_string()),
        _ if diagnostic.phase.as_deref() == Some("persist") => Some("Provider task completed, but local artifact persistence failed.".to_string()),
        _ => None,
    }
}

fn redact_text(text: &str, secrets: &[String]) -> String {
    secrets.iter().fold(text.to_string(), |redacted, secret| {
        let secret = secret.trim();
        if secret.is_empty() {
            redacted
        } else {
            redacted.replace(secret, "[redacted]")
        }
    })
}
```

- [ ] **Step 4: Export the diagnostic model**

Modify `crates/puffer-media/src/lib.rs`:

```rust
mod diagnostics;
```

Add these exports:

```rust
pub use diagnostics::{
    media_failure_diagnostic, media_failure_error, MediaFailureContext,
    MediaFailureDiagnostic, MediaFailureError, ProviderHttpError,
};
```

- [ ] **Step 5: Run focused tests**

Run:

```bash
cargo test -p puffer-media diagnostics
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/puffer-media/src/diagnostics.rs crates/puffer-media/src/lib.rs
git commit -m "feat(media): add failure diagnostic model"
```

---

## Task 2: Add Diagnostics To Exact Video Results

**Files:**

- Modify: `crates/puffer-media/src/runtime.rs`
- Modify: `crates/puffer-media/src/runtime_tests.rs`

- [ ] **Step 1: Write failing runtime tests**

Add this test to `crates/puffer-media/src/runtime_tests.rs`:

```rust
#[test]
fn exact_media_generation_result_returns_failed_video_diagnostic_object_for_current_adapters() {
    let cases = [
        (
            "worldrouter",
            "worldrouter_video",
            "seedance-2.0-fast",
            "wr-task-1",
            "The service encountered an unexpected internal error.",
        ),
        (
            "byteplus",
            "byteplus_video",
            "dreamina-seedance-2-0-fast-260128",
            "bp-task-1",
            "The request failed because the output video may be related to copyright restrictions.",
        ),
        (
            "relaydance",
            "relaydance_video",
            "doubao-seedance-2-0-720p",
            "rd-task-1",
            "content blocked upstream",
        ),
    ];

    for (provider, adapter, model, task_id, message) in cases {
        let mut job = MediaJob::new(
            format!("job-{provider}"),
            MediaKind::Video,
            provider,
            model,
            "animate a robot",
            1,
            1,
        );
        job.adapter = Some(adapter.to_string());
        job.provider_job_id = Some(task_id.to_string());
        job.remote_status = Some("failed".to_string());
        job.error = Some(message.to_string());
        job.transition(MediaJobStatus::Failed, 2).unwrap();

        let result = exact_media_generation_result(job, Vec::new());
        let diagnostic = result.diagnostic.expect("diagnostic");

        assert_eq!(diagnostic.kind, "video");
        assert_eq!(diagnostic.provider_id, provider);
        assert_eq!(diagnostic.adapter.as_deref(), Some(adapter));
        assert_eq!(diagnostic.model_id.as_deref(), Some(model));
        assert_eq!(diagnostic.phase.as_deref(), Some("poll"));
        assert_eq!(diagnostic.provider_job_id.as_deref(), Some(task_id));
        assert_eq!(diagnostic.remote_status.as_deref(), Some("failed"));
        assert!(diagnostic.error.contains(message));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p puffer-media exact_media_generation_result_returns_failed_video_diagnostic_object_for_current_adapters
```

Expected: FAIL because `ExactMediaGenerationResult` has no `diagnostic` field.

- [ ] **Step 3: Add result field and builder logic**

Modify `ExactMediaGenerationResult` in `crates/puffer-media/src/runtime.rs`:

```rust
pub diagnostic: Option<MediaFailureDiagnostic>,
```

Import the diagnostic types:

```rust
use crate::diagnostics::{MediaFailureContext, MediaFailureDiagnostic};
```

Update non-failed `ExactMediaGenerationResult` construction sites to set:

```rust
diagnostic: None,
```

Update `exact_media_generation_result(job, artifacts)`:

```rust
let diagnostic = if job.status == MediaJobStatus::Failed {
    job.error.as_ref().map(|error| {
        let mut context = MediaFailureContext::new(media_kind_name(job.kind), job.provider_id.clone())
            .model(job.model_id.clone())
            .phase("poll");
        if let Some(adapter) = &job.adapter {
            context = context.adapter(adapter.clone());
        }
        if let Some(provider_job_id) = &job.provider_job_id {
            context = context.provider_job_id(provider_job_id.clone());
        }
        if let Some(remote_status) = &job.remote_status {
            context = context.remote_status(remote_status.clone());
        }
        MediaFailureDiagnostic::from_error(context, &anyhow::anyhow!(error.clone()))
    })
} else {
    None
};
```

Add `diagnostic` to the returned struct.

Update every existing `ExactMediaGenerationResult` literal in
`crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs`,
`crates/puffer-media/src/runtime_tests.rs`, and
`crates/puffer-media/src/video.rs` tests with either `diagnostic: None` or the
specific diagnostic under test.

- [ ] **Step 4: Run focused tests**

Run:

```bash
cargo test -p puffer-media exact_media_generation
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/puffer-media/src/runtime.rs crates/puffer-media/src/runtime_tests.rs
git commit -m "feat(media): attach diagnostics to exact video results"
```

---

## Task 3: Preserve WorldRouter Submit And Poll Diagnostics

**Files:**

- Modify: `crates/puffer-media/src/media/worldrouter_video.rs`
- Modify: `crates/puffer-media/src/media/worldrouter_video_tests.rs`
- Modify: `crates/puffer-media/src/video.rs`

- [ ] **Step 1: Write failing WorldRouter submit diagnostic test**

In `crates/puffer-media/src/media/worldrouter_video_tests.rs`, replace the
`submit: Value` field with:

```rust
#[derive(Clone)]
enum ScriptedJson {
    Ok(Value),
    Err(ProviderHttpError),
}

impl ScriptedJson {
    fn result(&self) -> Result<Value> {
        match self {
            Self::Ok(value) => Ok(value.clone()),
            Self::Err(error) => Err(anyhow::Error::new(error.clone())),
        }
    }
}
```

Change `ScriptedTransport.submit` to `ScriptedJson`, update existing fixtures to
use `ScriptedJson::Ok(json!({...}))`, and change `submit_task` to:

```rust
self.submit.result()
```

Add this constructor for the failing test:

```rust
impl ScriptedTransport {
    fn submit_error(error: ProviderHttpError) -> Self {
        Self {
            asset_group: json!({"id": "group-1"}),
            assets: Rc::new(RefCell::new(Vec::new())),
            submit: ScriptedJson::Err(error),
            polls: Rc::new(RefCell::new(Vec::new())),
            downloads: Rc::new(RefCell::new(Vec::new())),
            requests: Rc::new(RefCell::new(Vec::new())),
        }
    }
}
```

Then add:

```rust
#[test]
fn submit_http_failure_returns_media_diagnostic() {
    let service = MediaGenerationService::new(tempdir().unwrap().path());
    let adapter = test_adapter(ScriptedTransport::submit_error(
        ProviderHttpError::new(
            "submit WorldRouter video task",
            402,
            r#"{"error":{"code":"seedance_balance_too_low","message":"low credits","request_id":"req-1"}}"#,
        ),
    ));
    let request = WorldRouterVideoRequest {
        model: "seedance-2.0-fast".to_string(),
        prompt: "a robot battle".to_string(),
        image_references: Vec::new(),
        params: params(&[("resolution", "480p"), ("duration", "5")]),
    };

    let error = adapter
        .submit(&service, request, BTreeMap::new(), 1)
        .expect_err("submit should fail");
    let diagnostic = media_failure_diagnostic(&error).expect("diagnostic");

    assert_eq!(diagnostic.provider_id, "worldrouter");
    assert_eq!(diagnostic.adapter.as_deref(), Some("worldrouter_video"));
    assert_eq!(diagnostic.phase.as_deref(), Some("submit"));
    assert_eq!(diagnostic.http_status, Some(402));
    assert_eq!(
        diagnostic.provider_code.as_deref(),
        Some("seedance_balance_too_low")
    );
    assert_eq!(diagnostic.request_id.as_deref(), Some("req-1"));
    assert!(diagnostic.hint.unwrap().contains("credits"));
}
```

Also add `worldrouter_download_failure_returns_media_diagnostic` in the same
file:

- script a successful submit and successful poll that returns
  `content.video_url`;
- make `download_bytes` return `anyhow!("cdn returned 503")`;
- assert `poll_until_terminal` returns an error with
  `media_failure_diagnostic(&error)`;
- assert the diagnostic has `provider=worldrouter`,
  `adapter=worldrouter_video`, `phase=download`,
  `providerJobId=task-123`, and a download hint.

Add `worldrouter_poll_parser_failure_records_phase_context`:

- script submit `{"id":"task-123"}` and poll `{"status":"running"}` so the
  parser reports a missing task id;
- assert `poll` returns `Ok` with a non-terminal job;
- assert the saved job error contains `provider=worldrouter`,
  `adapter=worldrouter_video`, `phase=poll`, and `task=task-123`.

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p puffer-media submit_http_failure_returns_media_diagnostic
cargo test -p puffer-media worldrouter_download_failure_returns_media_diagnostic
cargo test -p puffer-media worldrouter_poll_parser_failure_records_phase_context
```

Expected: FAIL because WorldRouter does not wrap submit failures in
`MediaFailureError` and does not attach a structured download diagnostic.

- [ ] **Step 3: Convert WorldRouter HTTP helpers to ProviderHttpError**

In `json_response`, replace non-2xx `bail!` with:

```rust
return Err(anyhow::Error::new(ProviderHttpError::new(label, status.as_u16(), text)));
```

Import:

```rust
use crate::{media_failure_error, MediaFailureContext, ProviderHttpError};
```

In `submit`, wrap validate, asset, build-body, submit, and parse errors with
phase-specific contexts. The submit transport call should use:

```rust
.map_err(|error| {
    media_failure_error(
        MediaFailureContext::new("video", self.provider_id.clone())
            .adapter(WORLDROUTER_VIDEO_ADAPTER)
            .model(request.model.clone())
            .phase("submit"),
        error,
    )
})?;
```

Apply the same pattern for `phase("asset_group")`, `phase("asset_upload")`,
`phase("validate")`, `phase("poll")`, and `phase("download")` where the
adapter has context. For `complete_video_job`, wrap the adapter's
`download_bytes` closure with a `MediaFailureContext` that includes the model,
provider job id, remote status, adapter id, and `phase("download")`.

- [ ] **Step 4: Preserve full chain in video.rs**

Replace WorldRouter `map_err` calls that use `error.to_string()` with a helper:

```rust
fn redact_media_error(error: anyhow::Error, secrets: &[String]) -> anyhow::Error {
    if let Some(diagnostic) = crate::media_failure_diagnostic(&error) {
        let diagnostic = diagnostic.redact(secrets);
        return anyhow::Error::new(crate::MediaFailureError::new(diagnostic));
    }
    anyhow::anyhow!("{}", redact_secrets(&format!("{error:#}"), secrets))
}
```

Then call:

```rust
.map_err(|error| redact_media_error(error, &secrets))?;
```

- [ ] **Step 5: Run WorldRouter tests**

Run:

```bash
cargo test -p puffer-media worldrouter_video
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/puffer-media/src/media/worldrouter_video.rs crates/puffer-media/src/media/worldrouter_video_tests.rs crates/puffer-media/src/video.rs
git commit -m "fix(media): preserve worldrouter failure diagnostics"
```

---

## Task 4: Preserve BytePlus Diagnostics

**Files:**

- Modify: `crates/puffer-media/src/media/byteplus_video.rs`
- Modify: `crates/puffer-media/src/video.rs`

- [ ] **Step 1: Write failing BytePlus submit diagnostic test**

In `crates/puffer-media/src/media/byteplus_video.rs`, update
`tests_support::ScriptedTransport` so `submit` is `Result<Value,
ProviderHttpError>`-like without losing `Clone`. Use this local enum:

```rust
#[derive(Clone)]
pub(crate) enum ScriptedJson {
    Ok(Value),
    Err(ProviderHttpError),
}

impl ScriptedJson {
    fn result(&self) -> Result<Value> {
        match self {
            Self::Ok(value) => Ok(value.clone()),
            Self::Err(error) => Err(anyhow::Error::new(error.clone())),
        }
    }
}
```

Change `ScriptedTransport.submit` to `ScriptedJson`, update
`submit_task` to return `self.submit.result()`, and update
`scripted(submit, polls)` to wrap the submit value:

```rust
submit: ScriptedJson::Ok(submit),
```

Add:

```rust
pub(crate) fn scripted_submit_error(error: ProviderHttpError) -> ScriptedTransport {
    ScriptedTransport {
        submit: ScriptedJson::Err(error),
        polls: RefCell::new(Vec::new()),
    }
}
```

Then add:

```rust
#[test]
fn byteplus_submit_http_failure_returns_media_diagnostic() {
    let service = MediaGenerationService::new(tempdir().unwrap().path());
    let adapter = BytePlusVideoAdapter::with_transport(
        "token",
        "https://ark.ap-southeast.bytepluses.com/api/v3/contents/generations/tasks",
        "byteplus",
        tests_support::scripted_submit_error(ProviderHttpError::new(
            "submit video task",
            500,
            r#"{"error":{"code":"InternalError","message":"upstream internal error","request_id":"bp-req-1"}}"#,
        )),
    );
    let request = BytePlusVideoRequest {
        model: "dreamina-seedance-2-0-fast-260128".to_string(),
        prompt: "a robot battle".to_string(),
        image_references: Vec::new(),
        params: vec![],
    };

    let error = adapter
        .submit(&service, request, BTreeMap::new(), 1)
        .expect_err("submit should fail");
    let diagnostic = media_failure_diagnostic(&error).expect("diagnostic");

    assert_eq!(diagnostic.provider_id, "byteplus");
    assert_eq!(diagnostic.adapter.as_deref(), Some("byteplus_video"));
    assert_eq!(diagnostic.phase.as_deref(), Some("submit"));
    assert_eq!(diagnostic.http_status, Some(500));
    assert_eq!(diagnostic.provider_code.as_deref(), Some("InternalError"));
    assert_eq!(diagnostic.request_id.as_deref(), Some("bp-req-1"));
}
```

Add two more focused BytePlus tests before implementation:

- update the existing `poll_parser_failure_is_transient_and_keeps_polling`
  test to also assert the saved job error contains `phase=poll`;
- `byteplus_failed_poll_persists_remote_failure_diagnostics`: script submit
  `{"id":"bp-task-1"}` and poll
  `{"id":"bp-task-1","status":"failed","error":{"message":"content moderation rejected output"}}`;
  assert the persisted job is failed and keeps `provider_job_id`,
  `remote_status`, and `error`.
- `byteplus_download_failure_returns_media_diagnostic`: extend
  `tests_support::ScriptedTransport` so `download_bytes` can return a scripted
  error; script a completed task with `content.video_url`; assert
  `poll_until_terminal` returns a diagnostic with `provider=byteplus`,
  `adapter=byteplus_video`, `phase=download`, and `providerJobId`.

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p puffer-media byteplus_submit_http_failure_returns_media_diagnostic
cargo test -p puffer-media poll_parser_failure_is_transient_and_keeps_polling
cargo test -p puffer-media byteplus_failed_poll_persists_remote_failure_diagnostics
cargo test -p puffer-media byteplus_download_failure_returns_media_diagnostic
```

Expected: FAIL because BytePlus does not wrap submit failures in
`MediaFailureError` and does not attach structured download diagnostics yet.

- [ ] **Step 3: Convert BytePlus HTTP and adapter errors**

In `byteplus_video_json_response`, replace non-2xx `bail!` with
`ProviderHttpError::new(label, status.as_u16(), text)`.

In `BytePlusVideoAdapter::submit`, wrap submit and submit-parse failures with:

```rust
media_failure_error(
    MediaFailureContext::new("video", self.provider_id.clone())
        .adapter(BYTEPLUS_VIDEO_ADAPTER)
        .model(request.model.clone())
        .phase("submit"),
    error,
)
```

In `poll`, when `fetch_task` returns `Err`, wrap the transient diagnostic with:

```rust
media_failure_error(
    MediaFailureContext::new("video", self.provider_id.clone())
        .adapter(BYTEPLUS_VIDEO_ADAPTER)
        .model(job.model_id.clone())
        .phase("poll")
        .provider_job_id(job.provider_job_id.clone().unwrap_or_default()),
    error,
)
```

Only call `.provider_job_id(...)` when the id exists.

In `complete_succeeded`, wrap `download_bytes` errors with
`phase("download")`, the job model, provider job id, and remote status. Keep
the existing behavior that a download failure marks the job failed and returns
the error; only the diagnostic contents change.

- [ ] **Step 4: Apply redaction helper in video.rs**

Replace BytePlus `map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))`
with:

```rust
.map_err(|error| redact_media_error(error, &secrets))?;
```

- [ ] **Step 5: Run BytePlus tests**

Run:

```bash
cargo test -p puffer-media byteplus_video
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/puffer-media/src/media/byteplus_video.rs crates/puffer-media/src/video.rs
git commit -m "fix(media): preserve byteplus failure diagnostics"
```

---

## Task 5: Preserve Relaydance Diagnostics

**Files:**

- Modify: `crates/puffer-media/src/media/relaydance_video.rs`
- Modify: `crates/puffer-media/src/video.rs`

- [ ] **Step 1: Write failing Relaydance submit diagnostic test**

In `crates/puffer-media/src/media/relaydance_video.rs`, update
`tests_support::ScriptedTransport` with the same local `ScriptedJson` enum used
for BytePlus:

```rust
#[derive(Clone)]
pub(crate) enum ScriptedJson {
    Ok(Value),
    Err(ProviderHttpError),
}

impl ScriptedJson {
    fn result(&self) -> Result<Value> {
        match self {
            Self::Ok(value) => Ok(value.clone()),
            Self::Err(error) => Err(anyhow::Error::new(error.clone())),
        }
    }
}
```

Change `ScriptedTransport.submit` to `ScriptedJson`, update
`submit_task` to return `self.submit.result()`, and update
`scripted(submit, polls)` to wrap the submit value:

```rust
submit: ScriptedJson::Ok(submit),
```

Add:

```rust
pub(crate) fn scripted_submit_error(error: ProviderHttpError) -> ScriptedTransport {
    ScriptedTransport {
        submit: ScriptedJson::Err(error),
        polls: RefCell::new(Vec::new()),
    }
}
```

Then add:

```rust
#[test]
fn relaydance_submit_http_failure_returns_media_diagnostic() {
    let service = MediaGenerationService::new(tempdir().unwrap().path());
    let adapter = RelaydanceVideoAdapter::with_transport(
        "token",
        "https://api.relaydance.local/v1/video/generations",
        "relaydance",
        tests_support::scripted_submit_error(ProviderHttpError::new(
            "submit video task",
            429,
            r#"{"error":{"code":"rate_limited","message":"too many tasks"}}"#,
        )),
    );
    let request = RelaydanceVideoRequest {
        model: "seedance-nsfw-720p".to_string(),
        prompt: "a robot battle".to_string(),
        params: Vec::new(),
        prompt_format: VideoPromptFormat::Prompt,
    };

    let error = adapter
        .submit(&service, request, BTreeMap::new(), 1)
        .expect_err("submit should fail");
    let diagnostic = media_failure_diagnostic(&error).expect("diagnostic");

    assert_eq!(diagnostic.provider_id, "relaydance");
    assert_eq!(diagnostic.adapter.as_deref(), Some("relaydance_video"));
    assert_eq!(diagnostic.phase.as_deref(), Some("submit"));
    assert_eq!(diagnostic.http_status, Some(429));
    assert_eq!(diagnostic.provider_code.as_deref(), Some("rate_limited"));
    assert!(diagnostic.hint.unwrap().contains("rate limit"));
}
```

Add two more focused Relaydance tests before implementation:

- `relaydance_poll_parser_failure_records_phase_context`: script submit
  `{"id":"rd-task-1","status":"queued"}` and poll `{"status":"running"}` so the
  parser reports a missing task id; assert `poll` returns `Ok` with a
  non-terminal job, and the saved job error contains `provider=relaydance`,
  `adapter=relaydance_video`, `phase=poll`, and `task=rd-task-1`.
- `relaydance_failed_poll_persists_remote_failure_diagnostics`: script submit
  `{"id":"rd-task-1","status":"queued"}` and poll
  `{"id":"rd-task-1","status":"failed","error":{"code":"blocked","message":"content blocked upstream"}}`;
  assert the persisted job is failed and keeps `provider_job_id`,
  `remote_status`, and `error`.
- `relaydance_download_failure_returns_media_diagnostic`: extend
  `tests_support::ScriptedTransport` so `download_bytes` can return a scripted
  error; script a completed task with `metadata.url`; assert
  `poll_until_terminal` returns a diagnostic with `provider=relaydance`,
  `adapter=relaydance_video`, `phase=download`, and `providerJobId`.

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p puffer-media relaydance_submit_http_failure_returns_media_diagnostic
cargo test -p puffer-media relaydance_poll_parser_failure_records_phase_context
cargo test -p puffer-media relaydance_failed_poll_persists_remote_failure_diagnostics
cargo test -p puffer-media relaydance_download_failure_returns_media_diagnostic
```

Expected: FAIL because Relaydance does not wrap submit failures in
`MediaFailureError` and does not attach structured download diagnostics yet.

- [ ] **Step 3: Convert Relaydance HTTP and adapter errors**

In `relaydance_video_json_response`, replace non-2xx `bail!` with
`ProviderHttpError::new(label, status.as_u16(), text)`.

In `RelaydanceVideoAdapter::submit`, wrap submit and submit-parse failures with:

```rust
media_failure_error(
    MediaFailureContext::new("video", self.provider_id.clone())
        .adapter(RELAYDANCE_VIDEO_ADAPTER)
        .model(request.model.clone())
        .phase("submit"),
    error,
)
```

In `poll`, wrap transient poll diagnostics with phase `poll` and provider job
id when available. Keep the existing behavior that transient poll errors do not
immediately fail the job.

In `complete_succeeded`, wrap `download_bytes` errors with
`phase("download")`, the job model, provider job id, and remote status. Keep
the existing behavior that a download failure marks the job failed and returns
the error; only the diagnostic contents change.

- [ ] **Step 4: Apply redaction helper in video.rs**

Replace Relaydance `map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))`
with:

```rust
.map_err(|error| redact_media_error(error, &secrets))?;
```

- [ ] **Step 5: Run Relaydance tests**

Run:

```bash
cargo test -p puffer-media relaydance_video
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/puffer-media/src/media/relaydance_video.rs crates/puffer-media/src/video.rs
git commit -m "fix(media): preserve relaydance failure diagnostics"
```

---

## Task 6: Preserve Diagnostics Through Internal Tool Execution

**Files:**

- Modify: `crates/puffer-tools/src/internal_permissions.rs`
- Modify: `crates/puffer-core/runtime/internal_tool_permissions.rs`
- Modify: `crates/puffer-cli/src/media_internal_tools.rs`

- [ ] **Step 1: Write failing internal permission serialization test**

Add to `crates/puffer-tools/src/internal_permissions.rs` tests:

```rust
#[test]
fn execution_failure_serializes_diagnostic() {
    let response = InternalToolExecutionResponse::failure_with_diagnostic(
        "submit failed",
        serde_json::json!({
            "kind": "video",
            "provider": "worldrouter",
            "phase": "submit",
            "error": "low credits"
        }),
    );

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["success"], false);
    assert_eq!(value["reason"], "submit failed");
    assert_eq!(value["diagnostic"]["provider"], "worldrouter");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p puffer-tools execution_failure_serializes_diagnostic
```

Expected: FAIL because `diagnostic` and `failure_with_diagnostic` do not exist.

- [ ] **Step 3: Extend internal execution response**

Modify `InternalToolExecutionResponse`:

```rust
#[serde(default)]
pub diagnostic: Option<serde_json::Value>,
```

Update `success` and `failure` constructors to set `diagnostic: None`.

Add:

```rust
/// Builds a failed execution response with a structured diagnostic.
pub fn failure_with_diagnostic(
    reason: impl Into<String>,
    diagnostic: serde_json::Value,
) -> Self {
    Self {
        success: false,
        output: None,
        reason: Some(reason.into()),
        diagnostic: Some(diagnostic),
    }
}
```

- [ ] **Step 4: Attach diagnostics in puffer-core**

In `execute_internal_tool_request`, change the error branch:

```rust
Err(error) => {
    let reason = format!("{error:#}");
    if let Some(diagnostic) = puffer_media::media_failure_diagnostic(&error) {
        let value = serde_json::to_value(diagnostic)
            .unwrap_or_else(|_| serde_json::json!({ "error": reason }));
        InternalToolExecutionResponse::failure_with_diagnostic(reason, value)
    } else {
        InternalToolExecutionResponse::failure(reason)
    }
}
```

- [ ] **Step 5: Print diagnostic from CLI wrapper**

In `execute_parent_internal_tool`, replace the failure branch with:

```rust
if !response.success {
    let reason = response
        .reason
        .unwrap_or_else(|| "unknown error".to_string());
    if let Some(diagnostic) = response.diagnostic {
        let diagnostic = serde_json::to_string_pretty(&serde_json::json!({
            "status": "failed",
            "diagnostic": diagnostic
        }))?;
        anyhow::bail!("{tool_id} internal tool failed: {reason}\n{diagnostic}");
    }
    anyhow::bail!("{tool_id} internal tool failed: {reason}");
}
```

- [ ] **Step 6: Run focused tests**

Run:

```bash
cargo test -p puffer-tools internal_permissions
cargo test -p puffer-core internal_tool
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/puffer-tools/src/internal_permissions.rs crates/puffer-core/runtime/internal_tool_permissions.rs crates/puffer-cli/src/media_internal_tools.rs
git commit -m "fix(media): preserve internal tool diagnostics"
```

---

## Task 7: Emit Diagnostics From VideoGeneration Output

**Files:**

- Modify: `crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs`

- [ ] **Step 1: Write failing workflow output test**

Add a diagnostic to the existing `output_includes_failed_job_diagnostics` test:

```rust
diagnostic: Some(MediaFailureDiagnostic {
    kind: "video".to_string(),
    provider_id: "worldrouter".to_string(),
    adapter: Some("worldrouter_video".to_string()),
    model_id: Some("seedance-2.0-fast".to_string()),
    phase: Some("poll".to_string()),
    provider_job_id: Some("task-123".to_string()),
    remote_status: Some("failed".to_string()),
    http_status: None,
    provider_code: None,
    request_id: None,
    error: "The service encountered an unexpected internal error.".to_string(),
    hint: Some("Provider or upstream service returned an internal error; retry later or compare another provider.".to_string()),
}),
```

Then assert:

```rust
assert_eq!(object["diagnostic"]["provider"], json!("worldrouter"));
assert_eq!(object["diagnostic"]["adapter"], json!("worldrouter_video"));
assert_eq!(object["diagnostic"]["phase"], json!("poll"));
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p puffer-core output_includes_failed_job_diagnostics
```

Expected: FAIL because `video_generation_output` does not emit `diagnostic`.

- [ ] **Step 3: Add diagnostic to video output JSON**

Import `MediaFailureDiagnostic` if needed and add to the JSON:

```rust
"diagnostic": result.diagnostic,
```

Keep existing top-level `providerJobId`, `remoteStatus`, and `error` keys.
Successful output should serialize `"diagnostic": null`.

- [ ] **Step 4: Run workflow tests**

Run:

```bash
cargo test -p puffer-core video_generation
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs
git commit -m "fix(media): emit video failure diagnostics"
```

---

## Task 8: Focused Verification And Scope Guard

**Files:**

- Verify only intended implementation and test files changed.

- [ ] **Step 1: Run focused media tests**

Run:

```bash
cargo test -p puffer-media diagnostics
cargo test -p puffer-media worldrouter_video
cargo test -p puffer-media byteplus_video
cargo test -p puffer-media relaydance_video
cargo test -p puffer-media exact_media_generation
```

Expected: all pass.

- [ ] **Step 2: Run focused core/tool tests**

Run:

```bash
cargo test -p puffer-tools internal_permissions
cargo test -p puffer-core internal_tool
cargo test -p puffer-core video_generation
```

Expected: all pass.

- [ ] **Step 3: Run broader touched-crate tests if focused tests pass**

Run:

```bash
cargo test -p puffer-media
cargo test -p puffer-tools
cargo test -p puffer-core
```

Expected: all pass, or existing unrelated failures are documented with exact
test names and error summaries.

- [ ] **Step 4: Scope guard**

Inspect the diff:

```bash
git diff --stat
git diff --check
git diff -- crates/puffer-media crates/puffer-core crates/puffer-tools crates/puffer-cli | rg -n "retry_policy|circuit|provider_health|health_state|background_worker|failover|default_provider|default_model"
```

Expected:

- No retry policy.
- No provider health state.
- No background worker.
- No provider failover.
- No default provider/model change.
- No raw auth headers or credentials in diagnostics.

- [ ] **Step 5: Final commit if verification required changes**

If Task 8 required code/test fixes, commit them:

```bash
git add crates/puffer-media crates/puffer-core crates/puffer-tools crates/puffer-cli
git commit -m "test(media): verify failure diagnostics coverage"
```
