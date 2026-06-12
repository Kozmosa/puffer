# WorldRouter Video Adapter Design

Date: 2026-06-12

Status: Approved design, image-to-video included, implementation plan written

Constraints: do not preserve backward compatibility for the old WorldRouter
video path; optimize for long-term clarity, stability, and runtime performance;
avoid a broad async-video framework.

## Problem

The `Milhous`-reported video generation failure surfaced as:

```text
provider=worldrouter adapter=relaydance_video task=unknown
```

The failing session ran:

```bash
videogen --prompt '创建一个机器人战斗的视频'
```

The active user media config selected:

```toml
[media.video]
provider_id = "worldrouter"
logical_model_id = "seedance-2.0-fast"

[media.video.selections]
duration = "5"
resolution = "480p"
```

The bundled WorldRouter provider descriptor currently routes video generation
through the generic `relaydance_video` adapter:

```yaml
video:
  execution:
    adapter: relaydance_video
    base_url: https://inference-api.worldrouter.ai
    path: /api/v3/contents/generations/tasks
    prompt_format: content_array
```

WorldRouter's official Seedance API is not the old Relaydance/NewAPI task
shape. It is a native async task API:

- Submit: `POST /api/v3/contents/generations/tasks`
- Submit response: `{ "id": "task-123", "requestId": "req-123" }`
- Poll: `GET /api/v3/contents/generations/tasks/{task_id}`
- Poll success response includes `status: "succeeded"` and
  `content.video_url`.

The current `relaydance_video` submit path parses the submit response with
`RelaydanceVideoTask::from_value`, which requires both a task id and a
`status`. Because WorldRouter submit responses do not include `status`, submit
parsing fails before a `MediaJob` is persisted. That is why the error says
`task=unknown` and no WorldRouter job file appears in `.puffer/media/jobs`.

## Goals

- Introduce a dedicated `worldrouter_video` adapter for WorldRouter Seedance.
- Make submit parsing match the documented WorldRouter contract: submit only
  needs a task id.
- Persist the queued job immediately after submit succeeds.
- Poll the documented task status endpoint until terminal state.
- Persist generated MP4 artifacts from `content.video_url`.
- Make errors stage-specific and useful for diagnosis.
- Keep shared video job mechanics in `video_jobs`; do not create a generic
  provider framework.
- Support image-to-video through the existing `videogen --image-reference`
  entrypoint for public `https://` image URLs.
- Implement only the WorldRouter asset helper calls required to convert those
  public image URLs into `asset://...` references for the same Seedance request.

## Non-Goals

- Do not support the old WorldRouter-through-`relaydance_video` path.
- Do not preserve compatibility with a WorldRouter submit response that contains
  `status`.
- Do not redesign desktop media settings.
- Do not add background polling or UI event streaming.
- Do not alter BytePlus, Replicate, or Relaydance behavior except where a shared
  adapter enum or adapter availability list requires adding `worldrouter_video`.
- Do not add provider-specific UI.
- Do not support local image paths, `file://`, data URLs, or base64 image
  references.
- Do not support user-supplied `asset://` references for WorldRouter in this
  change. WorldRouter requires an `asset_group_id` that belongs to the same
  owner context; the adapter should create the group and asset URLs itself.
- Do not implement persistent asset library management, asset reuse, asset
  deletion, or asset browsing.
- Do not redesign shared polling behavior, retry budgets, or job reclaim logic.
  Add only the adapter-specific reclaim branch needed to preserve the existing
  async-video reclaim behavior for persisted WorldRouter jobs.

## Selected Approach

Add a focused `worldrouter_video` adapter and switch WorldRouter's video
execution descriptor to it.

Rejected alternatives:

- Keep WorldRouter inside `relaydance_video` with conditional parsing.
  This is smaller, but it preserves a misleading adapter boundary and makes the
  next protocol drift harder to diagnose.
- Build a generic async-video schema framework.
  This is too broad for the current provider set. The only needed abstraction is
  already present in `video_jobs`: polling, terminal status mapping, and artifact
  persistence.

## Architecture

Add:

```text
crates/puffer-media/src/media/worldrouter_video.rs
```

The module owns the WorldRouter Seedance protocol:

- request body construction
- transient asset group creation for image-to-video
- reference image asset upload for public `https://` URLs
- task submission
- submit response parsing
- task polling
- poll response parsing
- successful artifact completion through shared video job helpers

The module should expose a production adapter plus a small test transport, using
the existing pattern from `relaydance_video.rs` and `byteplus_video.rs`.
That transport boundary must stay module-local for HTTP mocking; do not promote
it into a cross-provider async-video abstraction.

Also add the adapter identifier at the typed resource boundary:

- `MediaExecutionKind::WorldRouterVideo` in `puffer-provider-registry`, with
  `#[serde(rename = "worldrouter_video")]` so the wire name stays compact
