# Media Failure Diagnostics Design

Date: 2026-06-13

Status: Approved design, awaiting implementation plan

Constraints: do not optimize for backward compatibility; optimize for
long-term clarity, stability, and performance; prevent overdesign.

## Problem

Video generation failures are currently hard to diagnose at the `videogen`
boundary. A WorldRouter Seedance submit failure can surface only as:

```text
video-generation internal tool failed: provider=worldrouter adapter=worldrouter_video phase=submit
```

That message identifies the adapter phase, but it can lose the provider's
actionable cause, such as HTTP status, provider error code, response message, or
request id. A later WorldRouter task failure did persist useful diagnostics:

```json
{
  "providerJobId": "agt_56f6548fd09642da9081a760ab00",
  "remoteStatus": "failed",
  "error": "The service encountered an unexpected internal error."
}
```

The current implementation therefore has two diagnostic gaps:

- submit failures before a stable provider job exists can lose the final
  provider error body while crossing the media runtime and internal tool
  boundary;
- provider-specific details are not normalized into a shape that users, agents,
  tests, and desktop surfaces can read consistently.

BytePlus video generation succeeds through the same `videogen` entrypoint, so
the issue is not the skill, shell helper, artifact store, or video runtime as a
whole. The fix should improve failure reporting for all media providers without
redesigning the generation lifecycle.

## Goals

- Preserve the full, redacted error chain from provider adapters to `videogen`.
- Return a stable media failure diagnostic shape for image and video generation
  failures where the media runtime can identify provider context.
- Make `phase`, provider, adapter, model, HTTP status, provider code, provider
  job id, remote status, error, and hint visible when available.
- Keep `MediaJob` as the persisted source of truth for jobs that were submitted
  successfully.
- Make submit-stage failures diagnosable even when no provider job id exists.
- Keep hints small and practical: auth, credits, rate limits, invalid request,
  provider internal error, timeout, download/storage failure.
- Keep all diagnostics secret-redacted before crossing process or stdout
  boundaries.

## Non-Goals

- Do not convert video generation to background jobs.
- Do not change the synchronous `videogen` behavior.
- Do not add automatic retry, circuit breakers, or provider failover.
- Do not add provider health state or a provider health dashboard.
- Do not change default provider or model selection.
- Do not build a complete cross-provider error taxonomy.
- Do not expose API keys, auth headers, full raw request payloads, raw prompts,
  credentials, or unredacted provider bodies.
- Do not redesign desktop UI. UI changes should be limited to consuming the
  shared diagnostic shape if an existing surface already displays media errors.

## Selected Approach

Add a lightweight, provider-agnostic media failure diagnostic contract in
`puffer-media`. Provider adapters contribute facts; a shared helper derives a
small hint from those facts; `puffer-core` serializes the diagnostic into
`videogen` and `imagegen` outputs.

Rejected alternatives:

- Print `{error:#}` directly. This is useful for humans, but it is unstable for
  agents and UI because phase, HTTP status, and provider code remain embedded in
  free text.
- Add a complete provider error enum. That is too broad for the current issue
  and will grow stale as providers change their APIs.
- Add background polling or provider failover. Those improve other dimensions
  of media generation, but they do not address the narrow diagnostic boundary.

## Diagnostic Contract

Use one shared diagnostic shape:

```rust
pub struct MediaFailureDiagnostic {
    pub kind: String,
    pub provider_id: String,
    pub adapter: Option<String>,
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
```

`serde` output uses camelCase:

```json
{
  "kind": "video",
  "provider": "worldrouter",
  "adapter": "worldrouter_video",
  "model": "seedance-2.0-fast",
  "phase": "submit",
  "providerJobId": null,
  "remoteStatus": null,
  "httpStatus": 402,
  "providerCode": "seedance_balance_too_low",
  "requestId": "req-123",
  "error": "submit WorldRouter video task failed with status 402: ...",
  "hint": "Provider account may not have enough video-generation credits."
}
```

When a field is unknown, serialize it as `null` rather than omitting it. This
keeps the contract stable for tools and UI. `error` is required because a
failure diagnostic without a message is not actionable.

## Phases

Use simple string phases rather than a large enum exposed across crates:

- `validate`
- `asset_group`
- `asset_upload`
- `submit`
- `poll`
- `download`
- `persist`
- `resolve`
- `auth`
- `config`

Provider adapters should set the most specific phase they know. Shared runtime
code may set `resolve`, `auth`, or `config` before adapter dispatch.

## Provider Facts

Adapters should preserve these provider facts when available:

