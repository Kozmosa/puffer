# Image Generation Planner Batch Design

- Date: 2026-06-07
- Status: Approved design, pending implementation
- Scope: Count-aware image generation planning for all current image providers

## Summary

Puffer should route every image generation request through one deterministic
planner before any provider adapter executes. The planner decides whether a
multi-image request is split into single-image calls or sent through a verified
exact batch path.

The long-term default is stability:

- Every model uses `per_image` execution unless its descriptor explicitly opts
  into exact batching.
- Exact batching is allowed only when the provider/model is known to return the
  requested number of images for each batch call.
- Provider responses are normalized into one ordered artifact list before the
  `ImageGeneration` tool or desktop UI sees the result.

This design intentionally removes the old implicit behavior where omitting a
batch limit meant the `images_json` adapter could send one request for all
requested images.

## Context

Current image generation providers use three execution shapes:

- `images_json`: OpenAI-compatible image endpoints used by OpenAI, xAI, Zhipu,
  BytePlus, and many Vercel AI Gateway models.
- `minimax_image`: MiniMax native image endpoint.
- `chat_image_output`: chat-completion output images used by OpenRouter and
  Vercel AI Gateway discovery models.

The unstable path is `images_json`. Some models accept an `n` field and return
exactly that many images. Others accept only stable single-image calls. BytePlus
Seedream exposes `sequential_image_generation=auto`, but that mode can return
fewer images than requested unless provider-specific options are supported and
verified. Treating all of these as the same batch contract is incorrect.

## Goals

- Make requested image count fulfillment exact and predictable.
- Support every current image provider through one planning model.
- Keep new provider onboarding simple: default to stable single-image calls.
- Preserve performance by allowing explicit exact batch mode for verified
  models.
- Keep adapter code focused on request/response conversion, not count planning.
- Keep the public result contract as one job with ordered `artifacts[]`.
- Avoid provider-specific fallback behavior that hides descriptor bugs.
- Keep the first implementation serial to avoid concurrency, rollback, and
  rate-limit complexity.

## Non-Goals

- Do not preserve the old top-level `max_images_per_call` behavior.
- Do not dynamically probe provider batch behavior at runtime.
- Do not add automatic fallback from exact batch to single-image calls.
- Do not add a generic retry framework.
- Do not expand requested image count beyond the existing `1..=4` range.
- Do not add nested provider parameter support solely for speculative batch
  modes.
- Do not introduce a database, queue, or media gallery.
- Do not add per-provider concurrency controls in the first implementation.

## Batch Descriptor

Replace the implicit top-level batch limit with an explicit batch policy under
the execution descriptor:

```yaml
execution:
  adapter: images_json
  path: /images/generations
  batch:
    mode: per_image
```

High-performance exact batch mode is opt-in:

```yaml
execution:
  adapter: images_json
  path: /images/generations
  batch:
    mode: exact
    max_images_per_call: 4
```

Rules:

- `per_image` means a request for `N` images creates `N` one-image calls.
- `exact` means the provider/model may receive batch calls, but each response
  must contain at least the requested number for that call.
- Missing `batch` resolves to `per_image` at runtime for safety.
- Bundled provider governance should require image executions to declare
  `batch.mode` explicitly so resource files document intent.
- `exact` requires `max_images_per_call >= 2`.
- `per_image` must not carry `max_images_per_call`.
- `max_concurrency` is intentionally not part of the descriptor. The first
  implementation executes plan calls serially; concurrency can be added later
  only after measuring a concrete performance gap.
- BytePlus Seedream, Zhipu, MiniMax, and chat-output models should begin as
  `per_image`.
- All bundled providers should begin as `per_image`. OpenAI, xAI, or Vercel
  models may move to `exact` only in a later resource-only change after focused
  tests or live verification prove their endpoint returns exact counts.

## Planner

Add an image generation planner that owns count splitting for all adapters:

```rust
struct ImageGenerationPlan {
    calls: Vec<ImageCallPlan>,
}

struct ImageCallPlan {
    call_index: usize,
    requested_count: u8,
}
```

Planning rules:

- `per_image`, `count=4` produces `[1, 1, 1, 1]`.
- `exact`, `max_images_per_call=2`, `count=4` produces `[2, 2]`.
- `exact`, `max_images_per_call=3`, `count=4` produces `[3, 1]`.
- Artifact indexes are assigned by plan order, not completion order.
- The final successful result must contain exactly `requested_count` artifacts.

The planner should be image-specific. It is not a generic batch framework.

## Execution Flow

The image runtime should follow one flow for every adapter:

1. Validate `count` in the existing `1..=4` range.
2. Resolve provider/model/adapter capability and batch descriptor.
3. Build an `ImageGenerationPlan`.
4. Create one media job for the user request.
5. Execute plan calls serially through the selected adapter.
6. Normalize each provider response into image outputs.
7. Persist outputs only after all plan calls have succeeded.
8. Succeed only when the final artifact count equals the requested count.
9. Return one result containing `jobId`, `requestedCount`, and `artifacts[]`.

Adapters should use the shared planner after resolving the execution descriptor.
They should not implement bespoke count splitting or infer batch behavior from
missing descriptor fields.

## Adapter Responsibilities

`images_json`:

- In `per_image`, send one-image requests and omit `n`.
- In `exact`, send the call plan count using `n` when the adapter shape
  supports it.
- Fail a call if `data[]` contains fewer images than the call requested.
- If the provider returns more images than requested, take only the planned
  count.

`minimax_image`:

- Remove adapter-local count looping.
- Execute one call plan at a time through the existing single-image request.
- Start with `per_image`.

`chat_image_output`:

- Remove adapter-local count looping.
- Execute one call plan at a time.
- If one chat response contains multiple image outputs, keep only the number
  required by the call plan.
- Start with `per_image`.

## Error Handling

- A provider response with fewer images than planned fails the call.
- A failed call fails the job. Do not silently retry through another mode.
- The tool response should not expose partial artifacts for failed jobs.
- The first implementation should not write artifact sidecars until every
  planned provider call has succeeded. This avoids partial artifacts and
  rollback logic.
- Error messages should include the call index and expected/actual counts.

Example:

```text
image generation returned 1 image(s), expected 2 for call 0
```

## Performance

The first implementation gets performance from exact batching only. `per_image`
mode is serial because the user-facing count is capped at four and serial calls
avoid rate-limit, cancellation, ordering, and partial-persistence complexity.

If real usage shows serial `per_image` is too slow, add bounded concurrency as a
separate follow-up after the planner and exact-batch semantics are stable.

## Testing

Planner tests:

- `per_image count=4 -> [1,1,1,1]`.
- `exact max=2 count=4 -> [2,2]`.
- `exact max=3 count=4 -> [3,1]`.
- Missing batch descriptor resolves to `per_image`.
- Planner output contains no concurrency policy.

Adapter tests:

- `images_json per_image` sends multiple requests and omits `n`.
- `images_json exact` sends `n` for batch calls.
- Under-production fails the call and job.
- Over-production is truncated to the planned count.
- Failed multi-call generation writes no artifact sidecars.
- `minimax_image` and `chat_image_output` use planner calls instead of local
  count loops.

Workflow tests:

- `ImageGeneration` returns `artifacts.len() == requestedCount` on success.
- BytePlus Seedream descriptors use `per_image`.
- Zhipu descriptors use `per_image`.
- Vercel and OpenRouter chat-output descriptors use `per_image`.

Governance tests:

- Every bundled image execution declares `batch.mode`.
- `exact` requires `max_images_per_call >= 2`.
- `per_image` rejects `max_images_per_call`.
- The old top-level `max_images_per_call` field is no longer accepted.

## Implementation Boundaries

The first implementation should be intentionally small:

- Add batch descriptor parsing and validation.
- Add the planner.
- Route all current adapters through shared planner calls without adding a new
  generic adapter trait or execution runner.
- Convert current provider resources to explicit `per_image`.
- Add `exact` support against local fake-provider tests only. Do not mark any
  bundled real provider as `exact` in the first implementation.

Do not add provider-specific nested request parameters, runtime probes, learned
capability caches, automatic fallback, or bounded concurrency in this change.
