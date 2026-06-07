# Stable Multi Image Generation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make one multi-image user request execute as one `ImageGeneration(count=N)` job that preserves any successfully generated artifacts.

**Architecture:** Keep the existing `jobId` plus `artifacts[]` result model. Strengthen the model-facing tool contract so multi-image requests use one tool call, then update adapters to treat partial provider output as a succeeded job with fewer artifacts. Do not add parallelism, runtime tool-call merging, new status models, or provider-registry schema migrations.

**Tech Stack:** Rust (`puffer-core`, `puffer-resources`), YAML resource tools, Cargo tests.

---

## Recheck Outcome

The existing multi-artifact implementation already covers these foundations:

- `MediaJob` has `requested_count` and `produced_count()`.
- `ExactImageGenerationRequest` has `count`.
- `ExactImageGenerationResult` returns `artifacts[]`.
- Desktop and daemon parsing already understand multi-artifact results.
- Provider descriptors already use image `batch.mode`.

This plan only implements the remaining corrections:

- Model-visible `ImageGeneration` contract still reads as single-image.
- Tool input still defaults missing `count` to `1`.
- `images_json`, `chat_image_output`, and `minimax_image` still fail whole jobs
  when some images were already produced.
- `images_json` still mutates `sequential_image_generation=disabled` to `auto`
  for multi-image exact batches, which is a provider-specific optimization in a
  generic adapter.
- Old all-or-nothing tests must be replaced, not kept with compatibility
  branches.

## File Structure

- Modify `resources/tools/image_generation.yaml`
  - Update description to one-or-more images.
  - Make `count` required.

- Modify `crates/puffer-resources/src/loader.rs`
  - Add a narrow resource-loading test for the bundled `ImageGeneration` tool
    description and required `count`.

- Modify `crates/puffer-core/runtime/system_prompt.rs`
  - Add one explicit multi-image `ImageGeneration(count=N)` rule.
  - Extend the existing image-generation prompt test.

- Modify `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
  - Remove the `count` serde default.
  - Delete `default_image_count`.
  - Add tests proving count is required.
  - Update existing tests and tool invocations to pass `count: 1`.

- Modify `crates/puffer-core/runtime/media/planner.rs`
  - Reject image generation counts outside `1..=4`.

- Modify `crates/puffer-core/runtime/media/images_json.rs`
  - Remove count-based provider parameter mutation.
  - Allow shorter `data[]` responses to produce partial success.
  - Preserve earlier outputs when a later planned call returns an error.
  - Fail only when no image output was collected.

- Modify `crates/puffer-core/runtime/media/images_json_tests.rs`
  - Replace all-or-nothing tests with partial-success tests.
  - Keep over-production truncation and exact-batch request tests.

- Modify `crates/puffer-core/runtime/media/chat_image_output.rs`
  - Allow responses with no image outputs to return an empty output vector.
  - Preserve earlier outputs if a later planned call returns no images.

- Modify `crates/puffer-core/runtime/media/minimax_image.rs`
  - Preserve earlier outputs if a later planned call fails.
  - Fail only when no image output was collected.

## Task 1: Model-Facing Count Contract

**Files:**
- Modify: `crates/puffer-resources/src/loader.rs`
- Modify: `resources/tools/image_generation.yaml`
- Modify: `crates/puffer-core/runtime/system_prompt.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/image_generation/tests/image_generation_tool_tests.rs`
- Modify: `crates/puffer-core/runtime/media/planner.rs`

- [ ] **Step 1: Write the failing bundled tool resource test**

Add this test in `crates/puffer-resources/src/loader.rs` next to `load_tool_resources_reads_tools_without_scanning_skills`:

```rust
#[test]
fn bundled_image_generation_tool_requires_count_and_describes_multi_image_use() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    let paths = ConfigPaths::discover(&root);

    let loaded = load_tool_resources(&paths, &FsTestRunner).unwrap();
    let tool = loaded
        .tools
        .iter()
        .find(|tool| tool.value.id == "ImageGeneration")
        .expect("ImageGeneration tool");

    assert!(tool
        .value
        .description
        .contains("Generate one or more images"));
    assert!(tool.value.description.contains("count"));
    assert!(!tool.value.description.contains("Generate one image"));

    let schema = tool.value.input_schema.as_ref().expect("input schema");
    let required = schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .expect("required array");
    assert!(required.iter().any(|value| value == "prompt"));
    assert!(required.iter().any(|value| value == "count"));
}
```

- [ ] **Step 2: Run the failing bundled tool resource test**

Run:

```bash
cargo test -p puffer-resources bundled_image_generation_tool_requires_count_and_describes_multi_image_use -- --nocapture
```

Expected: FAIL because the bundled tool description still says `Generate one image` and `count` is not required.

- [ ] **Step 3: Update the bundled `ImageGeneration` tool resource**

Update `resources/tools/image_generation.yaml` description to:

```yaml
description: |-
  Generate one or more images through Puffer's configured image media settings
  and write the resulting image artifacts into the workspace image folder.

  Use one ImageGeneration call for one logical image-generation request. When
  the user asks for multiple images from one prompt, set count to the requested
  number instead of issuing multiple count=1 calls. The tool reads a
  workspace-relative prompt file when the prompt value names one; otherwise it
  treats prompt as literal text. The result includes one media job id and one
  artifact entry per persisted image file.