- HTTP status from non-2xx responses.
- Provider response body after redaction.
- Provider error code from common locations such as `error.code`, `code`, or
  `type`.
- Provider message from common locations such as `error.message`, `message`,
  `failure_reason`, `fail_reason`, or `reason`.
- Request id from common locations such as `requestId`, `request_id`, or
  `error.request_id`.
- Provider task id once submit succeeds.
- Remote task status from poll responses.

The provider-specific parser should stay local to the adapter when response
formats differ. The shared diagnostic type should not know WorldRouter,
BytePlus, or Relaydance response schemas.

## Hint Rules

Hints are deliberately shallow and stable. They are derived from status, code,
phase, and message text:

- `401` or `403`: check provider credentials or permissions.
- `402`: check provider credits or billing.
- `408`, timeout text, or connection timeout: retry later; provider/network
  timed out.
- `429`: wait for rate limits or pending tasks to clear.
- `400`: check request parameters, model id, endpoint, or media references.
- `5xx`: provider or upstream internal error; retry later or compare another
  provider.
- `download`: provider task completed but artifact download failed.
- `persist`: provider task completed but local artifact persistence failed.

Small provider-specific overlays are allowed only when they improve diagnosis
without creating a taxonomy:

- WorldRouter `seedance_balance_too_low`: check WorldRouter team credits.
- WorldRouter `seedance_too_many_pending_tasks`: wait for pending Seedance jobs.
- WorldRouter `unsupported_model`: verify `seedance-2.0` or
  `seedance-2.0-fast` and the `/api/v3/contents/generations/tasks` endpoint.
- WorldRouter `upload assets first`: upload image references through the
  WorldRouter asset helper flow.

## Data Flow

The existing flow stays synchronous:

```text
videogen
  -> internal tool execution
  -> VideoGeneration workflow
  -> puffer-media runtime
  -> provider adapter
  -> provider HTTP API
```

Successful jobs keep the current result shape and include diagnostic keys with
`null` values.

Remote terminal failures keep returning normal JSON with `status: "failed"` and
the persisted job diagnostics.

Submit-stage failures that cannot create a provider job remain tool failures,
but their stderr/output must include a structured `diagnostic` object with the
same contract. This avoids fabricating a `MediaJob` while still preserving the
provider's final error cause.

## Runtime Boundaries

`puffer-media` owns:

- the diagnostic struct;
- redacted provider error facts;
- hint derivation;
- conversion from adapter errors and failed jobs into diagnostics.

Provider adapters own:

- extracting provider-specific facts from HTTP responses and poll payloads;
- setting accurate phase labels;
- preserving provider task id and remote status when available.

`puffer-core` owns:

- serializing diagnostics into `VideoGeneration` and `ImageGeneration` workflow
  output;
- preserving the diagnostic object when returning an internal tool failure.

`puffer-cli` owns:

- printing successful tool output unchanged;
- printing failed internal tool diagnostics without replacing them with only a
  one-line reason.

## Safety

Diagnostics must pass through the existing secret redaction path before they are
stored or printed. Tests should include a synthetic bearer token in a provider
body or nested error string and assert it is not present in output.

Raw provider bodies may be summarized or included only after redaction. Do not
include request headers, authorization values, full raw request payloads, or
local environment values.

## Testing

Focused tests should cover:

- WorldRouter submit `402` with `seedance_balance_too_low` returns a diagnostic
  with `phase=submit`, `httpStatus=402`, `providerCode`, and a credits hint.
- WorldRouter submit `429` with `seedance_too_many_pending_tasks` returns a
  pending-tasks hint.
- WorldRouter submit `500` preserves the redacted response message and returns a
  provider-internal-error hint.
- WorldRouter remote failed job still returns `providerJobId`, `remoteStatus`,
  `error`, and a diagnostic.
- BytePlus non-2xx submit errors use the same diagnostic shape.
- Internal tool failure output preserves the structured diagnostic object.
- Secret-like values in response bodies are redacted.
- Successful video output still includes diagnostic keys with `null` values.

## Acceptance Criteria

- A Milhous-style WorldRouter submit failure no longer stops at
  `phase=submit`; the user can see HTTP status, provider code/message, and a
  hint when the provider returns them.
- Failed remote jobs remain self-contained in `videogen` JSON.
- The diagnostic shape is shared across media providers.
- No background worker, retry policy, provider health state, or provider
  failover is introduced.
- No secrets or credentials are exposed in stdout, persisted job files, or
  desktop-visible DTOs.
