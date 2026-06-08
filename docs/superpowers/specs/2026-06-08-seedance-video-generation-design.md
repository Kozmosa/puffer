# Seedance Video Generation — Design

**Date:** 2026-06-08
**Status:** Approved (design)
**Branch:** feat/chat-image-generation

## Problem

The desktop "Video generation settings" modal shows no available provider or
model. Root cause (verified): **no provider resource declares a `media.video`
section**. Video capability discovery (`resolve_video_capabilities`,
`crates/puffer-core/runtime/media/resolver.rs`) emits a capability only when a
provider's registry descriptor has `media.video` whose execution adapter
resolves to a supported video `MediaExecutionKind`. Every shipped provider YAML
declares only `media.image`. The video runtime skeleton (submit → poll →
artifacts) exists and is tested via the `replicate_video` adapter, but no real
video provider is wired.

### Provider audit (code side × reality side)

- 18 providers; 8 declare `media:` (all image-only); **0 declare `media.video`**.
- Only video execution adapter implemented: `replicate_video` (Replicate-style
  async `POST /v1/predictions` → `GET` poll). No real provider uses it.
- Reality side: 7 providers' real APIs support video (byteplus/Seedance,
  minimax/Hailuo ×2, zhipu/CogVideoX, xai/Grok, openai/Sora, openrouter). All
  use async submit→poll; none is OpenAI-compatible for video. The remaining
  providers are text-only inference or local runtimes and will never have video.

## Decision

Wire up **BytePlus Seedance**, direct to ModelArk, via a dedicated
`seedance_video` execution adapter — **fully symmetric with how image
generation is already handled** (byteplus direct + `images_json` adapter +
structured params declared in YAML).

Rationale (constraints: no backward compat, long-term ROI, stability,
performance, avoid over-engineering):

- The image path is the proven prescription: direct-to-provider +
  per-provider adapter + YAML-declared structured params. Mirroring it yields a
  video path structurally identical to existing code — lowest cognitive cost,
  most stable, least over-designed.
- BytePlus is already the configured image provider (`byteplus.yaml`,
  `base_url: https://ark.ap-southeast.bytepluses.com/api/v3`). Seedance video
  lives under the **same base_url** (`/contents/generations/tasks`). No new
  provider, no new auth, no gateway layer.
- OpenRouter would cover more models with one adapter but introduces a gateway
  dependency inconsistent with the current self-hosted direct-connect baseline.
  Rejected for first cut.

### Explicit non-goals (anti-over-engineering)

- No generic "video adapter" trait abstraction — only one video provider exists
  (YAGNI). Keep the existing per-adapter match-arm dispatch in
  `generate_exact_video_from_media_request`.
- Do not remove or change `replicate_video` (tested, no provider references it,
  serves as a second reference; removing it is unrelated cleanup).
- Do not touch any image-generation code path.
- No defensive handling for ModelArk concurrency limits (3 tasks / QPS 2) — a
  single `/video` command submits one task; `validate_video_count` already caps
  count at 1.

## Architecture

| | Image (existing) | Video (this design) |
|---|---|---|
| Provider | byteplus, direct ModelArk | same byteplus, same base_url |
| YAML | `media.image` + `adapter: images_json` | new `media.video` + `adapter: seedance_video` |
| Adapter module | `images_json` | new `seedance_video.rs` (mirrors `replicate_video.rs`) |
| Protocol | sync `POST /images/generations` | async `POST /contents/generations/tasks` → poll `/tasks/{id}` |
| Param mapping | structured → request fields | structured → prompt-inline `--ratio/--duration/--resolution` (in adapter) |

## Components

### 1. `byteplus.yaml` — add `media.video`

```yaml
media:
  image: { ... }            # unchanged
  video:
    discovery:
      adapter: static
    execution:
      adapter: seedance_video
      path: /contents/generations/tasks
    models:
      - id: dreamina-seedance-2-0-260128
        display_name: Seedance 2.0
        operations: [generate]
        parameters:
          - { name: resolution, label: Resolution,  values: [480p, 720p, 1080p], default: 1080p, request_field: resolution }
          - { name: ratio,      label: Aspect ratio, values: ["16:9","9:16","1:1"], default: "16:9", request_field: ratio }
          - { name: duration,   label: Duration,     values: ["5","10"],          default: "5",    request_field: duration }
      - id: dreamina-seedance-2-0-fast-260128
        display_name: Seedance 2.0 Fast
        operations: [generate]
        parameters: [ ... same as above ... ]
```