- video adapter availability in `puffer-media/src/media/resolver.rs`
- `adapter_id(MediaExecutionKind::WorldRouterVideo) == "worldrouter_video"`

Shared helpers remain in:

```text
crates/puffer-media/src/media/video_jobs.rs
```

`worldrouter_video` should reuse:

- `VideoPollingConfig`
- `video_poll_url`
- `poll_video_until_terminal`
- `persist_failed_video_job`
- `complete_video_job`
- `map_video_task_status`

It should not change those helpers' semantics. In particular, polling remains
bounded and synchronous, and transient-poll behavior stays consistent with the
current video runtime.

Provider descriptor update:

```yaml
video:
  execution:
    adapter: worldrouter_video
    base_url: https://inference-api.worldrouter.ai
    path: /api/v3/contents/generations/tasks
    prompt_format: content_array
```

The provider's public model ids stay stable:

- `seedance-2.0`
- `seedance-2.0-fast`

## Data Flow

### Text-To-Video

1. `videogen` sends a `video-generation` internal tool request.
2. Runtime reads `[media.video]` and resolves `worldrouter/seedance-2.0-fast`.
3. Media resolver returns adapter `worldrouter_video`, concrete model id, and
   request-field parameters.
4. `generate_exact_video_from_media_request` dispatches to
   `generate_worldrouter_video`.
5. Adapter submits the task with a body shaped like:

   ```json
   {
     "model": "seedance-2.0-fast",
     "content": [
       { "type": "text", "text": "创建一个机器人战斗的视频" }
     ],
     "resolution": "480p",
     "duration": 5
   }
   ```

6. Submit parser reads `id` and optional `requestId`.
7. Runtime creates a queued `MediaJob` with:

   - `providerId = "worldrouter"`
   - `adapter = "worldrouter_video"`
   - `providerJobId = submit.id`
   - `remoteStatus = null`

8. Polling calls:

   ```text
   GET /api/v3/contents/generations/tasks/{task_id}
   ```

9. Poll parser reads:

   - `id`
   - `status`
   - `content.video_url` when present
   - provider error message fields when present

10. On `succeeded`, `complete_video_job` downloads and persists the MP4.

### Image-To-Video

The user-facing entrypoint stays the current CLI contract:

```bash
videogen --prompt 'animate image 1' --image-reference https://example.com/ref.png
```

When `image_references` is non-empty for WorldRouter:

1. Validate every reference is a public `https://` URL. Reject local paths,
   `file://`, data URLs, base64 payloads, and user-supplied `asset://`.
   Validation happens before any asset-group or asset-upload request.
2. Create one asset group for this generation request:

   ```text
   POST /v1/asset-groups
   ```

   Request body:

   ```json
   {
     "name": "puffer-seedance-video",
     "description": "reference assets for one Puffer Seedance video generation"
   }
   ```

3. Upload each public image URL into that group:

   ```text
   POST /v1/asset-groups/{asset_group_id}/assets
   ```

   Request body for image `n`:

   ```json
   {
     "name": "reference-image-n",
     "description": "Puffer Seedance reference image n",
     "type": "image",
     "url": "https://example.com/ref.png"
   }
   ```

4. Use the returned `asset://...` URLs in the Seedance task body and include
   the returned `asset_group_id`.
5. Keep image references in CLI order. Each image content item uses
   `type: "image_url"` and `role: "reference_image"`.

Example task body:

```json
{
  "model": "seedance-2.0-fast",
  "asset_group_id": "group-1",
  "content": [
    { "type": "text", "text": "animate image 1" },
    {
      "type": "image_url",
      "role": "reference_image",
      "image_url": { "url": "asset://asset-1" }
    }
  ],
  "resolution": "480p",
  "duration": 5
}
```

This design deliberately treats all reference images as `reference_image`.
First-frame/last-frame role selection is a separate UX/API feature because the
current CLI only carries ordered references, not role metadata.

## Parsing Rules

### Asset Group Response

Required:

- `id` as a non-empty string

Optional fields such as `requestId`, `name`, and `description` are not
persistent runtime state.

### Asset Upload Response

Required:

- `url` as a non-empty string beginning with `asset://`

Optional:

- `id`
- `asset_group_id`
- `source_url`
- `requestId`

The adapter should not accept an uploaded asset response that lacks an
`asset://` URL.

### Submit Response

Required:

- `id` as a non-empty string

Optional:

- `requestId`

The submit parser must not require `status`.
`requestId` is useful for diagnostics but does not need new persistence unless
the existing job model already has a natural place for it. Do not expand
`MediaJob` just to store `requestId`.

### Poll Response

Required:

- `id` as a non-empty string
- `status` as a non-empty string

Success:

- `status == "succeeded"` maps to `MediaJobStatus::Succeeded`
- `content.video_url` is required before artifact completion

