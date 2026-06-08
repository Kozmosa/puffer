# Video Provider Capabilities Design

Date: 2026-06-08

## Purpose

Puffer Desktop currently shows `No video capabilities available` because the
daemon returns zero available video capabilities when no connected provider has a
usable `media.video` descriptor. The long-term fix is not a UI workaround. The
daemon should expose accurate video capability metadata for every supported
provider, and the desktop should distinguish "no provider supports video" from
"video providers exist, but none are connected".

This design intentionally does not preserve old stored media settings or adapter
IDs. The priority is stable runtime behavior, clear provider contracts, and no
capability that appears selectable unless Puffer can execute it.

## Current State

- `resources/providers/relaydance.yaml` already exists in this checkout and
  declares one Seedance video model through the current `openai_video` adapter.
- `resolve_video_capabilities` only returns providers that are authenticated.
  A provider with a valid static descriptor is invisible until it has a stored
  credential.
- The current `openai_video` adapter is not OpenAI's official video API. It is a
  Relaydance/NewAPI-style endpoint using `/v1/video/generations`, task polling by
  appending `/{id}`, and completed video URLs at `metadata.url`.
- The implemented video execution adapters are `openai_video` and
  `replicate_video`; the provider catalog has no bundled `replicate` provider.

## Goals

- Audit every bundled provider descriptor for video support.
- Add or keep `media.video` declarations only for providers with a clear
  execution path.
- Add minimal provider-specific adapters where the provider has a documented
  asynchronous REST contract.
- Reserve the `openai_video` adapter name for OpenAI's official `/v1/videos`
  API. Rename the existing Relaydance-shaped adapter because backward
  compatibility is out of scope.
- Return unavailable video descriptors with a clear reason, so the UI can show a
  connect callout instead of a dead-end empty state.
- Keep settings fast: no network discovery on modal open; all first-version
  video capabilities are static descriptors.

## Non-Goals

- No video editing, extension, first-frame, last-frame, or reference-image modes
  in this pass.
- No webhook callback support in this pass.
- No dynamic video model discovery unless a provider requires it to avoid
  hardcoding an unstable model list.
- No compatibility shim for old `openai_video` stored settings.
- No support for provider APIs that are only documented through a JS SDK
  experimental wrapper without a stable REST contract.

## Provider Audit

| Provider | Video support decision | Reason |
| --- | --- | --- |
| `anthropic` | Do not declare | No provider descriptor or official REST video generation contract in scope. |
| `byteplus` | Declare after exact model IDs are verified from official ModelArk docs | BytePlus ModelArk documents Seedance 2.0 video task APIs, but the model IDs must be sourced from first-party docs before YAML is changed. |
| `cerebras` | Do not declare | Text inference provider only. |
| `groq` | Do not declare | Text inference provider only. |
| `kimi-coding` | Do not declare | Text/coding provider only. |
| `kimi-openai` | Do not declare | Text/coding provider only. |
| `llama-cpp` | Do not declare | Local text inference provider only. |
| `lmstudio` | Do not declare | Local text inference provider only. |
| `minicpm5` | Do not declare | Local text inference provider only. |
| `minimax` | Declare | MiniMax documents text-to-video task creation, task query, and file retrieval. |
| `minimax-cn` | Declare | Same adapter as `minimax`, with CN base URL. |
| `ollama` | Do not declare | Local text inference provider only. |
| `openai` | Declare | OpenAI documents `/v1/videos`, job polling, and content download for Sora. |
| `openrouter` | Declare | OpenRouter documents `/api/v1/videos`, polling URL, and `unsigned_urls` download. |
| `relaydance` | Declare | Already bundled; keep as a first-class video provider and rename the adapter to match its contract. |
| `vercel-ai-gateway` | Defer | Vercel documents video through AI SDK v6 `experimental_generateVideo`; do not add a Rust REST adapter until the stable HTTP contract is explicit. |
| `vllm` | Do not declare | Local text inference provider only. |
| `worldrouter` | Do not declare | No verified first-party video generation contract in scope. |
| `xai` | Declare | xAI documents `/v1/videos/generations`, request ID polling, and video URL output. |
| `zhipu` | Declare | Zhipu/Z.AI documents `/paas/v4/videos/generations` and async result retrieval for CogVideoX/Vidu models. |

## Capability Semantics

`list_media_capabilities` should return declared capabilities for the requested
kind, not only authenticated capabilities. Each capability should carry:

- `status = "available"` when the provider is connected and the adapter is
  implemented.
- `status = "unavailable"` when the descriptor exists but the provider is not
  connected.
- `reason = "missing_auth"` for missing credentials.
- `reason = "adapter_unavailable"` for descriptors whose adapter is not compiled
  into this runtime.

Generation continues to validate only `status = "available"` selections. The UI
can render a connect prompt from unavailable video capabilities while still
preventing selection and generation until credentials exist.

## Adapter Model

Use one adapter per stable provider protocol family. Avoid a universal video
adapter because the APIs differ in request shape, status values, poll URL
construction, and download handoff.

Planned adapters:

- `relaydance_video`: replaces the current Relaydance-shaped `openai_video`
  adapter. It submits to `/v1/video/generations`, polls `/{task_id}`, maps
  queued/running/completed/failed statuses, and downloads `metadata.url`.
- `openai_video`: new official OpenAI adapter. It submits to `/v1/videos`, polls
  `/v1/videos/{video_id}`, and downloads `/v1/videos/{video_id}/content`.
