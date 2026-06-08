# Agent VideoGeneration Tool Design

Date: 2026-06-08

## Problem

Milhous session `c6ad17e3-7444-48a5-ba32-0c92bc89788b` shows a user asking
`创建一个高达的视频`. The agent replied with an intention statement and the turn
ended. The transcript contains no `tool_invocation`, no `VideoGeneration`, and
no `generate_media` call.

The video runtime exists: desktop/daemon `generate_media` accepts
`kind = "video"`, `media.video` settings can point at Relaydance, and
`openai_video` can execute the configured provider. The missing piece is the
agent-facing tool surface. The model has an `ImageGeneration` workflow tool and
image-only prompt guidance, but no equivalent video tool or prompt rule.

## Goals

- Add an agent-callable `VideoGeneration` workflow tool for text-to-video.
- Reuse the existing exact media runtime and configured `media.video`
  provider/model/adapter selection.
- Keep video model onboarding descriptor-driven: new compatible models should be
  YAML entries or small adapter additions, not prompt or agent rewrites.
- Produce persisted generated-media artifacts and transcript-visible
  attachments like image generation.
- Make unsupported image-to-video requests fail clearly instead of creating
  empty intention replies.
- Keep the first version small, stable, and easy to test.

## Non-Goals

- No image-to-video, first-frame, last-frame, reference image, video editing, or
  video extension support in this change.
- No generic `MediaGeneration` super-tool in this change.
- No direct model access to the desktop RPC shape as a workflow tool.
- No provider-specific prompt rules for Relaydance, Seedance, Kling, Vidu, Sora,
  or future models.
- No batch video generation. Video count stays one.
- No compatibility layer for absent or historical video tool names.

## Chosen Approach

Add a dedicated `VideoGeneration` workflow tool and implement it as a thin agent
adapter over the existing exact media runtime.

This mirrors the successful image path without collapsing image and video into a
larger abstraction. The model sees a clear domain tool. The runtime keeps one
media execution facade. Future video providers can attach below the facade
through provider descriptors and adapters without changing the agent contract.

Rejected alternatives:

- A generic `MediaGeneration` tool would reduce one resource file, but it would
  make the model choose `kind`, enlarge the schema, and force image/video result
  semantics into one prompt contract too early.
- Exposing daemon `generate_media` as a workflow tool would be fast but would
  leak UI RPC concerns into the agent layer and bypass the clearer
  workflow-tool permission and transcript semantics.

## Recheck Outcome

The first design pass included one unnecessary cleanup: renaming the existing
`ImageGenerationMediaContext` to a generic media context. That is not needed for
the fix. Two tools do not justify shared context abstraction churn when the
existing image context is local, tested, and only used by image generation.

This design keeps the long-term direction descriptor-driven, but the first
implementation should be conservative:

- Add `VideoGeneration` as a sibling of `ImageGeneration`.
- Add a small `VideoGenerationMediaContext` for provider/auth/discovery inputs.
- Do not rename or move image generation code unless a direct compile boundary
  requires it.
- Extract helper functions only when image and video need identical behavior in
  the same implementation step.

## Tool Contract

Add `resources/tools/video_generation.yaml`:

- `id`: `VideoGeneration`
- `name`: `VideoGeneration`
- `handler`: `runtime:workflow:video_generation`
- `approval_policy`: `ask`
- `sandbox_policy`: `network`
- `display.group`: `media`

Input schema:

```yaml
type: object
properties:
  prompt:
    type: string
    description: Literal text prompt or a workspace-relative prompt file path.
  parameters:
    type: object
    description: Optional scalar video parameter overrides.
    additionalProperties:
      type: string
  purpose:
    type: string
    description: Optional caller purpose preserved in result metadata.
required:
  - prompt
additionalProperties: false
```

The schema intentionally has no image, reference image, frame, URL, or file
input. If a user asks to animate an existing image, the model should report that
Puffer's current agent video tool only supports text-to-video.

Output JSON:

```json
{
  "jobId": "uuid",
  "kind": "video",
  "requestedCount": 1,
  "artifacts": [
    {
      "artifactId": "uuid",
      "index": 0,
      "path": "/absolute/path/video.mp4",
      "mimeType": "video/mp4",
      "size": 1234,
      "remoteSourceUrl": "https://..."
    }
  ],
  "provider": "relaydance",
  "model": "doubao-seedance-2-0-720p",
  "status": "succeeded",
  "parameters": {
    "duration": "5",
    "ratio": "16:9",
    "resolution": "720p"
  },
  "purpose": "create a short mecha video"
}
```

`remoteSourceUrl` is included only when the runtime provides it.

## Runtime Design

Add `crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs`.

The module should be intentionally small:

1. Parse `VideoGenerationInput`.
2. Resolve prompt text the same way `ImageGeneration` does: literal text unless
   the value names a workspace-relative prompt file.
3. Read `state.config.media.video`.
4. Merge saved video settings parameters with tool `parameters` overrides.
5. Build `ExactMediaGenerationRequest` with `kind = "video"`, `operation`,
   `provider_id`, `model_id`, `adapter`, prompt, merged parameters, and
   `count = 1`.
6. Execute `generate_exact_media_with_cache`.
7. Return compact JSON metadata.