Failure:

- `failed` and `expired` map to failed
- `cancelled` maps to canceled
- readable error text should be taken from the most specific available field,
  such as `error.message`, `message`, or provider-specific reason fields.

Unknown status should use the existing shared status mapping behavior: keep the
job non-terminal so bounded polling can continue.

The poll parser should accept the documented WorldRouter shape only. It does
not need to parse Relaydance/NewAPI envelopes such as nested `data.data`,
`metadata.url`, or `result_url`; those remain owned by `relaydance_video`.

## Error Handling

All user-visible adapter errors should include provider, adapter, and phase:

```text
provider=worldrouter adapter=worldrouter_video phase=asset_group
provider=worldrouter adapter=worldrouter_video phase=asset_upload image=1
provider=worldrouter adapter=worldrouter_video phase=submit
provider=worldrouter adapter=worldrouter_video phase=poll task=task-123
provider=worldrouter adapter=worldrouter_video phase=download task=task-123
provider=worldrouter adapter=worldrouter_video phase=validate
```

Asset helper and submit failures are terminal for that tool invocation because
no local video job can be safely resumed without a Seedance task id.
Validation failures are also terminal for that invocation and must happen before
any WorldRouter asset helper request.

Poll transport and parse failures are transient. They should record the latest
job error and keep polling within the bounded attempt budget.

Terminal provider failures must persist the job as failed with the provider's
best available error message.

If a poll response is `succeeded` but lacks `content.video_url`, mark the job
failed with a precise diagnostic:

```text
succeeded WorldRouter video task is missing content.video_url
```

Secrets must continue to be redacted through the existing provider error
redaction path.

## Performance

This design keeps the current synchronous generation behavior. It does not add
background workers, new event streams, or a scheduler.

Runtime cost is bounded by the existing polling configuration. The adapter adds
no extra requests for text-to-video beyond the documented submit plus poll loop.

Image-to-video adds exactly one asset-group request plus one asset-upload
request per image reference before task submission. Do not add caching or asset
reuse in this change; that would require lifecycle and ownership rules beyond
the bug fix.

Request parsing should use direct JSON field access, not schema reflection or a
generic mapping engine.

Avoid introducing a trait or schema DSL for async video providers. The only new
runtime polymorphism needed is the existing `match resolved.adapter.as_str()`
branch.

## Testing

Add fixtures for:

- WorldRouter submit success:

  ```json
  { "id": "task-123", "requestId": "req-123" }
  ```

- WorldRouter poll queued/running.
- WorldRouter poll succeeded:

  ```json
  {
    "id": "task-123",
    "model": "seedance-2.0-fast",
    "status": "succeeded",
    "content": {
      "video_url": "https://media.example.com/output.mp4"
    },
    "resolution": "480p",
    "duration": 5
  }
  ```

- WorldRouter poll failed with an error message.
- WorldRouter asset group success.
- WorldRouter asset upload success.
- WorldRouter asset upload missing `asset://` URL.
- Submit response missing `id`.
- Poll success missing `content.video_url`.

Required test coverage:

- `MediaExecutionKind` parses `worldrouter_video`.
- request body matches WorldRouter docs for text-to-video.
- request body matches WorldRouter docs for image-to-video after asset upload.
- public `https://` image references are uploaded before submit and preserved
  in order.
- local paths, `file://`, data URLs, base64 values, and user-supplied
  `asset://` references are rejected for WorldRouter with a clear message.
- invalid image references are rejected before any asset group is created.
- submit without `status` creates a queued job.
- poll success downloads and stores one MP4 artifact.
- asset helper failures include `phase=asset_group` or `phase=asset_upload`.
- submit parse failure includes `phase=submit` and response shape context.
- poll parse/transport failure is transient when a task id exists.
- `resources/providers/worldrouter.yaml` declares `adapter: worldrouter_video`.
- capability listing exposes WorldRouter Seedance via `worldrouter_video`.
- orphan reclaim polls persisted non-terminal `worldrouter_video` jobs with the
  same one-shot best-effort pattern used by Relaydance and BytePlus.
- the old `relaydance_video` tests still pass unchanged for Relaydance fixtures.

## Acceptance Criteria

- `videogen` with `[media.video] provider_id = "worldrouter"` no longer fails
  at submit because `status` is absent.
- A successful WorldRouter Seedance job persists a local MP4 artifact.
- A successful WorldRouter Seedance image-to-video job uploads public image
  references through asset helper routes and persists a local MP4 artifact.
- WorldRouter failures produce phase-specific diagnostics instead of
  `task=unknown` unless the submit response truly lacks an id.
- No generic async-video framework is introduced.
- `cargo test -p puffer-media` passes.
- Provider resource tests verify the WorldRouter adapter declaration.
- Existing Relaydance and BytePlus video tests still pass without behavior
  changes.
