# Chat Image Generation Routing - Design

## Scope

Add image generation routing to the desktop chat composer. The chat composer
gets an image button that lets the user select an image-generation model from
connected providers. When the user asks the chat agent to create an image, the
agent uses the existing `ImageGeneration` tool and the tool executes with the
selected provider/model.

This design removes the old env-only image backend behavior. It does not keep a
compatibility path for `PUFFER_IMAGE_MODEL`, `OPENAI_API_KEY`, or
`PUFFER_OPENAI_API_KEY` as the primary routing mechanism. Credentials are
resolved through the provider registry and auth store.

## Goals

- Let users choose an image-generation provider/model from the chat composer.
- Use connected provider credentials, not ad hoc environment variables.
- Keep the agent responsible for deciding when a user prompt needs image
  generation.
- Keep generated image output in the workspace and visible through the existing
  tool-card flow.
- Add explicit, narrow model metadata so image models are not inferred from
  input modality or model names and do not appear in the normal chat model
  picker.
- Keep the change small enough to implement and test without building a full
  media-provider platform.

## Non-Goals

- No client-side natural-language prompt classification.
- No frontend direct image API call that bypasses the agent loop.
- No OpenAI Responses native `image_generation` server tool in this change.
- No image editing, reference-image upload, streaming partial images, or
  multi-image asset library.
- No generic audio/video/media registry.
- No migration or compatibility layer for old image tool inputs.

## Current Behavior

`ConversationView.svelte` already builds turn options in `composerOptions()` and
passes them through `onSubmitMessage`. The current options include provider,
model, thinking, fast mode, and permission mode.

`ModelPicker.svelte` lazy-loads provider models with `listProviderModels` and
filters regular chat models by tool support. This shape is useful, but image
model selection needs a separate image-generation filter.

`ModelDescriptor` currently has `input: Vec<Modality>`. That field describes
what a model can accept as input, such as text or image. It does not describe
what the model can output. The desktop model DTO also does not expose image
generation support.

`ImageGeneration` is already registered as a workflow tool. It validates the
prompt, maps aspect ratios to image sizes, writes a workspace-relative output
file, and returns JSON metadata for the tool card. Its network execution is
currently hardcoded to OpenAI Images API and reads credentials/model from
environment variables.

Provider discovery is authoritative for regular model availability. Fresh
discovery can replace static provider models. That is correct for chat models
but unsafe for static image API models when the discovery endpoint reports only
chat-capable models.

## Design Summary

Use an explicit, turn-scoped image generation route:

1. Provider resources mark image models with `supports_image_generation: true`.
2. Desktop model DTO exposes `supportsImageGeneration` and computes
   `supportsTools: false` for image-only models.
3. The composer image button opens a compact picker filtered to connected
   providers and `supports_image_generation` models.
4. The selected image route is stored per chat session and included in
   `AgentTurnOptions`.
5. The daemon applies the option to `AppState` before the agent turn starts.
6. The runtime system prompt includes one concise line describing the selected
   image generation route.
7. `ImageGeneration` resolves provider, endpoint, credential, and model from
   the selected route and performs the request.

The frontend never calls the image API directly. The model still has to call
the `ImageGeneration` tool, so permissions, tool events, transcript storage,
and tool-card rendering stay in the existing flow.

## Model Metadata

Add one narrow boolean to `ModelDescriptor`:

```rust
#[serde(default)]
pub supports_image_generation: bool,
```

`supports_image_generation` defaults to `false`; only image API models set it
to `true`. Do not overload `input: [image]`; that means the model can consume
image input, not produce images.

`ModelDescriptorDto` exposes:

```ts
supportsTools: boolean;
supportsImageGeneration: boolean;
```

`supportsTools` is computed for the desktop DTO only to protect the normal chat
model picker. For this change, the only new false case is
`supports_image_generation == true`; do not add a cross-crate generic tool
capability model or generic media-capability model. If audio, video, or image
editing become first-class later, they should get their own focused design.

## Provider Endpoint Metadata

Add a small provider-level image generation endpoint config:

```yaml
image_generation:
  path: /v1/images/generations
```

OpenAI uses `/v1/images/generations`. BytePlus ModelArk uses
`/images/generations` because its base URL already includes `/api/v3`.

No request templating or provider-specific expression language is introduced.
The tool sends the common OpenAI-compatible body:

```json
{
  "model": "<model>",
  "prompt": "<prompt>",
  "size": "<size>",
  "n": 1
}
```

Provider-specific optional fields stay out of scope. If a provider needs
mandatory fields that are not OpenAI-compatible, it is not supported by this
first version.

`ProviderSummary` exposes `supportsImageGeneration: boolean`, derived from
whether the provider descriptor has an `image_generation` endpoint. This lets
the frontend avoid offering providers that cannot execute the selected image
model route.

## Discovery Merge Rule

Change discovery merge behavior so fresh chat discovery does not remove static
image API models.

Discovery remains authoritative for discovered chat models, but it must
preserve existing static models where `supports_image_generation` is true.
When a provider discovery config reports `api: openai-responses`, the merge may
replace stale `openai-responses` chat models, but it keeps curated image API
models such as `openai-images` models.

This keeps regular chat model selection fresh while preventing an image button
from losing its curated image models after `/v1/models` discovery.

## Frontend UX

The composer footer gets one image icon button near the model picker. It uses
the same compact chip visual language as the existing composer controls.

Behavior:

- Closed state shows the selected image model label, or a neutral image icon if
  no route is selected.
- Opening the picker lazy-loads models for available connected providers.
- The list only shows models with `supportsImageGeneration === true`.
- The provider list uses a dedicated image-generation availability filter, not
  `providerIsAvailableForAgent`, so image-capable providers that are not normal
  chat providers can still appear.
- If no connected provider has image models, show a small empty state telling
  the user to connect or configure an image-capable provider.
- Selection is stored per session with `sessionPreferenceKey`.
- The selected route is not a global default and does not mutate normal chat
  model routing.

The regular text model picker remains unchanged. Image model routing and chat
model routing are independent.

## Turn Options

Extend the desktop API shape:

```ts
export type AgentImageGenerationRoute = {
  providerId: string;
  modelId: string;
};

export type AgentTurnOptions = {
  providerId?: string | null;
  modelId?: string | null;
  thinkingOptionId?: string | null;
  fastMode?: boolean;
  permissionMode?: AgentPermissionMode;
  mode?: AgentTurnMode;
  imageGeneration?: AgentImageGenerationRoute | null;
};
```

Daemon parsing validates that both ids are non-empty and canonicalizes the
provider id. A partial `imageGeneration` object is an error, not a silent no-op.
Applying turn options writes a session-local route to `AppState`.

`AppState` gets:

```rust
pub image_generation_route: Option<ImageGenerationRoute>
```

The route is intentionally turn/session state, not persisted provider config.
The frontend persists the UI choice per session; the runtime only needs the
effective route for the active turn.

## Prompt Guidance

The system prompt should include only a concise, factual reminder when a route
is selected:

```text
Selected image generation route: provider/model. When the user asks you to
create an image, call ImageGeneration; the tool will use this route.
```

Do not append hidden instructions to the user message. Do not ask the model to
choose provider/model. The model decides whether the request needs image
generation; the runtime decides which image model is used.

## Tool Input Contract

`ImageGeneration` input changes to:

```json
{
  "prompt": "literal prompt or workspace-relative prompt file",
  "promptReference": "optional workspace-relative reference file",
  "aspect": "square | landscape | portrait | auto | size",
  "outputPath": "workspace-relative PNG path",
  "purpose": "optional caller purpose",
  "retryFromError": "optional previous error payload"
}
```

The tool does not need `model` or `providerId` in normal chat use because the
selected route is already in `AppState`. If direct tool callers need explicit
routing later, that should be a separate design. Removing model/provider from
the tool input keeps the LLM from accidentally overriding the composer choice.

## Tool Execution

`execute_tool_call` already receives `ProviderRegistry` and `AuthStore`, but
the workflow tool dispatcher does not pass them to `ImageGeneration`. Extend the
workflow dispatch path for this tool so it can resolve:

- selected provider descriptor
- selected model descriptor
- provider auth credential
- provider image generation endpoint path
- provider network proxy settings

Execution fails before network access when:

- no image route is selected
- provider is unknown
- provider lacks credentials
- model is unknown
- model lacks `supports_image_generation`
- provider lacks an image generation endpoint

The tool request uses bearer auth for API-key credentials. For the first
implementation, image generation accepts API-key credentials and no-auth
providers only. OpenAI Codex OAuth is rejected for image generation because it
is not the public Images API credential path.