```

Update the required fields to:

```yaml
required:
  - prompt
  - count
```

- [ ] **Step 4: Verify the bundled tool resource test passes**

Run:

```bash
cargo test -p puffer-resources bundled_image_generation_tool_requires_count_and_describes_multi_image_use -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Write the failing system prompt test assertion**

Extend `runtime_system_prompt_forbids_fabricating_image_substitutes` in `crates/puffer-core/runtime/system_prompt.rs`:

```rust
assert!(prompt.contains("call ImageGeneration once with count"));
assert!(prompt.contains("Do not issue multiple ImageGeneration calls"));
```

- [ ] **Step 6: Run the failing system prompt test**

Run:

```bash
cargo test -p puffer-core runtime_system_prompt_forbids_fabricating_image_substitutes -- --nocapture
```

Expected: FAIL because the prompt does not yet contain the multi-image rule.

- [ ] **Step 7: Add the system prompt multi-image rule**

In `crates/puffer-core/runtime/system_prompt.rs`, replace the current image-generation bullet with:

```text
 - When the user asks you to create or generate an image, use the ImageGeneration tool. If the user asks for multiple images from one prompt, call ImageGeneration once with count set to the requested number. Do not issue multiple ImageGeneration calls for that single request unless the user asks for separate prompts or separate jobs. If image generation fails, or you believe it will fail, report that plainly to the user. Never hand-author an SVG, ASCII art, or any placeholder file and present it as if it were a generated image. A failed or unavailable image generation is something to report, not a cue to improvise a substitute.
```

- [ ] **Step 8: Verify the system prompt test passes**

Run:

```bash
cargo test -p puffer-core runtime_system_prompt_forbids_fabricating_image_substitutes -- --nocapture
```

Expected: PASS.

- [ ] **Step 9: Write the failing explicit count parsing tests**

In `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`, add these tests near `parses_prompt_reference_from_tool_input`:

```rust
#[test]
fn parses_explicit_image_generation_count() {
    let parsed: ImageGenerationInput = serde_json::from_value(json!({
        "prompt": "panel 1 action",
        "promptReference": "prompts.md",
        "count": 2
    }))
    .unwrap();

    assert_eq!(parsed.prompt_reference.as_deref(), Some("prompts.md"));
    assert_eq!(parsed.count, 2);
}

#[test]
fn rejects_missing_image_generation_count() {
    let error = serde_json::from_value::<ImageGenerationInput>(json!({
        "prompt": "panel 1 action"
    }))
    .unwrap_err();

    assert!(error.to_string().contains("missing field `count`"));
}
```

- [ ] **Step 10: Run the failing count parsing test**

Run:

```bash
cargo test -p puffer-core rejects_missing_image_generation_count -- --nocapture
```

Expected: FAIL because `count` currently defaults to `1`.

- [ ] **Step 11: Require count in `ImageGenerationInput`**

In `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`, change:

```rust
#[serde(default = "default_image_count")]
count: u8,
```

to:

```rust
count: u8,
```

Delete:

```rust
fn default_image_count() -> u8 {
    1
}
```

Update existing JSON test inputs in this file and `image_generation_tool_tests.rs` to include explicit `count: 1`. Use this check to find any remaining omissions:

```bash
rg -n 'json!\(\{[^}]+"prompt"' crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation/tests/image_generation_tool_tests.rs
```

Every `execute_image_generation` and `execute_tool` JSON input for `ImageGeneration` should include `"count": 1` unless the test is explicitly checking `count: 2`.

- [ ] **Step 12: Verify model-facing contract tests**

Run:

```bash
cargo test -p puffer-core parses_explicit_image_generation_count -- --nocapture
cargo test -p puffer-core rejects_missing_image_generation_count -- --nocapture
cargo test -p puffer-core image_generation -- --nocapture
```

Expected: PASS.

- [ ] **Step 13: Write the failing planner count range test**

In `crates/puffer-core/runtime/media/planner.rs`, add this test:

```rust
#[test]
fn rejects_requested_count_outside_supported_range() {
    let zero = plan_image_generation(0, &per_image_batch()).unwrap_err();
    let too_many = plan_image_generation(5, &per_image_batch()).unwrap_err();

    assert_eq!(
        zero.to_string(),
        "image generation count must be between 1 and 4"
    );
    assert_eq!(
        too_many.to_string(),
        "image generation count must be between 1 and 4"
    );
}
```

- [ ] **Step 14: Run the failing planner count range test**

Run:

```bash
cargo test -p puffer-core rejects_requested_count_outside_supported_range -- --nocapture
```

Expected: FAIL because `count=5` currently produces a plan.

- [ ] **Step 15: Enforce the planner count upper bound**

In `plan_image_generation`, replace:

```rust
if requested_count == 0 {
```

with:

```rust
if requested_count == 0 || requested_count > 4 {
```

- [ ] **Step 16: Verify the full count contract**

Run:

```bash
cargo test -p puffer-core rejects_requested_count_outside_supported_range -- --nocapture
cargo test -p puffer-core parses_explicit_image_generation_count -- --nocapture
cargo test -p puffer-core rejects_missing_image_generation_count -- --nocapture
cargo test -p puffer-core image_generation -- --nocapture
```

Expected: PASS.

- [ ] **Step 17: Commit model-facing count contract**

```bash
git add resources/tools/image_generation.yaml crates/puffer-resources/src/loader.rs crates/puffer-core/runtime/system_prompt.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation/tests/image_generation_tool_tests.rs crates/puffer-core/runtime/media/planner.rs
git commit -m "fix(media): require image generation count"
```

## Task 2: Images JSON Stable Serial Execution

**Files:**
- Modify: `crates/puffer-core/runtime/media/images_json.rs`
- Modify: `crates/puffer-core/runtime/media/images_json_tests.rs`

- [ ] **Step 1: Replace the count-based sequence-mode test**

In `crates/puffer-core/runtime/media/images_json_tests.rs`, rename `request_body_enables_sequential_generation_when_requesting_multiple_images` to:

```rust
#[test]
fn request_body_preserves_sequential_generation_descriptor_default() {
```

Keep the exact-batch setup and `count: 2`, but replace the final assertions with:

```rust
let request_text = server.join().unwrap();
assert!(request_text.contains("\"n\":2"));
assert!(request_text.contains("\"sequential_image_generation\":\"disabled\""));
assert_eq!(
    result.artifacts[0].metadata["parameters"]["sequential_image_generation"],
    "disabled"
);
```

- [ ] **Step 2: Run the failing sequence-mode preservation test**

Run:

```bash
cargo test -p puffer-core request_body_preserves_sequential_generation_descriptor_default -- --nocapture
```

Expected: FAIL because `parameters_for_image_count` currently changes `disabled` to `auto` for multi-image exact batches.

- [ ] **Step 3: Replace the later-call failure test with a partial-success test**

In `crates/puffer-core/runtime/media/images_json_tests.rs`, rename `images_json_failed_later_per_image_call_writes_no_artifacts` to:

```rust
#[test]
fn images_json_failed_later_per_image_call_preserves_first_artifact() {
```

In that test's server body, make the first response return one image and the
second response return an HTTP 500 response body such as `rate limited`. Then
replace the execution and assertions with:

```rust
let result = ImagesJsonAdapter::new()
    .unwrap()
    .execute(&registry, &auth_store(), &service, request)
    .expect("partial generation succeeds");

assert_eq!(server.join().unwrap().len(), 2);
assert_eq!(result.job.requested_count, 2);
assert_eq!(result.job.status, crate::runtime::media::MediaJobStatus::Succeeded);
assert_eq!(result.job.produced_count(), 1);
assert_eq!(result.artifacts.len(), 1);
assert_eq!(std::fs::read(&result.artifacts[0].path).unwrap(), b"image");
assert!(service_dir.path().join(".puffer/media/images").exists());
```

- [ ] **Step 4: Replace the exact-batch under-production failure test**

Rename `images_json_fails_when_response_contains_fewer_images_than_requested` to:

```rust
#[test]
fn images_json_exact_batch_underproduction_returns_partial_success() {
```

Replace the execution and assertions with:

```rust
let result = ImagesJsonAdapter::new()
    .unwrap()
    .execute(&registry, &auth_store, &service, request)
    .expect("under-produced exact batch is partial success");

let request_text = server.join().unwrap();
let saved_job = load_single_saved_job(service_dir.path());
assert!(request_text.contains("\"n\":2"));
assert_eq!(saved_job.status, crate::runtime::media::MediaJobStatus::Succeeded);
assert_eq!(saved_job.requested_count, 2);
assert_eq!(saved_job.artifact_ids.len(), 1);
assert_eq!(result.artifacts.len(), 1);
assert_eq!(std::fs::read(&result.artifacts[0].path).unwrap(), b"image-1");
```

- [ ] **Step 5: Add a zero-output failure test**

Add this test in `crates/puffer-core/runtime/media/images_json_tests.rs`:

```rust
#[test]
fn images_json_zero_outputs_fails_without_artifacts() {
    let (base_url, server) = spawn_image_server_with_body(r#"{"data":[]}"#);
    let registry = registry_with_provider_parameters_and_batch(
        "exact-provider",
        base_url,
        image_parameters(),
        exact_batch(4),
    );
    let service_dir = tempfile::tempdir().unwrap();
    let service = MediaGenerationService::new(service_dir.path());
    let mut request = request();
    request.count = 2;

    let error = ImagesJsonAdapter::new()
        .unwrap()
        .execute(&registry, &auth_store(), &service, request)
        .expect_err("zero outputs fail");

    let _request_text = server.join().unwrap();
    let saved_job = load_single_saved_job(service_dir.path());
    assert_eq!(error.to_string(), "image generation produced no images");
    assert_eq!(saved_job.status, crate::runtime::media::MediaJobStatus::Failed);
    assert!(saved_job.artifact_ids.is_empty());
    assert!(!service_dir.path().join(".puffer/media/images").exists());
}
```

- [ ] **Step 6: Run the failing Images JSON tests**

Run:

```bash
cargo test -p puffer-core request_body_preserves_sequential_generation_descriptor_default -- --nocapture
cargo test -p puffer-core images_json_failed_later_per_image_call_preserves_first_artifact -- --nocapture
cargo test -p puffer-core images_json_exact_batch_underproduction_returns_partial_success -- --nocapture
cargo test -p puffer-core images_json_zero_outputs_fails_without_artifacts -- --nocapture
```

Expected: the first three FAIL. The zero-output test should PASS after implementation and may fail before the implementation with the older error message.

- [ ] **Step 7: Remove count-based Images JSON parameter mutation**

In `ImagesJsonAdapter::execute`, replace the current `request_parameters` assignment with a direct descriptor/default resolution:

```rust
let request_parameters =
    selected_parameters_with_defaults(execution.capability.as_ref(), &request.parameters)?;
```

Delete `parameters_for_image_count`. Do not replace it with a provider-specific
capability system in this pass.

- [ ] **Step 8: Let `image_outputs_from_response` return fewer outputs**

In `crates/puffer-core/runtime/media/images_json.rs`, replace the under-production bail in `image_outputs_from_response` with truncation-only collection:

```rust
let Some(items) = value.get("data").and_then(Value::as_array) else {
    bail!("image generation response did not contain an image");
};
let requested_count = count as usize;
let mut outputs = Vec::new();
for item in items.iter().take(requested_count) {
    outputs.push(image_output_from_item(client, item)?);
}
Ok(outputs)
```

Do not bail when `items` is empty or shorter than `requested_count`.

- [ ] **Step 9: Refactor Images JSON per-call handling**