The module owns a small `VideoGenerationMediaContext` carrying providers,
auth store, and exact media discovery cache. Keep the existing
`ImageGenerationMediaContext` in place. If a later third media workflow needs
the same wiring, extract a shared context then.

Dispatcher changes stay narrow:

- `workflow/mod.rs` exports `video_generation`.
- `execute_tool` constructs one `ImageGenerationMediaContext` and one
  `VideoGenerationMediaContext` from the same provider/auth/discovery inputs.
- `execute_workflow_tool_with_media_context` accepts the two contexts as
  separate optional arguments. This is deliberately explicit; do not introduce a
  generic media-context trait or enum for two tools.
- Match arm `"VideoGeneration"` calls
  `workflow::video_generation::execute_video_generation`.

## Parameter Semantics

Tool-supplied `parameters` are scalar overrides for descriptor-declared
capability parameters. They do not bypass runtime validation.

Rules:

- Start from saved `media.video.parameters`.
- Apply tool overrides by key.
- Let `validate_media_generate_selection` reject unknown parameter names or
  unsupported values.
- Let capability defaults fill missing descriptor parameters as they do today.

This keeps future video model support data-driven. A provider can add
`duration`, `ratio`, `resolution`, or other scalar parameters in YAML. The agent
tool does not need model-specific fields.

## Prompt Guidance

Update the core system prompt with one concise rule:

- When the user asks to create or generate a video, use `VideoGeneration`.
- Use one `VideoGeneration` call for one logical video request.
- If the user asks to use an existing image, reference image, first frame, or
  last frame, state that the current tool supports text-to-video only.
- If video generation fails or is unavailable, report the error plainly.

This directly prevents the observed Milhous failure mode where the model replied
with an intention but never invoked a tool.

## Transcript And Attachments

The desktop timeline currently synthesizes generated attachments only for
successful `ImageGeneration` tool results. Replace the image-specific timeline
helper with generated-media handling that accepts both `ImageGeneration` and
`VideoGeneration`.

Video attachment behavior is metadata-first:

- Use the artifact id, MIME type, size, path, and optional remote source URL from
  the tool output.
- Do not read video bytes into transcript memory.
- Render the attachment with existing generated-media DTOs where possible.
- Keep image preview behavior unchanged except for shared helper naming.

## Error Handling

Errors should occur before or inside the runtime, not after an empty assistant
reply:

- Missing video settings:
  `video media provider/model/adapter is not configured`
- Empty prompt:
  `VideoGeneration prompt is required`
- Unsupported image input fields:
  rejected by `additionalProperties: false`
- Unsupported parameter:
  current exact media validation error, e.g.
  `video generation parameter unsupported: ratio=4:3`
- Missing provider credential:
  current exact media capability/auth error
- Provider failure:
  redacted adapter error from `openai_video`
- Non-one video count:
  impossible at the tool boundary; runtime still validates count one

The model should see tool failures and explain them instead of inventing a
placeholder.

## Stability And Performance

- Capability resolution remains local and cached.
- Static video descriptors do not require live provider discovery per turn.
- Video generation remains one job per tool call.
- Polling and downloads stay in the existing video runtime.
- Artifacts stay on disk; transcript output contains metadata only.
- No frontend prompt classification is added.
- No new long-lived scheduler or background queue is introduced.

## Testing

Use test-first implementation.

Resource and prompt tests:

- Bundled resources load `VideoGeneration`.
- The tool schema rejects image/reference/frame fields.
- The system prompt includes the `VideoGeneration` rule.
- The prompt includes the text-to-video boundary for image-to-video requests.

Workflow tests:

- Dispatcher routes `"VideoGeneration"` to the workflow module.
- Missing `media.video` settings returns a clear configuration error.
- Empty prompts are rejected.
- Tool parameter overrides are merged with saved video parameters.
- Unknown or unsupported parameter values are rejected by exact media validation.
- A successful mocked video generation returns `kind = "video"` and one artifact.

Desktop/timeline tests:

- A successful `VideoGeneration` transcript event creates a generated-media
  attachment on the next assistant message.
- A video attachment is metadata-first and does not load file bytes.
- Existing `ImageGeneration` attachment tests still pass through the shared
  helper.

Regression scenario:

- A session turn equivalent to `创建一个高达的视频` must have an available
  `VideoGeneration` tool surface and prompt guidance. The regression assertion
  should focus on tool availability and prompt instructions rather than relying
  on a live model to choose the tool.

## Future Video Models

Future text-to-video models should follow this order:

1. Add or update provider YAML `media.video` descriptors when an existing
   adapter can execute the provider protocol.
2. Add a focused adapter only when the provider protocol differs from existing
   adapters.
3. Keep model-specific scalar options in descriptor parameters.
4. Do not add prompt rules or tool fields for one provider's parameter names.

Image-to-video should be designed separately because it needs a durable input
artifact contract from UI/session attachments through daemon params and adapter
request bodies. That is a different subsystem from this text-to-video tool.

## Implementation Boundary

This design is a single implementation plan. Stop and write a separate design if
implementation requires any of the following:

- Image, video, or URL input fields.
- A generic media request DSL.
- Provider-specific frontend controls.
- A scheduler or resumable background job manager beyond existing polling.
- Changes to normal chat model selection.
