# Stable Multi Image Generation Design

- Date: 2026-06-07
- Status: Approved design, pending implementation plan
- Scope: Stable multi-image `ImageGeneration` planning, execution, and result
  semantics

## Summary

Puffer should treat "generate N images" as one logical media generation request.
The model should call `ImageGeneration` once with a typed `count`, the runtime
should create one media job, and every generated file should become one
artifact in that job's `artifacts[]`.

The long-term priority is stability over latency:

- one user image request maps to one `jobId`;
- one generated file maps to one `artifactId`;
- provider calls may be split serially under the job;
- any produced artifact is preserved;
- only zero produced images is a generation failure.

This design supersedes any earlier exact-count success requirement in the
multi-image planning work. A request for two images that produces one image is a
partial success, not a total failure.

This is a corrective design for the already-started multi-artifact work. It is
not a second batch framework and should not restart the implementation from
scratch.

## Context

The current implementation already has much of the right internal shape:

- `ImageGenerationInput` accepts `count`.
- `ExactImageGenerationRequest` carries `count`.
- `MediaJob` stores `requested_count`.
- image generation results expose `artifacts[]`.
- the planner can split a count into provider calls.

The observed Milhous failure mode is at the model-facing boundary: the tool
description still says "Generate one image", so the model can choose two
`ImageGeneration(count=1)` calls for a "two images" request. That creates two
jobs instead of one job with two artifacts.

The second instability is execution semantics. Some adapters still fail the
whole job when the provider returns fewer images than requested. That discards
valid generated media and trains the model to avoid `count > 1` by making
multiple single-image calls.

There is also one provider-specific shortcut that should not survive this
corrective pass: the Images JSON adapter currently changes
`sequential_image_generation=disabled` to `auto` when a multi-image exact batch
call is planned. That is a BytePlus sequence-mode optimization hidden inside a
generic adapter. It conflicts with descriptor-driven planning and should be
removed unless a provider descriptor explicitly opts into that mode later.

## Goals

- Make multi-image user intent produce one logical `ImageGeneration` call.
- Keep the durable identity model simple and inspectable.
- Preserve every successfully generated artifact.
- Keep provider execution serial in the first implementation.
- Keep provider batch behavior descriptor-driven.
- Preserve descriptor-selected provider parameters without count-based mutation.
- Keep artifact preview and desktop attachment rendering artifact-scoped.
- Make tool descriptions strong enough that models prefer `count`.
- Remove exact-count failure behavior for partial output.
- Avoid compatibility shims for old single-artifact result shapes.

## Non-Goals

- No gallery, collection artifact, database, object storage, or queue.
- No runtime heuristic that merges separate tool calls.
- No parallel provider execution in the first implementation.
- No generic batch framework outside image generation.
- No automatic fallback from exact batch mode to per-image mode.
- No retry UI or per-artifact retry orchestration.
- No provider-native BytePlus sequence mode as the default path.
- No count-driven parameter rewrite such as forcing
  `sequential_image_generation=auto`.
- No pre-created success JSON or synthetic artifact metadata.
- No provider-registry schema migration solely to reject older descriptors.
- No warning/status sub-model for partial generation in the first pass.

## Recommended Approach

Use one logical request with serial provider execution.

For a user request such as "generate two images", the model-facing contract is:

```json
{
  "prompt": "two mobile suits fighting in orbit",
  "count": 2,
  "purpose": "generate two images"
}
```

The runtime then:

1. validates `count`;
2. creates one `MediaJob` with `requested_count = count`;
3. builds a provider call plan from the descriptor;
4. executes plan calls serially;
5. persists each successful image as one artifact;
6. marks the job succeeded when at least one artifact exists;
7. returns one result containing `jobId`, `requestedCount`, and `artifacts[]`.

The execution plan is internal. The result JSON is written only after real
files and sidecars exist.

## Tool Contract

The `ImageGeneration` tool description must describe one or more images:

- say "Generate one or more images in one logical request";
- state that `count` is the number of images requested;
- state that multi-image user requests should use one call with `count=N`;
- state that callers must not split a single multi-image user request into
  multiple `count=1` tool calls.

The system prompt should repeat the operational rule because it is a recurring
model-planning failure:

```text
When the user asks for multiple images from one prompt, call ImageGeneration
once with count set to the requested number. Do not issue multiple
ImageGeneration calls for that single request unless the user asks for separate
prompts or separate jobs.
```

`count` should remain a typed request field with range `1..=4`. It should be
required in the tool schema and Rust input. Single-image requests must pass
`count: 1` explicitly. This intentionally drops compatibility for old
tool-call payloads that omit `count`.

## Runtime Semantics

The public result contract is:

```json
{
  "jobId": "job-1",
  "requestedCount": 2,
  "artifacts": [
    {
      "artifactId": "artifact-1",
      "index": 0,
      "path": "/workspace/.puffer/media/images/artifact-1/image.jpeg",
      "mimeType": "image/jpeg",
      "size": 123
    }
  ],
  "status": "succeeded"
}
```

Rules:

- `requestedCount` is the user's requested count.
- `artifacts.length` is the produced count.
- `artifact.index` is stable output order within the job.
- `artifactId` always identifies exactly one persisted file.
- `jobId` is the only grouping identity for multi-image output.
- old top-level `artifactId` and `path` fields are removed.