In `ImagesJsonAdapter::request_images`, handle each planned call as a local
`Result<Vec<ImageOutput>>` and append outputs only through one `match`:

```rust
let call_result = (|| -> Result<Vec<ImageOutput>> {
    let body = ImagesJsonRequest::new(
        &request.model_id,
        &request.prompt,
        parameters.clone(),
        call.requested_count,
    )
    .to_body();
    let mut http = self.client.post(url.clone()).json(&body);
    for (name, value) in &provider.headers {
        http = http.header(name.as_str(), value.as_str());
    }
    if let Some(token) = &token {
        http = http.bearer_auth(token);
    }
    let response = http
        .send()
        .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))
        .context("send image generation request")?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| anyhow!("{}", redact_secrets(&error.to_string(), &secrets)))
        .context("read image generation response")?;
    if !status.is_success() {
        bail!(
            "image generation failed with status {}: {}",
            status.as_u16(),
            redact_secrets(&body, &secrets)
        );
    }
    let value: Value =
        serde_json::from_str(&body).context("parse image generation response")?;
    image_outputs_from_response(
        &self.client,
        &value,
        call.requested_count,
        call.call_index,
    )
})();

match call_result {
    Ok(mut call_outputs) => {
        let short_response = call_outputs.len() < call.requested_count as usize;
        outputs.append(&mut call_outputs);
        if short_response {
            break;
        }
    }
    Err(error) => {
        if outputs.is_empty() {
            return Err(error);
        }
        break;
    }
}
```

This keeps the original provider error for zero-output jobs and preserves
already collected outputs for partial jobs. It also stops planned calls after a
short response. Keep the implementation local to `request_images`; do not add a
new result type or warning model.

- [ ] **Step 10: Remove exact-count failure from Images JSON execution**

In `ImagesJsonAdapter::execute`, delete the whole `if outputs.len() != request.count as usize` block. Keep the existing `if artifacts.is_empty()` failure after persistence, because zero produced artifacts is still a failed job.

- [ ] **Step 11: Verify Images JSON stable serial execution**

Run:

```bash
cargo test -p puffer-core request_body_preserves_sequential_generation_descriptor_default -- --nocapture
cargo test -p puffer-core images_json_failed_later_per_image_call_preserves_first_artifact -- --nocapture
cargo test -p puffer-core images_json_exact_batch_underproduction_returns_partial_success -- --nocapture
cargo test -p puffer-core images_json_zero_outputs_fails_without_artifacts -- --nocapture
cargo test -p puffer-core images_json -- --nocapture
```

Expected: PASS.

- [ ] **Step 12: Commit Images JSON stable serial execution**

```bash
git add crates/puffer-core/runtime/media/images_json.rs crates/puffer-core/runtime/media/images_json_tests.rs
git commit -m "fix(media): preserve partial images-json outputs"
```

## Task 3: Chat Image Output Partial Success

**Files:**
- Modify: `crates/puffer-core/runtime/media/chat_image_output.rs`

- [ ] **Step 1: Write empty-response parser test**

Add this test in the `chat_image_output.rs` test module after `chat_image_output_collects_multiple_images`:

```rust
#[test]
fn chat_image_output_returns_empty_outputs_when_response_has_no_images() {
    let value = serde_json::json!({
        "choices": [{
            "message": {
                "content": [{"type": "text", "text": "no image"}]
            }
        }]
    });
    let client = Client::new();

    let outputs = chat_outputs_from_response(&client, &value, 2).unwrap();

    assert!(outputs.is_empty());
}
```

- [ ] **Step 2: Run the failing empty-response parser test**

Run:

```bash
cargo test -p puffer-core chat_image_output_returns_empty_outputs_when_response_has_no_images -- --nocapture
```

Expected: FAIL because the parser currently bails when no image exists.

- [ ] **Step 3: Add partial execution test for chat image output**

Add this test in `crates/puffer-core/runtime/media/chat_image_output.rs`:

```rust
#[test]
fn chat_image_output_later_empty_call_preserves_first_artifact() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().expect("request");
            requests.push(read_http_request(&mut stream));
            let body = if index == 0 {
                json!({
                    "choices": [{
                        "message": {"images": [{"b64_json": "aW1hZ2U="}]}
                    }]
                })
                .to_string()
            } else {
                json!({
                    "choices": [{
                        "message": {"content": [{"type": "text", "text": "no image"}]}
                    }]
                })
                .to_string()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
        }
        requests
    });
    let registry = registry_with_provider(format!("http://{address}"));
    let service_dir = tempdir().expect("tempdir");
    let mut request = request();
    request.count = 2;

    let result = ChatImageOutputAdapter::new()
        .expect("adapter")
        .execute_with_discovery_cache(
            &registry,
            &auth_store(),
            &MediaGenerationService::new(service_dir.path()),
            request,
            &MediaDiscoveryCache::default(),
        )
        .expect("partial generation succeeds");

    assert_eq!(server.join().expect("server").len(), 2);
    assert_eq!(result.job.requested_count, 2);
    assert_eq!(result.job.status, MediaJobStatus::Succeeded);
    assert_eq!(result.job.produced_count(), 1);
    assert_eq!(result.artifacts.len(), 1);
    assert_eq!(std::fs::read(&result.artifacts[0].path).unwrap(), b"image");
}
```

- [ ] **Step 4: Run the failing chat partial execution test**

Run:

```bash
cargo test -p puffer-core chat_image_output_later_empty_call_preserves_first_artifact -- --nocapture
```

Expected: FAIL because the adapter currently fails the job when produced count is less than requested count.

- [ ] **Step 5: Let chat response parsing return empty vectors**

In `chat_outputs_from_response`, remove:

```rust
if outputs.is_empty() {
    bail!("chat image-output response did not contain an image");
}
```

Keep `Ok(outputs)`.

- [ ] **Step 6: Stop chat planned calls after a short response**

In `execute_with_discovery_cache`, replace the short-response error branch:

```rust
if response_outputs.len() < take_count {
    last_error = Some(anyhow::anyhow!(
        "chat image-output returned {} image(s), expected {} for call {}",
        response_outputs.len(),
        take_count,
        call.call_index
    ));
    break;
}
response_outputs.truncate(take_count);
outputs.append(&mut response_outputs);
```

with:

```rust
let short_response = response_outputs.len() < take_count;
response_outputs.truncate(take_count);
outputs.append(&mut response_outputs);
if short_response {
    break;
}
```

- [ ] **Step 7: Remove exact-count failure from chat execution**

Delete the `if outputs.len() != request.count as usize` block in `execute_with_discovery_cache`. Keep the preceding `if outputs.is_empty()` block so zero-output jobs still fail.

- [ ] **Step 8: Verify chat image output partial success**

Run:

```bash
cargo test -p puffer-core chat_image_output_returns_empty_outputs_when_response_has_no_images -- --nocapture
cargo test -p puffer-core chat_image_output_later_empty_call_preserves_first_artifact -- --nocapture
cargo test -p puffer-core chat_image_output -- --nocapture
```

Expected: PASS.

- [ ] **Step 9: Commit chat image-output partial success**

```bash
git add crates/puffer-core/runtime/media/chat_image_output.rs
git commit -m "fix(media): preserve partial chat image outputs"
```

## Task 4: MiniMax Partial Success

**Files:**
- Modify: `crates/puffer-core/runtime/media/minimax_image.rs`

- [ ] **Step 1: Replace the MiniMax all-or-nothing test**

Rename `minimax_image_failed_later_call_writes_no_artifacts` to:

```rust
#[test]
fn minimax_image_failed_later_call_preserves_first_artifact() {
```

Replace the execution and assertions in that test with:

```rust
let result = MinimaxImageAdapter::new()
    .expect("adapter")
    .execute(
        &registry,
        &auth_store(),
        &MediaGenerationService::new(service_dir.path()),
        request,
    )
    .expect("partial generation succeeds");

assert_eq!(server.join().expect("server").len(), 2);
assert_eq!(result.job.requested_count, 2);
assert_eq!(result.job.status, MediaJobStatus::Succeeded);
assert_eq!(result.job.produced_count(), 1);
assert_eq!(result.artifacts.len(), 1);
assert_eq!(std::fs::read(&result.artifacts[0].path).unwrap(), b"image");
assert!(service_dir.path().join(".puffer/media/images").exists());
```

- [ ] **Step 2: Run the failing MiniMax partial test**

Run:

```bash
cargo test -p puffer-core minimax_image_failed_later_call_preserves_first_artifact -- --nocapture
```

