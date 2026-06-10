# BytePlus Video References Design

## Context

Puffer's current video generation tool is prompt-only. The BytePlus video
adapter already uses the ModelArk task API shape, but it hardcodes
`content` to a single text item. Relaydance is configured through a separate
OpenAI-style video endpoint and currently builds only `prompt`, `seconds`,
and `metadata`.

BytePlus Seedance supports video generation from text plus image references.
VPL and real-human assets use the same content path as image-to-video, with
the image URL set to an approved `asset://...` reference instead of a public
HTTPS URL. Public image-to-video uses the same `image_url` item shape with an
HTTPS URL.

The design does not preserve the prompt-only input schema for compatibility.
It optimizes for clear ownership, stable validation, and future staging
support without turning the media runtime into a generic multimodal framework.

## Recheck Outcome

The initial design over-modeled the problem with generic `kind`, `source`,
`role`, and `local_path` fields. The confirmed need is narrower: BytePlus video
generation needs ordered image reference URLs. The updated design keeps only
`imageReferences: string[]`, where each value is a provider-usable
`https://...` or `asset://...` reference.

Local image paths are intentionally excluded from the first implementation.
They require a separate staging subsystem that turns a local file into a public
HTTPS URL or an approved `asset://...` reference before video generation. An
input field that always fails would be API surface without working behavior.

## Goals

- Add image references for BytePlus video generation.
- Support public HTTPS image URLs and BytePlus `asset://` references in the
  first implementation.
- Keep local image support out of the generation request until a staging
  backend can produce a provider-fetchable HTTPS URL or `asset://` URL.
- Keep Relaydance prompt-only until Relaydance documents or proves support for
  reference assets.
- Keep image references separate from model parameters.
- Avoid downloading public images or inlining base64 data in generation
  requests.

## Non-Goals

- No generic cross-provider multimodal media framework.
- No Relaydance reference support without provider-specific documentation or a
  successful probe.
- No base64 image submission in video generation requests.
- No first implementation of object storage or asset staging.
- No video or audio references until a provider capability requires them.
- No first-frame or last-frame roles until a concrete workflow asks for them.
- No local file path in the video-generation tool schema for this change.

## Input Model

Replace the current prompt-only tool input with a minimal image-reference
field:

```rust
pub struct VideoGenerationInput {
    pub prompt: String,
    pub image_references: Vec<String>,
    pub parameters: BTreeMap<String, String>,
    pub purpose: Option<String>,
}
```

The runtime boundary should carry the same field so adapters do not smuggle
reference data through `parameters`:

```rust
pub struct ExactMediaGenerationRequest {
    pub kind: String,
    pub provider_id: String,
    pub model_id: String,
    pub operation: String,
    pub adapter: String,
    pub prompt: String,
    pub image_references: Vec<String>,
    pub parameters: BTreeMap<String, String>,
    pub count: u8,
}
```

The public JSON shape should be explicit and provider-neutral at the tool
boundary:

```json
{
  "prompt": "Make image 1 wave at the camera.",
  "imageReferences": [
    "https://example.com/person.jpg",
    "asset://approved-person-asset-id"
  ],
  "parameters": {
    "duration_seconds": "5",
    "resolution": "720p",
    "aspect_ratio": "9:16"
  }
}
```

Asset-backed VPL uses the same field with a value such as
`asset://approved-person-asset-id`.

Reference order is meaningful. Prompts refer to `image 1`, `image 2`, and
similar indexes, and those indexes map to `imageReferences[0]`,
`imageReferences[1]`, and so on.

This is intentionally smaller than a generic `references` object. It avoids
unused `kind`, `source`, and `role` enums while preserving a clear future path:
add a new field or enum only when a supported workflow needs video, audio,
first-frame, or last-frame inputs.

## Image Reference Validation

Add a small validation function before provider request construction:

```rust
pub(crate) fn validate_video_image_references(values: &[String]) -> Result<Vec<String>>
```

Validation rules:

- HTTPS references
  - Must use the `https` scheme.
  - Must parse as a URL.
  - Is not fetched, downloaded, or probed.
- Asset references
  - Must use the `asset://` scheme.
  - Is not interpreted beyond scheme validation.
  - Provider-side ownership and review failures remain provider errors.
