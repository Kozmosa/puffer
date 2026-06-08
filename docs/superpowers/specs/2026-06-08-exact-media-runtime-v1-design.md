# Exact Media Runtime V1 Design

Date: 2026-06-08

## Summary

Puffer should converge image and video generation onto one exact media
selection, capability, validation, job, and artifact contract. This is not a
from-scratch media platform. The repo already has a useful foundation in
`crates/puffer-core/runtime/media`: exact image capability resolution,
image adapters, job/artifact sidecars, preview helpers, and a Replicate video
adapter module.

V1 should therefore focus on closing the gaps:

- unify persisted image/video defaults;
- expose video capabilities through the same resolver surface as image;
- make desktop settings fully capability-parameter driven;
- route `/video` through the existing job/artifact runtime once a concrete
  video adapter is available;
- keep all provider-specific request logic inside existing adapter modules.

## Goals

- Support every connected provider that declares a concrete media capability
  whose adapter is implemented.
- Store image and video defaults with the same selection shape.
- Render image and video settings from capability parameters, with no
  hard-coded video `aspectRatio` or `durationSeconds` UI logic.
- Reuse existing media job and artifact persistence instead of adding a new
  crate, database, object store, or workflow engine.
- Normalize sync image and async video execution behind daemon/API results that
  reference jobs and persisted artifacts.
- Make unsupported provider/model/adapter/parameter selections fail before a
  provider request is sent.

## Non-Goals

- No backward-compatible media config shape preservation.
- No generic provider request-template DSL.
- No new `puffer-media` workspace crate.
- No generic async adapter trait unless duplication across at least two video
  adapters proves it is needed.
- No provider-specific Svelte settings components.
- No dynamic video discovery in V1.
- No audio generation, video edit, video extend, image-to-video, or video-to-
  video operation.
- No natural-language media intent detection.
- No fake progress percentages.
- No generated media bytes in transcript events.

## Current State And Gaps

Already present:

- `MediaKind::Image` and `MediaKind::Video`;
- `MediaCapability` and `MediaCapabilityParameter`;
- exact image resolver and capability DTOs;
- image execution adapters: `images_json`, `chat_image_output`,
  `minimax_image`;
- `MediaGenerationService` job and artifact sidecars;
- generated image preview/attachment helpers;
- `replicate_video.rs` with submit, poll, cancel, download, and persistence
  tests;
- desktop and daemon media settings DTOs;
- desktop `MediaSettingsModal` with image parameters from capabilities.

Remaining gaps:

- `ProviderMediaDescriptor` only models image;
- video capability resolution currently returns no selectable capabilities;
- video settings persist provider/model plus special fields instead of adapter
  and parameters;
- desktop video settings still depend on hard-coded aspect/duration defaults;
- `/video` generation reports unsupported/no-capability instead of executing a
  validated video adapter;
- generated media preview helpers are image-biased.

## Unified Settings Contract

Replace the current split image/video settings with one selection type:

```ts
type MediaSettings = {
  image: MediaGenerationSelection | null;
  video: MediaGenerationSelection | null;
};

type MediaGenerationSelection = {
  providerId: string;
  modelId: string;
  operation: "generate";
  adapter: string;
  parameters: Record<string, string>;
};
```

The media branch (`image` or `video`) supplies the kind, so the selection does
not need a duplicate `kind` field. Capability identity remains:

```text
kind + providerId + modelId + operation + adapter
```

Rules:

- `null` means that media kind is not configured.
- `parameters` stores capability parameter names and exact string values.
- Values such as video duration are persisted as capability values, for example
  `"8"`, not as separate numeric fields.
- UI helpers may display `"8"` as `8s`; display formatting is not persisted.
- Saving settings validates against currently available capabilities.
- Generation validates again to catch stale config.

This intentionally removes:

- `ImageMediaSettings.adapter` as an image-only special case;
- `VideoMediaSettings.aspectRatio`;
- `VideoMediaSettings.durationSeconds`.

## Capability Contract

Keep one DTO for image and video:

```ts
type MediaCapabilityInfo = {
  providerId: string;
  providerDisplayName: string;
  modelId: string;
  modelDisplayName: string;
  kind: "image" | "video";
  operation: "generate";
  adapter: string;
  parameters: MediaCapabilityParameterInfo[];
  defaults: Record<string, string>;
  status: "available" | "unavailable" | "unknown";
  source: string;
  reason: string | null;
  checkedAtMs: number;
};

type MediaCapabilityParameterInfo = {
  name: string;
  label: string;
  values: string[];
  default: string;
  requestField: string | null;
};
```