Do not add runtime tool visibility filtering in this change. If no image route
is selected, the tool fails with a clear message before network access. This
keeps the change scoped to routing and execution instead of adding another
tool-advertising policy.

The response parser continues to accept either `data[0].b64_json` or
`data[0].url`. URL downloads remain HTTPS-only.

## Security

- Never expose API keys to the frontend.
- Do not serialize credentials into transcripts, tool inputs, or tool results.
- Keep output path validation workspace-relative.
- Keep network access behind the existing tool approval and network sandbox
  policy.
- Reject unsupported provider/model combinations before sending a request.
- Do not let the model override the composer-selected route.

## Performance

- Image models are loaded lazily when the image picker opens.
- Cache loaded image models per provider in the picker state.
- Avoid scanning every provider on every render.
- The system prompt adds at most one short line.
- Tool execution performs one image API request plus an optional HTTPS download.
- Discovery merge preserves static image models without requiring a second
  discovery pass.

## Testing

Rust:

- `ModelDescriptor` serde defaults `supports_image_generation` to false.
- `ModelDescriptorDto` exposes `supportsTools` and
  `supportsImageGeneration`.
- Discovery merge preserves static image-generation models when chat discovery
  returns only chat models.
- Turn option parsing accepts valid `imageGeneration` and rejects partial
  routes.
- `ImageGeneration` builds the expected URL for OpenAI and BytePlus base URLs.
- `ImageGeneration` rejects missing route, missing credential, and non-image
  model before network execution.
- Response parsing still handles `b64_json` and HTTPS `url`.

Frontend:

- Image picker filters to connected providers with `supportsImageGeneration`.
- Session-local route persists and restores.
- `composerOptions()` includes `imageGeneration` when selected and omits it
  when unset.
- Empty state renders when no image-capable models are available.

Manual:

- Select an image model, ask chat to generate an image, approve the tool call,
  and verify a workspace image file plus an image-generation tool card.
- Switch sessions and verify image route selection is session-local.

## File-Level Changes

- `crates/puffer-provider-registry/src/model.rs`
  - Add `supports_image_generation` to `ModelDescriptor`.
  - Add provider-level image generation endpoint metadata.
- `crates/puffer-provider-registry/src/discovery.rs`
  - Preserve static image-generation models across chat discovery merges.
- `crates/puffer-cli/src/desktop_api_types.rs`
  - Expose computed `supports_tools` and `supports_image_generation`.
- `crates/puffer-cli/src/daemon.rs`
  - Parse and apply image generation turn options.
- `crates/puffer-core/state.rs`
  - Store the active image generation route.
- `crates/puffer-core/runtime/system_prompt.rs`
  - Render the concise selected-route guidance.
- `crates/puffer-core/runtime/claude_tools/mod.rs`
  - Pass provider/auth context to the image generation workflow.
- `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
  - Resolve route, endpoint, credential, and model through provider registry
    and auth store.
- `resources/providers/openai.yaml`
  - Add image generation endpoint metadata and curated image models.
- `resources/providers/byteplus.yaml`
  - Add image generation endpoint metadata and curated Seedream image models
    when known.
- `resources/tools/image_generation.yaml`
  - Update description to mention selected chat image route and remove env
    backend wording.
- `apps/puffer-desktop/src/lib/api/desktop.ts`
  - Add image route option and image capability DTO fields.
- `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
  - Add image button, session-local route state, and `composerOptions` wiring.
- `apps/puffer-desktop/src/lib/screens/agent/ModelPicker.svelte`
  - Add minimal `providerFilter`, `modelFilter`, and image-specific trigger
    label/icon props. The default filters remain the current agent-provider and
    agent-tool checks, so normal chat model picking stays unchanged while image
    picking reuses provider loading and menu state.
- `apps/puffer-desktop/src/lib/design/Icon.svelte`
  - Add a lucide image icon.

## Design Boundaries

This is a routing feature, not a media platform. The stable boundary is:

- Provider registry owns image-generation support and endpoint metadata.
- Composer owns the user's selected route.
- AppState owns the effective route for the active turn.
- System prompt informs the model that image generation is available.
- `ImageGeneration` owns request validation, provider resolution, network I/O,
  and workspace file output.

Anything beyond that boundary, such as image editing or asset libraries, should
be designed separately after this route is proven.