- Other values
  - Are rejected with:
    `VideoGeneration imageReferences[N] must be an https:// or asset:// URL`

Local paths are intentionally not accepted here. A later staging feature should
turn local files into HTTPS or `asset://` references before calling
VideoGeneration.

## BytePlus Request Construction

Change `BytePlusVideoRequest` to hold image references alongside the prompt:

```rust
pub struct BytePlusVideoRequest {
    pub model: String,
    pub prompt: String,
    pub image_references: Vec<String>,
    pub params: Vec<(String, Value)>,
}
```

For text-only generation, build the same single text item as today:

```json
{
  "type": "text",
  "text": "A cinematic city at sunrise"
}
```

For image references, append one `image_url` item per validated reference:

```json
{
  "type": "image_url",
  "image_url": {
    "url": "https://example.com/person.jpg"
  }
}
```

VPL uses the same item shape with `asset://...` as the URL:

```json
{
  "type": "image_url",
  "image_url": {
    "url": "asset://approved-person-asset-id"
  }
}
```

The BytePlus request body remains:

```json
{
  "model": "...",
  "content": [
    { "type": "text", "text": "..." },
    {
      "type": "image_url",
      "image_url": { "url": "https://... or asset://..." }
    }
  ],
  "duration": 5,
  "ratio": "9:16",
  "resolution": "720p",
  "generate_audio": false
}
```

`generate_audio` should keep the current conservative default of `false`,
with provider parameters allowed to override it later if the descriptor
declares that parameter.

## Relaydance Behavior

Relaydance remains prompt-only. If the selected provider is Relaydance and
`image_references` is non-empty, request building fails before any HTTP call:

```text
provider relaydance does not support video image references
```

This avoids silently dropping references and avoids assuming Relaydance shares
BytePlus asset namespaces or content semantics.

## Error Handling

Errors should fail as early as possible:

- Empty prompt: tool input validation.
- Unknown fields: deserialization validation.
- Invalid HTTPS URL or `asset://` URL: reference validation.
- Unsupported provider reference capability: provider request building.
- BytePlus moderation or generation failure: provider response handling with
  provider, model, adapter, and task context.

Provider errors should preserve the provider message while adding enough
Puffer context for debugging.

## Performance and Stability

- Do not download public image URLs for validation.
- Do not inline local files as base64.
- Keep local file staging outside the video-generation tool and provider
  adapter.
- Keep request construction deterministic and allocation-light.
- Cache staged local files in a future staging backend by file identity and
  content digest, not by prompt.
- Keep reference ordering stable because prompts refer to `image 1`,
  `image 2`, and similar indexes.

## Tests

Unit coverage:

- `VideoGenerationInput` accepts `imageReferences`.
- Unknown input fields are rejected.
- Empty prompt is rejected.
- HTTPS URL references validate without network access.
- HTTP URL references are rejected.
- `asset://` references validate.
- Local-looking paths are rejected by the image-reference URL validator.
- BytePlus text-only requests still produce one text content item.
- BytePlus public image references serialize to `content[].image_url.url`.
- BytePlus asset references serialize to `asset://...`.
- BytePlus numeric parameters such as `duration` remain numeric.
- Relaydance rejects non-empty `imageReferences` before submission.
- Relaydance prompt-only request bodies remain unchanged.

Integration-style fake HTTP coverage:

- BytePlus submit test server receives the expected `content[]`.
- Polling and download continue to use the existing video job flow.
- Failed provider responses include provider, adapter, and task context.

No real BytePlus or Relaydance calls are required for this change.

## Rollout Order

1. Add `imageReferences` to the video generation tool schema and CLI boundary.
2. Add URL validation for HTTPS and `asset://` image references.
3. Add `image_references` to `ExactMediaGenerationRequest`.
4. Change BytePlus request construction to append `image_url` content items.
5. Reject `imageReferences` for Relaydance.
6. Update the video-generation skill text so agents know BytePlus supports
   public image URLs and `asset://` references while local paths require
   staging outside this tool.
7. Add focused unit and fake HTTP tests.
8. Add component update specs for the touched crates.

The first implementation should support public HTTPS and asset references end
to end. Local image upload should be a separate staging design and should not
be represented as a half-supported video-generation input.