Expected: FAIL because the adapter currently fails when produced count is less than requested count.

- [ ] **Step 3: Fail MiniMax only when no output was collected**

In `MinimaxImageAdapter::execute`, replace:

```rust
if outputs.len() != request.count as usize {
    let error = last_error
        .map(|error| format!("{error:#}"))
        .unwrap_or_else(|| {
            format!(
                "MiniMax image generation returned {} image(s), expected {}",
                outputs.len(),
                request.count
            )
        });
    job.error = Some(error.clone());
    job.transition(MediaJobStatus::Failed, now_ms())?;
    service.save_job(&job)?;
    bail!(error);
}
```

with:

```rust
if outputs.is_empty() {
    let error = last_error
        .map(|error| format!("{error:#}"))
        .unwrap_or_else(|| "MiniMax image generation produced no images".to_string());
    job.error = Some(error.clone());
    job.transition(MediaJobStatus::Failed, now_ms())?;
    service.save_job(&job)?;
    bail!(error);
}
```

- [ ] **Step 4: Verify MiniMax partial success**

Run:

```bash
cargo test -p puffer-core minimax_image_failed_later_call_preserves_first_artifact -- --nocapture
cargo test -p puffer-core minimax_image -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit MiniMax partial success**

```bash
git add crates/puffer-core/runtime/media/minimax_image.rs
git commit -m "fix(media): preserve partial minimax image outputs"
```

## Task 5: Final Verification And Plan Alignment

**Files:**
- Modify: `docs/superpowers/specs/2026-06-07-stable-multi-image-generation-design.md` only if implementation reveals a contradiction.
- Do not modify older plan files.

- [ ] **Step 1: Run focused media and tool tests**

Run:

```bash
cargo test -p puffer-resources bundled_image_generation_tool_requires_count_and_describes_multi_image_use -- --nocapture
cargo test -p puffer-core runtime_system_prompt_forbids_fabricating_image_substitutes -- --nocapture
cargo test -p puffer-core rejects_requested_count_outside_supported_range -- --nocapture
cargo test -p puffer-core image_generation -- --nocapture
cargo test -p puffer-core images_json -- --nocapture
cargo test -p puffer-core chat_image_output -- --nocapture
cargo test -p puffer-core minimax_image -- --nocapture
```

Expected: PASS.

- [ ] **Step 2: Run workspace-level Rust verification**

Run:

```bash
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 3: Confirm no old exact-count assertions remain in changed code**

Run:

```bash
rg -n "returned \\{\\} image\\(s\\), expected|writes_no_artifacts|expected \\{\\}|parameters_for_image_count" crates/puffer-core/runtime/media crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs
```

Expected: no matches in the changed image adapter paths for exact-count failure after partial output, and no residual `parameters_for_image_count` helper. Matches in unrelated code must be inspected before committing.

- [ ] **Step 4: Confirm no compatibility fields were reintroduced**

Run:

```bash
rg -n '"artifactId"|"path"' crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs crates/puffer-cli/src/daemon.rs crates/puffer-cli/src/desktop_api.rs apps/puffer-desktop/src
```

Expected: top-level result construction must not reintroduce single `artifactId` or single `path`; artifact-scoped fields inside `artifacts[]` and generated-media attachment sources are expected.

- [ ] **Step 5: Confirm working tree only contains intended changes**

Run:

```bash
git status --short
git diff --stat
```

Expected: only files named in this plan are modified.

- [ ] **Step 6: Commit final alignment fixes if needed**

If Step 1 through Step 5 required small follow-up fixes, commit them:

```bash
git add resources/tools/image_generation.yaml crates/puffer-resources/src/loader.rs crates/puffer-core/runtime/system_prompt.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation/tests/image_generation_tool_tests.rs crates/puffer-core/runtime/media/planner.rs crates/puffer-core/runtime/media/images_json.rs crates/puffer-core/runtime/media/images_json_tests.rs crates/puffer-core/runtime/media/chat_image_output.rs crates/puffer-core/runtime/media/minimax_image.rs docs/superpowers/specs/2026-06-07-stable-multi-image-generation-design.md
git commit -m "test(media): verify stable multi-image generation"
```

Expected: commit succeeds only if there are remaining changes after prior task commits.