`request_field` reuses the existing schema; for video its semantics =
the `--` flag name. The adapter builds `--resolution 1080p --ratio 16:9
--duration 5`. UI, capability resolution, and parameter validation all flow
through existing logic — no new frontend.

### 2. `crates/puffer-core/runtime/media/seedance_video.rs` (new)

Mirrors `replicate_video.rs`:

- `SeedanceVideoTransport` trait: `submit_task(base_url, key, body) -> {id}`,
  `poll_task(base_url, key, id) -> {status, content.video_url}`,
  `download_bytes(url)`.
- `ReqwestSeedanceVideoTransport`: `POST {base}/contents/generations/tasks`,
  `GET {base}/contents/generations/tasks/{id}`, `Authorization: Bearer`.
- `SeedanceVideoAdapter`: `submit()` (build queued job) → `poll_until_terminal()`
  (status normalization: `succeeded` → done; `failed`/`expired` → error with
  ModelArk code/message) → load artifacts.
- `seedance_request_from_parameters()`: the only genuinely new logic — maps
  structured params into the prompt-inline `--` string and assembles the
  `content` array. Isolated and unit-testable.

### 3. Four wiring points (all small)

1. `crates/puffer-provider-registry/src/model.rs` (`MediaExecutionKind` enum):
   add `SeedanceVideo` (serde snake_case → `"seedance_video"`).
2. `crates/puffer-core/runtime/media/resolver.rs`
   (`execution_adapter_is_available_for_kind`): add
   `(MediaKind::Video, MediaExecutionKind::SeedanceVideo)`; add `adapter_id`
   mapping.
3. `crates/puffer-core/runtime/media/mod.rs`: add
   `pub(crate) mod seedance_video;`.
4. `crates/puffer-core/media_runtime.rs`
   (`generate_exact_video_from_media_request`): add match arm
   `"seedance_video" => { ... }`, symmetric with the existing `replicate_video`
   arm (resolve provider, bearer token, submit, poll, load artifacts).

## Data flow

`/video <prompt>` → backend/daemon `generate_media_job("video", …)` →
`generate_exact_media_with_cache` → `generate_exact_video_from_media_request` →
`validate_media_generate_selection` (existing param validation) →
`seedance_video` arm → `SeedanceVideoAdapter.submit()` → ModelArk task id →
`poll_until_terminal()` → `content.video_url` → download MP4 →
`MediaGenerationService` persists artifact (same path scheme as images).

## Error handling & stability

- **Bounded polling:** `SeedancePollingConfig` (interval ~3s, generous
  minute-scale total timeout, since video is slow), reusing the
  `poll_until_terminal` pattern — no new mechanism.
- **Status normalization:** `failed`/`expired` → clear error carrying ModelArk
  error code/message; never silent.
- **Download validation:** enforce https on `video_url` (mirrors
  `replicate_video` scheme check); persist via `MediaGenerationService`.
- **Count:** `validate_video_count` already enforces count == 1.

## Testing

Mirror existing `replicate_video` and `daemon.rs` cases — no new test paradigm:

- `seedance_video.rs` unit tests with a fake transport: submit saves queued job;
  poll downloads completed MP4 artifact; `seedance_request_from_parameters`
  builds the correct prompt-inline `--` string and `content` array; `failed`
  status surfaces ModelArk error.
- daemon/backend tests: capability discovery returns the Seedance video
  capability; `generate_media` rejects a stale/mismatched adapter with a clear
  error.

## Out of scope

- Image-to-video (first_frame / last_frame), reference images, audio.
- Other providers (minimax/zhipu/xai/openai/openrouter) — separate specs.
- Relaydance gateway routing — current image path is direct ModelArk; video
  mirrors that. Gateway is a later, orthogonal decision.