- `byteplus_video`: submits content generation tasks, retrieves task status, and
  downloads the completed video URL. YAML model declarations wait for verified
  first-party model IDs.
- `minimax_video`: submits `/v1/video_generation`, polls
  `/v1/query/video_generation`, retrieves file metadata via `/v1/files/retrieve`,
  then downloads the file URL.
- `xai_video`: submits `/v1/videos/generations`, polls `/v1/videos/{request_id}`,
  and downloads `video.url`.
- `openrouter_video`: submits `/api/v1/videos`, polls the returned
  `polling_url`, and downloads the first `unsigned_urls` entry.
- `zhipu_video`: submits `/paas/v4/videos/generations`, polls the shared async
  result endpoint, and downloads the returned video URL.

Small shared helpers are acceptable for bounded polling, status normalization,
secret redaction, HTTPS/loopback-safe downloads, and media job/artifact
persistence. Provider request/response parsing should stay in provider-specific
modules.

## Descriptor Shape

Provider YAML remains the source of truth for visible settings:

- `media.video.discovery.adapter = static`
- `media.video.execution.adapter = <provider_video_adapter>`
- `media.video.execution.base_url` when media endpoints differ from chat base
  URLs.
- `media.video.execution.path` for task creation.
- model descriptors include only text-to-video generation in this pass.
- parameter descriptors include only stable scalar/select parameters exposed by
  the UI: duration, resolution or size, aspect ratio, quality, fps, and audio
  toggle where provider docs define bounded values.

Do not expose free-form provider passthrough parameters in the settings modal.
They are too easy to misuse and would make generated settings hard to validate.

## Desktop Behavior

The video settings modal should split states:

- Loading: existing loading state.
- Available capabilities exist: show provider/model/parameter controls.
- Only unavailable video capabilities exist: show a concise connect-provider
  state listing provider display names.
- No declared video capabilities exist: show a true empty state.
- Saved video model unavailable: keep the existing warning, but include the
  provider reason when available.

The save path stores the same typed media settings shape. It should only allow
available capabilities.

## Runtime Flow

1. User saves a video selection from a provider with `status = "available"`.
2. `generate_media` loads the saved selection and validates it against current
   capabilities.
3. The selected adapter creates a media job sidecar before polling.
4. The adapter polls with bounded retry/backoff and no unbounded loops.
5. On success, the adapter downloads the MP4 into `.puffer/media/artifacts`,
   writes an artifact sidecar, and returns the same `GenerateMediaResult` shape
   used by current video generation.
6. On failure, the adapter stores the failed job state and returns a redacted
   provider error.

## Performance and Stability

- Capability listing is local and static. It should not call provider APIs.
- Polling is bounded per adapter, with provider-specific intervals where docs
  recommend them.
- Video downloads use the existing safe downloader behavior or an equivalent
  helper that only permits HTTPS and loopback URLs.
- Provider secrets, headers, query parameters, and bearer tokens are redacted
  from all returned errors.
- Every adapter test uses fake transports; no live provider calls in unit tests.

## Tests

Add focused tests for:

- Provider descriptors parse after new adapter enum variants are added.
- Connected providers appear as `available`; unconnected providers appear as
  `unavailable` with `missing_auth`.
- Unimplemented or mismatched adapters never become available.
- Each adapter maps provider submit/poll/download success into a persisted MP4
  artifact.
- Each adapter maps terminal failure/cancel/expired statuses into failed or
  canceled jobs with redacted errors.
- Desktop media settings renders unavailable video providers as a connect state,
  not as `No video capabilities available`.
- `generate_media` rejects unavailable saved selections.

## Implementation Order

1. Rename the current Relaydance-shaped adapter to `relaydance_video` and update
   `relaydance.yaml`.
2. Change capability resolution to return unavailable declared capabilities with
   reasons while generation validation still requires availability.
3. Update the desktop modal empty/connect states.
4. Add official OpenAI, xAI, MiniMax, OpenRouter, and Zhipu adapters and YAML
   declarations.
5. Add BytePlus adapter and provider declaration only after exact first-party
   model IDs are verified.
6. Run targeted Rust tests, Svelte tests, and then the broader workspace check
   that is practical for this repository.

## Source References

- OpenAI Videos API: https://developers.openai.com/api/reference/resources/videos/
- OpenAI Sora guide: https://developers.openai.com/api/docs/guides/video-generation
- xAI video REST API: https://docs.x.ai/developers/rest-api-reference/inference/videos
- OpenRouter video API: https://openrouter.ai/docs/api/api-reference/video-generation/create-videos
- OpenRouter polling API: https://openrouter.ai/docs/api/api-reference/video-generation/get-videos
- MiniMax video generation guide: https://platform.minimax.io/docs/guides/video-generation
- MiniMax text-to-video API: https://platform.minimax.io/docs/api-reference/video-generation-t2v
- BytePlus Seedance 2.0 API reference: https://docs.byteplus.com/en/docs/ModelArk/1520757
- BytePlus retrieve video task API: https://docs.byteplus.com/en/docs/ModelArk/1521309
- Zhipu video generation async API: https://docs.bigmodel.cn/api-reference/%E6%A8%A1%E5%9E%8B-api/%E8%A7%86%E9%A2%91%E7%94%9F%E6%88%90%E5%BC%82%E6%AD%A5
- Vercel AI Gateway video generation: https://vercel.com/docs/ai-gateway/capabilities/video-generation