Rules:

- Normal UI lists only `status == "available"` capabilities.
- `values` must be non-empty and must include `default`.
- One-value parameters render as read-only settings.
- Multi-value parameters render as selects.
- `requestField` maps the selection parameter name to the provider request
  field inside the adapter.
- Capability DTOs stay descriptor/adapter output, not frontend inference.

## Provider Descriptor Model

The provider registry should model media by kind, not by image-only structs.
Since backward compatibility is out of scope, the clean target shape is:

```rust
pub struct ProviderMediaDescriptor {
    pub image: Option<MediaKindDescriptor>,
    pub video: Option<MediaKindDescriptor>,
}

pub struct MediaKindDescriptor {
    pub discovery: Option<MediaDiscoveryDescriptor>,
    pub execution: Option<MediaExecutionDescriptor>,
    pub models: Vec<MediaModelDescriptor>,
}
```

`MediaModelDescriptor`, `MediaParameterSpec`, `MediaExecutionDescriptor`, and
`MediaOperation::Generate` remain shared. `MediaExecutionKind` grows only when
an adapter is implemented. For V1 video, the only new execution kind should be:

```rust
pub enum MediaExecutionKind {
    ImagesJson,
    ChatImageOutput,
    MinimaxImage,
    ReplicateVideo,
}
```

Descriptor rules:

- Model ids must be concrete. Empty ids, `auto`, wildcards, and regex markers
  are invalid.
- `operations` must include `generate`.
- Each parameter must have a non-empty name, label, values list, and default.
- `default` must be one of `values`.
- Provider-level execution can be overridden by model-level execution.
- Static descriptors are enough for V1 video. Dynamic video discovery is
  deferred.

Example:

```yaml
media:
  video:
    execution:
      adapter: replicate_video
      path: /v1/predictions
    models:
      - id: owner/model-version
        display_name: Replicate Video Model
        operations: [generate]
        parameters:
          - name: aspect_ratio
            label: Aspect ratio
            values: ["16:9", "9:16"]
            default: "16:9"
          - name: duration
            label: Duration
            values: ["5", "8"]
            default: "5"
```

## Capability Resolution

The resolver continues to take:

```text
ProviderRegistry + AuthStore + MediaKind + MediaOperation + discovery cache
```

V1 behavior:

- image resolution keeps the current static + trusted image discovery path;
- video resolution uses static descriptors only;
- providers without auth are skipped unless explicitly auth-free;
- descriptors with unimplemented execution adapters are skipped;
- invalid model ids and invalid parameters are skipped;
- emitted capability identity includes kind, operation, and adapter.

Validation should be kind-generic:

```rust
pub(crate) struct MediaGenerationSelection<'a> {
    pub(crate) kind: MediaKind,
    pub(crate) provider_id: &'a str,
    pub(crate) model_id: &'a str,
    pub(crate) operation: MediaOperation,
    pub(crate) adapter: &'a str,
    pub(crate) parameters: &'a BTreeMap<String, String>,
}
```

The existing image-specific validation can be kept as a wrapper while callers
move to the generic path.

## Runtime Boundary

Do not introduce a generic async adapter trait in V1. The existing adapter
style is enough:

- `ImagesJsonAdapter::execute(...)`;
- `MinimaxImageAdapter::execute(...)`;
- `ChatImageOutputAdapter::execute_with_discovery_cache(...)`;
- `ReplicateVideoAdapter::submit(...)`;
- `ReplicateVideoAdapter::poll(...)`;
- `ReplicateVideoAdapter::poll_until_terminal(...)`;
- `ReplicateVideoAdapter::cancel(...)`.

Add a small public media-runtime facade instead:

```rust
pub struct ExactMediaGenerationRequest {
    pub kind: String,
    pub provider_id: String,
    pub model_id: String,
    pub operation: String,
    pub adapter: String,
    pub prompt: String,
    pub parameters: BTreeMap<String, String>,
    pub count: u8,
}
```

The facade dispatches by `kind + adapter`. Unsupported adapters fail before
network I/O.

V1 execution:

- image requests continue to return completed/succeeded jobs and artifacts;
- video requests submit and poll Replicate to a terminal state inside the daemon
  request path;
- background job continuation can be added later if a second UX needs it.

This avoids building a worker scheduler before the desktop has a real need for
detached long-running job management.

## Job And Artifact Contract

Keep existing job statuses:

```text
queued
running
succeeded
failed
canceled
```

Do not add `canceling`, `expired`, `streaming`, or `completed` in V1. Provider
remote status can be preserved in `remote_status`.

Extend `MediaJob` only where it improves exact replay and diagnostics:

```rust
pub(crate) adapter: Option<String>,
pub(crate) parameters: BTreeMap<String, String>,
```

These fields let video polling/resume paths know which adapter and settings
created a job without reading transient request state.

Artifacts stay in the existing structure:

- image bytes may use `.puffer/media/images` where current preview code expects
  image-specific paths;
- generic/video bytes use `.puffer/media/artifacts`;
- all artifacts have sidecars under `.puffer/media/artifact-sidecars`;
- transcript attachments reference artifact ids and local metadata, not bytes.

Video preview support should be metadata-first in V1. The UI can show local path,
MIME type, and open/save actions without loading entire video bytes into memory.

## Desktop UI Design

`MediaSettingsModal` becomes capability-parameter driven for both kinds:

- title remains `Image generation settings` or `Video generation settings`;
- primary button is always `Save`;
- loading status aligns by kind:
  - `Loading image capabilities...`
  - `Checking available image generation models.`
  - `Loading video capabilities...`
  - `Checking available video generation models.`
- provider/model controls come from available capabilities;
- parameter controls come from `capability.parameters`;
- one-value parameters render read-only rows;
- multi-value parameters render selects;
- stale saved selections show warning and disable save until changed.

The modal should not contain field-name special cases for `aspect_ratio`,
`duration`, `size`, `quality`, or `output_format`. Those are labels and values
from capabilities.

Generated media attachment loading should share one surface for image and video,
with kind-specific labels only where helpful. Video shows progress only if the
job has real provider progress.

## Error Handling

Errors should be early and specific:

- no media default: `<kind> media provider/model is not configured`;
- no capability: `No <kind> capabilities available`;
- stale selection:
  `selected <kind> model unavailable: <provider>/<model> via <adapter>`;
- unsupported parameter:
  `<kind> generation parameter unsupported: <name>=<value>`;
- unsupported adapter:
  `<kind> media adapter unavailable for <adapter>`;
- provider failure: redacted adapter error;
- video output download failure: failed job with persisted error.

Settings save may clamp missing parameter values to defaults, but generation
must reject unknown parameter names and unsupported values.

## Performance And Stability

- Capability resolution stays lazy and kind-scoped.
- Capability caches live in daemon/core runtime, not in Svelte components.
- Video static descriptors do not require live provider discovery.
- Replicate polling uses the existing bounded backoff.
- Video bytes are written to disk and not loaded into transcript memory.
- Provider URLs are downloaded before a job is exposed as succeeded.
- Tests use mocked transports; no live provider calls are required.

## Testing Strategy

Unit tests:

- provider descriptor validation accepts image and video descriptors;
- invalid video model ids and invalid parameters are rejected;
- resolver emits video capabilities only for connected providers with
  `replicate_video`;
- generic selection validation rejects stale provider/model/adapter and invalid
  parameter values;
- facade dispatch rejects unsupported adapters before HTTP;
- Replicate video request maps `aspect_ratio` and `duration` parameters into
  the current request shape.

Desktop tests:

- image and video settings show aligned loading UI;
- `Save` label is stable for both kinds;
- video settings render capability parameters without hard-coded defaults;
- selecting video aspect ratio/duration writes `parameters`;
- stale selections warn and cannot save;
- generated video attachment metadata renders without reading video bytes.

Integration tests:

- `list_media_capabilities(kind=video)` returns descriptor-backed capabilities
  for a connected Replicate provider;
- `/video` with valid config creates a video job and persisted artifact through
  mocked transport;
- `/image` still works through existing image adapters after the settings shape
  change.

## Implementation Slices

1. Add the unified media selection config and DTOs.
2. Update frontend/fake daemon settings shape and media modal tests.
3. Convert `MediaSettingsModal` to parameter-driven rendering for video.
4. Generalize provider media descriptor from image-only to image/video.
5. Add `replicate_video` as an execution kind and descriptor-backed capability.
6. Generalize selection validation across media kinds.
7. Add the exact media generation facade and keep image wrappers intact.
8. Wire daemon and desktop `generate_media` to the facade for image/video.
9. Add metadata-first generated video attachment UI support.

Each slice should land with tests. Stop and revisit the design if a slice
requires a scheduler, a new crate, provider-specific UI, dynamic video
discovery, or a generic adapter trait.