No component should infer success by comparing produced count with requested
count. Consumers should render whatever `artifacts[]` contains and may display
a shortfall only as auxiliary metadata.

## Provider Planning

Keep the existing descriptor idea:

```yaml
batch:
  mode: per_image
```

Planning rules:

- `per_image` with `count=3` creates `[1, 1, 1]`.
- `exact` with `max_images_per_call=2` and `count=3` creates `[2, 1]`.
- bundled provider resources must explicitly declare `batch.mode`.
- existing Rust defaults may remain as internal test and deserialization
  defaults; do not expand this change into a provider-registry schema migration.
- BytePlus remains `per_image` by default.
- provider request parameters come from descriptor defaults and user settings;
  the generic planner must not mutate them based on `count`.

This design intentionally does not optimize BytePlus through
`sequential_image_generation=auto` first. That path can be evaluated later as a
provider descriptor change after live verification proves exact and stable
output behavior.

## Adapter Semantics

All image adapters should return a vector of normalized image outputs and let
the job layer persist produced outputs.

For each adapter:

- execute plan calls serially;
- append successful outputs in plan order;
- stop on the first provider error or malformed response;
- persist all outputs already collected;
- fail only if no outputs were collected;
- if one call returns more images than requested, keep only the planned count;
- if one call returns fewer images than requested, keep what it returned and
  stop the plan.

This gives a stable and explainable contract:

```text
zero outputs -> failed job and tool error
some outputs -> succeeded job with fewer artifacts
all outputs -> succeeded job with requested number of artifacts
```

## Error And Status Model

Use the existing job states:

- `Failed`: no image artifacts were produced.
- `Succeeded`: at least one image artifact was produced.

Do not add a new `Partial` status. The partial condition is derived:

```text
job.status == Succeeded && job.produced_count() < job.requested_count
```

Do not add a warning field in the first implementation. Consumers that need to
surface a shortfall can derive it by comparing `artifacts.length` with
`requestedCount`.

## Desktop And Session Behavior

The desktop timeline should synthesize one assistant item per successful
`ImageGeneration` tool result. That item contains one generated-media attachment
per artifact.

For direct `/image` or media generation requests, the live preview should use
the same result shape and create one assistant preview item with all generated
attachments.

Session history should remain factual:

- one tool invocation for one logical image request;
- one output JSON after generation completes;
- no placeholder artifact entries before files exist.

## Testing Strategy

Add or update tests at the behavior boundary first:

- tool definition says one-or-more images and instructs use of `count`;
- `ImageGeneration` with `count=2` returns one `jobId` and two artifacts;
- `per_image` provider planning uses multiple provider calls under one job;
- partial provider output returns succeeded result with fewer artifacts;
- zero provider output returns a tool error and failed job;
- desktop/session parsing renders all artifacts from one tool result;
- schema rejects `count=0` and `count>4`.

Do not add broad snapshot tests for full prompts. Test the exact guidance that
prevents the observed failure.

## Rejected Alternatives

### Prebuilding `artifacts[]` Before Generation

Rejected because artifact metadata is factual persistence state. Creating
`artifactId`, `path`, `mimeType`, or `size` before writing real files would make
session history untrustworthy and complicate failure cleanup.

### Runtime Tool-Call Merger

Rejected for now. Merging multiple `ImageGeneration(count=1)` calls by prompt
similarity is heuristic and risks combining separate user intents. Strong tool
schema and prompt guidance are simpler and easier to test. Revisit only if logs
show the model still frequently violates the contract.

### Parallel Provider Calls

Rejected for the first implementation. Parallelism improves latency but adds
ordering, cancellation, rate-limit, and partial failure complexity. Serial
execution is sufficient for the current `1..=4` range and preserves stable
artifact order.

### Provider-Native BytePlus Sequence Mode By Default

Rejected as the default. BytePlus sequence mode is promising but provider
specific. Keeping BytePlus as `per_image` gives predictable one-image calls
under one Puffer job. Exact or sequence batching can be enabled later by
descriptor after focused verification.

## Implementation Boundaries

The implementation should stay in these existing areas:

- `resources/tools/image_generation.yaml`
- `crates/puffer-core/runtime/system_prompt.rs`
- `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
- `crates/puffer-core/runtime/media/planner.rs`
- image adapters under `crates/puffer-core/runtime/media/`
- desktop and daemon result parsing already touched by multi-artifact work

Avoid new crates, new service layers, and new runtime normalization passes.
Do not modify provider-registry descriptor parsing unless an existing test
breaks directly because of the `batch.mode` resource declarations.
Remove generic count-based Images JSON parameter mutation instead of expanding
it into a provider capability system.

## Plan Review Findings

The earlier planner-batch design contained exact-count success language that no
longer matches the stability-first goal:

- "The final successful result must contain exactly `requested_count`
  artifacts."
- "Persist outputs only after all plan calls have succeeded."
- "The tool response should not expose partial artifacts for failed jobs."

Those statements are superseded by this design. The execution plan should update
tests that assert all-or-nothing persistence instead of adding compatibility
branches around them.
