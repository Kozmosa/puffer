# Image Generation Planner Batch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace implicit image batch behavior with a shared planner that defaults every current image provider to stable serial single-image calls and reserves exact batching for explicitly configured, locally tested models.

**Architecture:** Add an explicit `execution.batch` descriptor, remove the old top-level `max_images_per_call`, and route `images_json`, `minimax_image`, and `chat_image_output` through a small image-specific planner. Keep plan execution serial and keep artifact persistence after all planned provider calls have succeeded.

**Tech Stack:** Rust (`puffer-provider-registry`, `puffer-core`, `puffer-resources`), YAML provider descriptors, Cargo tests.

---

## Recheck Outcome

The approved design was tightened before planning:

- No first-pass concurrency support. `per_image` is serial; exact batch is the only performance optimization in scope.
- No `max_concurrency` descriptor field.
- No generic adapter trait, execution runner, provider probing, or learned capability cache.
- No provider-specific nested request parameters.
- No bundled real provider starts in `exact` mode. Local fake-provider tests prove the `exact` path.
- Failed multi-call generation should write no artifact sidecars because outputs are persisted only after every planned provider call succeeds.
- Adapters may call the shared planner internally after resolving descriptors. They must not keep bespoke count splitting.

## File Structure

- Modify `crates/puffer-provider-registry/src/model.rs`
  - Add `MediaBatchDescriptor` and `MediaBatchMode`.
  - Add `MediaExecutionDescriptor.batch`.
  - Remove `MediaExecutionDescriptor.max_images_per_call`.
  - Reject old top-level `max_images_per_call` with serde `deny_unknown_fields`.

- Create `crates/puffer-core/runtime/media/planner.rs`
  - Implement image-specific count planning.
  - Provide small helpers for total and max call count.

- Modify `crates/puffer-core/runtime/media/mod.rs`
  - Register the new planner module.

- Modify `crates/puffer-core/runtime/media/images_json.rs`
  - Use the shared planner.
  - Remove `image_call_counts`.
  - Default missing batch to `per_image`.
  - Persist artifacts only after every planned call has succeeded.

- Modify `crates/puffer-core/runtime/media/images_json_tests.rs`
  - Update descriptor helpers to use `batch`.
  - Replace execution-limit tests with planner-backed batch-mode tests.

- Modify `crates/puffer-core/runtime/media/minimax_image.rs`
  - Use the shared planner instead of adapter-local count looping.
  - Keep only serial one-image calls for current descriptors.

- Modify `crates/puffer-core/runtime/media/chat_image_output.rs`
  - Use the shared planner instead of adapter-local count looping.
  - Keep only the number of image outputs requested by each call plan.

- Modify affected tests that construct `MediaExecutionDescriptor`:
  - `crates/puffer-core/media_runtime_tests.rs`
  - `crates/puffer-core/runtime/media/resolver.rs`
  - `crates/puffer-core/runtime/media/discovery.rs`
  - `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`

- Modify provider YAMLs:
  - `resources/providers/openai.yaml`
  - `resources/providers/xai.yaml`
  - `resources/providers/zhipu.yaml`
  - `resources/providers/byteplus.yaml`
  - `resources/providers/minimax.yaml`
  - `resources/providers/minimax-cn.yaml`
  - `resources/providers/openrouter.yaml`
  - `resources/providers/vercel-ai-gateway.yaml`

- Modify `crates/puffer-resources/tests/image_catalog_governance.rs`
  - Require explicit `batch.mode` for bundled image executions.
  - Assert first-pass bundled providers use `per_image`.

- Create concise update specs:
  - `specs/puffer-core/255.md`
  - `specs/puffer-resources/92.md`

## Task 1: Provider Registry Batch Descriptor

**Files:**
- Modify: `crates/puffer-provider-registry/src/model.rs`

- [ ] **Step 1: Write failing provider registry tests**

Replace the current `valid_image_media_descriptor_parses_and_validates` YAML block so execution uses nested `batch`:

```yaml
media:
  image:
    discovery:
      adapter: static
    execution:
      adapter: images_json
      base_url: https://api.test-provider.example
      path: /v1/images/generations
      batch:
        mode: exact
        max_images_per_call: 4
    models:
      - id: gpt-image-1
        display_name: GPT Image 1
        operations:
          - generate
        parameters:
          - name: size
            label: Size
            values:
              - 1024x1024
              - 1536x1024
            default: 1024x1024
            request_field: size
```

Update the assertion in that test:

```rust
let execution = image.execution.as_ref().expect("image execution");
assert_eq!(execution.batch.mode, MediaBatchMode::Exact);
assert_eq!(execution.batch.max_images_per_call, Some(4));
```

Add these tests to the existing `#[cfg(test)] mod tests` in `crates/puffer-provider-registry/src/model.rs`:

```rust
#[test]
fn missing_image_execution_batch_defaults_to_per_image() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: /v1/images/generations
    models:
      - id: gpt-image-1
        operations:
          - generate
"#,
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");
    let execution = provider
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .and_then(|image| image.execution.as_ref())
        .expect("image execution");

    assert_eq!(execution.batch.mode, MediaBatchMode::PerImage);
    assert_eq!(execution.batch.max_images_per_call, None);
    provider
        .validate_media_descriptors()
        .expect("default per-image batch validates");
}

#[test]
fn per_image_batch_rejects_max_images_per_call() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: /v1/images/generations
      batch:
        mode: per_image
        max_images_per_call: 1
    models:
      - id: gpt-image-1
        operations:
          - generate
"#,
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    let error = provider
        .validate_media_descriptors()
        .expect_err("per-image mode must not carry an exact batch limit");

    assert!(
        error
            .to_string()
            .contains("media.image.execution.batch.max_images_per_call"),
        "{error}"
    );
}

#[test]
fn exact_batch_requires_at_least_two_images_per_call() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: /v1/images/generations
      batch:
        mode: exact
        max_images_per_call: 1
    models:
      - id: gpt-image-1
        operations:
          - generate
"#,
    );
    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("provider parses");

    let error = provider
        .validate_media_descriptors()
        .expect_err("exact mode needs a real batch size");

    assert!(
        error
            .to_string()
            .contains("media.image.execution.batch.max_images_per_call"),
        "{error}"
    );
}

#[test]
fn old_top_level_image_batch_limit_is_rejected() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  image:
    execution:
      adapter: images_json
      path: /v1/images/generations
      max_images_per_call: 4
    models:
      - id: gpt-image-1
        operations:
          - generate
"#,
    );

    let error = serde_yaml::from_str::<ProviderDescriptor>(&yaml)
        .expect_err("old top-level batch limit should be rejected");

    assert!(error.to_string().contains("max_images_per_call"), "{error}");
}
```

Remove or rename the existing `zero_image_execution_batch_limit_is_rejected_by_validation` test because the old top-level field should no longer parse.

- [ ] **Step 2: Run failing provider registry tests**

Run:

```bash
cargo test -p puffer-provider-registry image_media_descriptor -- --nocapture
```

Expected: FAIL because `MediaBatchMode`, `MediaBatchDescriptor`, and `execution.batch` do not exist yet, and old `max_images_per_call` still parses.

- [ ] **Step 3: Implement batch descriptor schema**

In `crates/puffer-provider-registry/src/model.rs`, replace the `MediaExecutionDescriptor` definition with:

```rust
/// Describes the API-shape adapter used for image execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MediaExecutionDescriptor {
    pub adapter: MediaExecutionKind,
    #[serde(default)]
    pub base_url: Option<String>,
    pub path: String,
    /// Describes how requested image counts are split into provider calls.
    #[serde(default)]
    pub batch: MediaBatchDescriptor,
}
```

Add the batch types near `MediaExecutionDescriptor`:

```rust
/// Describes how an image execution endpoint handles multi-image requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MediaBatchDescriptor {
    #[serde(default = "default_media_batch_mode")]
    pub mode: MediaBatchMode,
    #[serde(default)]
    pub max_images_per_call: Option<u8>,
}

impl Default for MediaBatchDescriptor {
    fn default() -> Self {
        Self {
            mode: MediaBatchMode::PerImage,
            max_images_per_call: None,
        }
    }
}

/// Describes supported image batch execution policies.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaBatchMode {
    PerImage,
    Exact,
}

fn default_media_batch_mode() -> MediaBatchMode {
    MediaBatchMode::PerImage
}
```

Update `MediaExecutionDescriptor::validate`:

```rust
match self.batch.mode {
    MediaBatchMode::PerImage => {
        if self.batch.max_images_per_call.is_some() {
            errors.push(format!(
                "{location}.batch.max_images_per_call is only valid when batch.mode is exact"
            ));
        }
    }
    MediaBatchMode::Exact => match self.batch.max_images_per_call {
        Some(limit) if limit >= 2 => {}
        _ => errors.push(format!(
            "{location}.batch.max_images_per_call must be at least 2 when batch.mode is exact"
        )),
    },
}
```

Remove the old `max_images_per_call` field and the old validation block that checks `self.max_images_per_call == Some(0)`.

- [ ] **Step 4: Run provider registry tests**

Run:

```bash
cargo test -p puffer-provider-registry image_media_descriptor -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit provider registry schema**

Run:

```bash
git add crates/puffer-provider-registry/src/model.rs
git commit -m "feat(media): add explicit image batch descriptor"
```

## Task 2: Shared Image Generation Planner

**Files:**
- Create: `crates/puffer-core/runtime/media/planner.rs`
- Modify: `crates/puffer-core/runtime/media/mod.rs`

- [ ] **Step 1: Write planner tests and implementation together**

Create `crates/puffer-core/runtime/media/planner.rs` with this complete module:

```rust
use anyhow::{bail, Result};
use puffer_provider_registry::{MediaBatchDescriptor, MediaBatchMode};

/// Describes a complete image generation execution plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageGenerationPlan {
    pub(crate) calls: Vec<ImageCallPlan>,
}

impl ImageGenerationPlan {
    /// Returns the largest image count requested by one provider call.
    pub(crate) fn max_call_count(&self) -> u8 {
        self.calls
            .iter()
            .map(|call| call.requested_count)
            .max()
            .unwrap_or(0)
    }

    /// Returns the total number of images requested by this plan.
    pub(crate) fn total_requested_count(&self) -> u8 {
        self.calls
            .iter()
            .map(|call| call.requested_count)
            .sum::<u8>()
    }
}

/// Describes one provider request within an image generation plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageCallPlan {
    pub(crate) call_index: usize,
    pub(crate) requested_count: u8,
}

/// Plans image generation provider calls for a requested output count.
pub(crate) fn plan_image_generation(
    requested_count: u8,
    batch: &MediaBatchDescriptor,
) -> Result<ImageGenerationPlan> {
    if requested_count == 0 {
        bail!("image generation count must be between 1 and 4");
    }

    let call_counts = match batch.mode {
        MediaBatchMode::PerImage => vec![1; requested_count as usize],
        MediaBatchMode::Exact => {
            let limit = batch.max_images_per_call.unwrap_or(0);
            if limit < 2 {
                bail!("exact image batch mode requires max_images_per_call of at least 2");
            }
            split_exact_batches(requested_count, limit)
        }
    };

    Ok(ImageGenerationPlan {
        calls: call_counts
            .into_iter()
            .enumerate()
            .map(|(call_index, requested_count)| ImageCallPlan {
                call_index,
                requested_count,
            })
            .collect(),
    })
}

fn split_exact_batches(total: u8, limit: u8) -> Vec<u8> {
    let mut remaining = total;
    let mut counts = Vec::new();
    while remaining > 0 {
        let count = remaining.min(limit);
        counts.push(count);
        remaining -= count;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn per_image_batch() -> MediaBatchDescriptor {
        MediaBatchDescriptor {
            mode: MediaBatchMode::PerImage,
            max_images_per_call: None,
        }
    }

    fn exact_batch(limit: u8) -> MediaBatchDescriptor {
        MediaBatchDescriptor {
            mode: MediaBatchMode::Exact,
            max_images_per_call: Some(limit),
        }
    }

    fn counts(plan: &ImageGenerationPlan) -> Vec<u8> {
        plan.calls
            .iter()
            .map(|call| call.requested_count)
            .collect()
    }

    #[test]
    fn per_image_plan_splits_every_image_into_its_own_call() {
        let plan = plan_image_generation(4, &per_image_batch()).expect("plan");

        assert_eq!(counts(&plan), vec![1, 1, 1, 1]);
        assert_eq!(plan.max_call_count(), 1);
        assert_eq!(plan.total_requested_count(), 4);
        assert_eq!(plan.calls[0].call_index, 0);
        assert_eq!(plan.calls[3].call_index, 3);
    }

    #[test]
    fn exact_plan_splits_by_declared_limit() {
        let plan = plan_image_generation(4, &exact_batch(2)).expect("plan");

        assert_eq!(counts(&plan), vec![2, 2]);
        assert_eq!(plan.max_call_count(), 2);
        assert_eq!(plan.total_requested_count(), 4);
    }

    #[test]
    fn exact_plan_uses_remainder_call() {
        let plan = plan_image_generation(4, &exact_batch(3)).expect("plan");

        assert_eq!(counts(&plan), vec![3, 1]);
        assert_eq!(plan.max_call_count(), 3);
        assert_eq!(plan.total_requested_count(), 4);
    }

    #[test]
    fn missing_batch_descriptor_defaults_to_per_image() {
        let plan =
            plan_image_generation(2, &MediaBatchDescriptor::default()).expect("default plan");

        assert_eq!(counts(&plan), vec![1, 1]);
    }
}
```

- [ ] **Step 2: Register the planner module**

Add this line to `crates/puffer-core/runtime/media/mod.rs`:

```rust
pub(crate) mod planner;
```

- [ ] **Step 3: Run planner tests**

Run:

```bash
cargo test -p puffer-core runtime::media::planner -- --nocapture
```

Expected: PASS.

- [ ] **Step 4: Commit planner**

Run:

```bash
git add crates/puffer-core/runtime/media/mod.rs crates/puffer-core/runtime/media/planner.rs
git commit -m "feat(media): add image generation planner"
```

## Task 3: Route `images_json` Through Planner

**Files:**
- Modify: `crates/puffer-core/runtime/media/images_json.rs`
- Modify: `crates/puffer-core/runtime/media/images_json_tests.rs`

- [ ] **Step 1: Update test helper to use batch descriptors**

In `crates/puffer-core/runtime/media/images_json_tests.rs`, replace `registry_with_provider_parameters_and_execution_limit` with a helper that accepts `MediaBatchDescriptor`:

```rust
fn registry_with_provider_parameters_and_batch(
    provider_id: &str,
    base_url: String,
    parameters: Vec<MediaParameterSpec>,
    batch: MediaBatchDescriptor,
) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderDescriptor {
        id: provider_id.to_string(),
        display_name: "Exact Provider".to_string(),
        base_url,
        default_api: "openai-responses".to_string(),
        auth_modes: vec![AuthMode::ApiKey],
        headers: IndexMap::from([("x-provider-header".to_string(), "present".to_string())]),
        query_params: IndexMap::from([("api-version".to_string(), "2026-06-05".to_string())]),
        chat_completions_path: None,
        discovery: None,
        media: Some(ProviderMediaDescriptor {
            image: Some(ImageMediaDescriptor {
                discovery: None,
                execution: Some(MediaExecutionDescriptor {
                    adapter: MediaExecutionKind::ImagesJson,
                    base_url: None,
                    path: "/custom/images".to_string(),
                    batch,
                }),
                models: vec![MediaModelDescriptor {
                    id: "exact-image-model".to_string(),
                    display_name: Some("Exact Image Model".to_string()),
                    execution: None,
                    operations: vec![MediaOperation::Generate],
                    parameters,
                }],
            }),
        }),
        models: Vec::<ModelDescriptor>::new(),
    });
    registry
}
```

Add helper constructors near `image_parameters()`:

```rust
fn per_image_batch() -> MediaBatchDescriptor {
    MediaBatchDescriptor {
        mode: MediaBatchMode::PerImage,
        max_images_per_call: None,
    }
}

fn exact_batch(limit: u8) -> MediaBatchDescriptor {
    MediaBatchDescriptor {
        mode: MediaBatchMode::Exact,
        max_images_per_call: Some(limit),
    }
}
```

Update imports to include `MediaBatchDescriptor` and `MediaBatchMode`.

- [ ] **Step 2: Replace the old split test with per-image planning test**

Replace `image_call_counts_split_by_execution_limit` and `images_json_repeats_single_image_calls_when_descriptor_limits_batch_size` with:

```rust
#[test]
fn images_json_repeats_single_image_calls_in_per_image_mode() {
    let (base_url, server) =
        spawn_repeated_image_server_with_body(r#"{"data":[{"b64_json":"aW1hZ2U="}]}"#, 2);
    let mut parameters = image_parameters();
    parameters.push(sequential_generation_parameter());
    let registry = registry_with_provider_parameters_and_batch(
        "exact-provider",
        base_url,
        parameters,
        per_image_batch(),
    );
    let mut auth_store = AuthStore::default();
    auth_store.set_api_key("exact-provider", "sk-test");
    let service_dir = tempfile::tempdir().unwrap();
    let service = MediaGenerationService::new(service_dir.path());
    let request = ImagesJsonGenerationRequest {
        provider_id: "exact-provider".to_string(),
        model_id: "exact-image-model".to_string(),
        adapter: "images_json".to_string(),
        prompt: "draw two images".to_string(),
        parameters: BTreeMap::from([
            ("size".to_string(), "1024x1024".to_string()),
            ("quality".to_string(), "auto".to_string()),
            ("output_format".to_string(), "png".to_string()),
        ]),
        count: 2,
    };

    let result = ImagesJsonAdapter::new()
        .unwrap()
        .execute(&registry, &auth_store, &service, request)
        .unwrap();

    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| !request.contains("\"n\"")));
    assert!(requests
        .iter()
        .all(|request| request.contains("\"sequential_image_generation\":\"disabled\"")));
    assert_eq!(result.job.requested_count, 2);
    assert_eq!(result.job.artifact_ids.len(), 2);
    assert_eq!(result.artifacts.len(), 2);
    assert_eq!(result.artifacts[0].metadata["index"], 0);
    assert_eq!(result.artifacts[1].metadata["index"], 1);
}
```

- [ ] **Step 3: Add exact batch and failed-no-artifacts tests**

Add:

```rust
#[test]
fn images_json_uses_exact_batch_mode_when_descriptor_opts_in() {
    let (base_url, server) = spawn_image_server_with_body(
        r#"{"data":[{"b64_json":"aW1hZ2UtMQ=="},{"b64_json":"aW1hZ2UtMg=="}]}"#,
    );
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

    let result = ImagesJsonAdapter::new()
        .unwrap()
        .execute(&registry, &auth_store(), &service, request)
        .unwrap();

    let request_text = server.join().unwrap();
    assert!(request_text.contains("\"n\":2"));
    assert_eq!(result.artifacts.len(), 2);
}

#[test]
fn images_json_failed_later_per_image_call_writes_no_artifacts() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().expect("request");
            requests.push(read_http_request(&mut stream));
            let body = if index == 0 {
                r#"{"data":[{"b64_json":"aW1hZ2U="}]}"#.to_string()
            } else {
                r#"{"data":[]}"#.to_string()
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
    let registry = registry_with_provider_parameters_and_batch(
        "exact-provider",
        format!("http://{address}"),
        image_parameters(),
        per_image_batch(),
    );
    let service_dir = tempfile::tempdir().unwrap();
    let service = MediaGenerationService::new(service_dir.path());
    let mut request = request();
    request.count = 2;

    let error = ImagesJsonAdapter::new()
        .unwrap()
        .execute(&registry, &auth_store(), &service, request)
        .expect_err("second call under-produces");

    assert_eq!(
        error.to_string(),
        "image generation returned 0 image(s), expected 1 for call 1"
    );
    assert_eq!(server.join().unwrap().len(), 2);
    assert!(!service_dir.path().join(".puffer/media/images").exists());
}
```

- [ ] **Step 4: Run tests to verify failure**

Run:

```bash
cargo test -p puffer-core images_json_repeats_single_image_calls_in_per_image_mode -- --nocapture
cargo test -p puffer-core images_json_uses_exact_batch_mode_when_descriptor_opts_in -- --nocapture
cargo test -p puffer-core images_json_failed_later_per_image_call_writes_no_artifacts -- --nocapture
```

Expected: FAIL because `images_json` still uses `execution.max_images_per_call`, the planner is not wired, and errors do not include call indexes.

- [ ] **Step 5: Implement planner usage in `images_json`**

In `crates/puffer-core/runtime/media/images_json.rs`, add imports:

```rust
use super::planner::{plan_image_generation, ImageGenerationPlan};
```

Replace the call-count block in `execute`:

```rust
let plan = plan_image_generation(request.count, &execution.batch)?;
let request_parameters = parameters_for_image_count(
    selected_parameters_with_defaults(&capability, &request.parameters)?,
    plan.max_call_count(),
);
```

Change `request_images` to accept the plan:

```rust
fn request_images(
    &self,
    provider: &ProviderDescriptor,
    auth_store: &AuthStore,
    request: &ImagesJsonGenerationRequest,
    parameters: BTreeMap<String, String>,
    execution: &MediaExecutionDescriptor,
    plan: &ImageGenerationPlan,
) -> Result<Vec<ImageOutput>> {
```

Use the plan inside the loop:

```rust
for call in &plan.calls {
    let body = ImagesJsonRequest::new(
        &request.model_id,
        &request.prompt,
        parameters.clone(),
        call.requested_count,
    )
    .to_body();
    // existing HTTP send/read/status handling stays here
    outputs.extend(image_outputs_from_response(
        &self.client,
        &value,
        call.requested_count,
        call.call_index,
    )?);
}
```

Change `image_outputs_from_response` signature and error:

```rust
fn image_outputs_from_response(
    client: &Client,
    value: &Value,
    count: u8,
    call_index: usize,
) -> Result<Vec<ImageOutput>> {
    let Some(items) = value.get("data").and_then(Value::as_array) else {
        bail!("image generation response did not contain an image");
    };
    let requested_count = count as usize;
    if items.len() < requested_count {
        bail!(
            "image generation returned {} image(s), expected {} for call {}",
            items.len(),
            requested_count,
            call_index
        );
    }
    let mut outputs = Vec::new();
    for item in items.iter().take(requested_count) {
        outputs.push(image_output_from_item(client, item)?);
    }
    if outputs.is_empty() {
        bail!("image generation response did not contain an image");
    }
    Ok(outputs)
}
```

After `request_images` succeeds and before persistence, add:

```rust
if outputs.len() != request.count as usize {
    job.error = Some(format!(
        "image generation returned {} image(s), expected {}",
        outputs.len(),
        request.count
    ));
    job.transition(MediaJobStatus::Failed, now_ms())?;
    service.save_job(&job)?;
    bail!(
        "image generation returned {} image(s), expected {}",
        outputs.len(),
        request.count
    );
}
```

Remove `image_call_counts`.

- [ ] **Step 6: Run `images_json` tests**

Run:

```bash
cargo test -p puffer-core images_json -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Commit `images_json` planner wiring**

Run:

```bash
git add crates/puffer-core/runtime/media/images_json.rs crates/puffer-core/runtime/media/images_json_tests.rs
git commit -m "feat(media): plan images_json generation calls"
```

## Task 4: Route MiniMax And Chat Image Output Through Planner

**Files:**
- Modify: `crates/puffer-core/runtime/media/minimax_image.rs`
- Modify: `crates/puffer-core/runtime/media/chat_image_output.rs`

- [ ] **Step 1: Update MiniMax loop to use planner**

In `crates/puffer-core/runtime/media/minimax_image.rs`, add:

```rust
use super::planner::plan_image_generation;
```

After resolving `execution`, create the plan:

```rust
let plan = plan_image_generation(request.count, &execution.batch)?;
```

Replace `for _ in 0..request.count` with:

```rust
for call in &plan.calls {
    if call.requested_count != 1 {
        bail!("MiniMax image generation supports only per-image call plans");
    }
    match self.request_image(
        provider,
        auth_store,
        &execution,
        &request,
        selected_parameters.clone(),
    ) {
        Ok(output) => outputs.push(output),
        Err(error) => last_error = Some(error),
    }
}
```

- [ ] **Step 2: Update chat image output loop to use planner**

In `crates/puffer-core/runtime/media/chat_image_output.rs`, add:

```rust
use super::planner::plan_image_generation;
```

After resolving `execution`, create the plan:

```rust
let plan = plan_image_generation(request.count, &execution.batch)?;
```

Replace the current `for _ in 0..request.count` loop with:

```rust
for call in &plan.calls {
    match self.request_image(provider, auth_store, &execution, &request) {
        Ok(mut response_outputs) => {
            let take_count = call.requested_count as usize;
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
        }
        Err(error) => {
            last_error = Some(error);
            break;
        }
    }
}
```

Keep the existing `outputs.is_empty()` failure block, then add a full-count guard before persistence:

```rust
if outputs.len() != request.count as usize {
    let error = last_error
        .map(|error| format!("{error:#}"))
        .unwrap_or_else(|| {
            format!(
                "chat image-output returned {} image(s), expected {}",
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

- [ ] **Step 3: Run focused adapter tests**

Run:

```bash
cargo test -p puffer-core minimax_image -- --nocapture
cargo test -p puffer-core chat_image_output -- --nocapture
```

Expected: PASS after constructor updates in the next task if these modules still fail on `MediaExecutionDescriptor.batch`.

- [ ] **Step 4: Commit MiniMax and chat planner wiring**

Run:

```bash
git add crates/puffer-core/runtime/media/minimax_image.rs crates/puffer-core/runtime/media/chat_image_output.rs
git commit -m "feat(media): plan native image adapter calls"
```

## Task 5: Update Descriptor Constructors And Resources

**Files:**
- Modify: `crates/puffer-core/media_runtime_tests.rs`
- Modify: `crates/puffer-core/runtime/media/resolver.rs`
- Modify: `crates/puffer-core/runtime/media/discovery.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
- Modify: provider YAMLs listed in File Structure
- Modify: `crates/puffer-resources/tests/image_catalog_governance.rs`

- [ ] **Step 1: Update Rust test constructors**

For every test `MediaExecutionDescriptor` literal, replace:

```rust
max_images_per_call: None,
```

with:

```rust
batch: puffer_provider_registry::MediaBatchDescriptor::default(),
```

If the file already imports many provider-registry types, add `MediaBatchDescriptor` to the import and use:

```rust
batch: MediaBatchDescriptor::default(),
```

- [ ] **Step 2: Update provider YAMLs to explicit `per_image`**

For every `media.image.execution` block in bundled provider YAMLs, add:

```yaml
      batch:
        mode: per_image
```

For every `models[].execution` override under `resources/providers/vercel-ai-gateway.yaml`, add:

```yaml
          batch:
            mode: per_image
```

Remove this old line from `resources/providers/zhipu.yaml`:

```yaml
      max_images_per_call: 1
```

- [ ] **Step 3: Update governance tests**

Replace `zhipu_images_json_limits_batches_to_single_image_calls` with:

```rust
#[test]
fn zhipu_images_json_uses_per_image_batch_mode() {
    let descriptor = provider_descriptor(
        "zhipu",
        include_str!("../../../resources/providers/zhipu.yaml"),
    );
    let image = descriptor
        .media
        .as_ref()
        .and_then(|media| media.image.as_ref())
        .expect("zhipu image media descriptor");
    let execution = image
        .execution
        .as_ref()
        .expect("zhipu image execution descriptor");

    assert_eq!(execution.adapter, MediaExecutionKind::ImagesJson);
    assert_eq!(execution.path, "/images/generations");
    assert_eq!(execution.batch.mode, MediaBatchMode::PerImage);
    assert_eq!(execution.batch.max_images_per_call, None);
}
```

Add a raw-YAML governance helper in `crates/puffer-resources/tests/image_catalog_governance.rs`:

```rust
fn assert_raw_image_executions_declare_batch_mode(provider_id: &str, yaml: &str) {
    let value: serde_yaml::Value =
        serde_yaml::from_str(yaml).unwrap_or_else(|error| panic!("{provider_id}: {error}"));
    let image = value
        .get("media")
        .and_then(|media| media.get("image"))
        .unwrap_or_else(|| panic!("{provider_id}: missing media.image"));
    let execution = image
        .get("execution")
        .unwrap_or_else(|| panic!("{provider_id}: missing media.image.execution"));
    assert!(
        execution
            .get("batch")
            .and_then(|batch| batch.get("mode"))
            .and_then(serde_yaml::Value::as_str)
            .is_some(),
        "{provider_id}: media.image.execution.batch.mode must be explicit"
    );
    if let Some(models) = image.get("models").and_then(serde_yaml::Value::as_sequence) {
        for model in models {
            if let Some(model_execution) = model.get("execution") {
                let model_id = model
                    .get("id")
                    .and_then(serde_yaml::Value::as_str)
                    .unwrap_or("<missing>");
                assert!(
                    model_execution
                        .get("batch")
                        .and_then(|batch| batch.get("mode"))
                        .and_then(serde_yaml::Value::as_str)
                        .is_some(),
                    "{provider_id}/{model_id}: models[].execution.batch.mode must be explicit"
                );
            }
        }
    }
}
```

Add a governance test:

```rust
#[test]
fn bundled_image_executions_declare_explicit_batch_mode() {
    for (provider_id, yaml) in [
        ("openai", include_str!("../../../resources/providers/openai.yaml")),
        ("xai", include_str!("../../../resources/providers/xai.yaml")),
        ("zhipu", include_str!("../../../resources/providers/zhipu.yaml")),
        ("byteplus", include_str!("../../../resources/providers/byteplus.yaml")),
        ("minimax", include_str!("../../../resources/providers/minimax.yaml")),
        (
            "minimax-cn",
            include_str!("../../../resources/providers/minimax-cn.yaml"),
        ),
        (
            "openrouter",
            include_str!("../../../resources/providers/openrouter.yaml"),
        ),
        (
            "vercel-ai-gateway",
            include_str!("../../../resources/providers/vercel-ai-gateway.yaml"),
        ),
    ] {
        assert_raw_image_executions_declare_batch_mode(provider_id, yaml);
    }
}
```

Update imports to include `MediaBatchMode`.

- [ ] **Step 4: Run resource and constructor tests**

Run:

```bash
cargo test -p puffer-resources image_catalog_governance -- --nocapture
cargo test -p puffer-core media_runtime_tests -- --nocapture
cargo test -p puffer-core runtime::media::resolver -- --nocapture
cargo test -p puffer-core runtime::media::discovery -- --nocapture
cargo test -p puffer-core image_generation_tool -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit resources and constructor updates**

Run:

```bash
git add crates/puffer-core/media_runtime_tests.rs crates/puffer-core/runtime/media/resolver.rs crates/puffer-core/runtime/media/discovery.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs crates/puffer-resources/tests/image_catalog_governance.rs resources/providers/*.yaml
git commit -m "chore(media): declare per-image batch mode"
```

## Task 6: Component Update Specs

**Files:**
- Create: `specs/puffer-core/255.md`
- Create: `specs/puffer-resources/92.md`

- [ ] **Step 1: Write puffer-core update spec**

Create `specs/puffer-core/255.md`:

```markdown
# Image Generation Planner Batch Modes

## Scope

Core image generation now plans requested image counts through a shared
image-specific planner before provider adapters execute.

## Change

- Added explicit image batch descriptors with `per_image` and `exact` modes.
- Added a shared planner that converts a requested count into serial provider
  call plans.
- `images_json`, `minimax_image`, and `chat_image_output` use the shared planner
  instead of adapter-local count splitting.
- `per_image` sends one-image calls and omits provider count fields such as
  `n`.
- `exact` sends batch count fields only when descriptors explicitly opt in.
- Artifacts are persisted only after all planned provider calls succeed.

## Compatibility

The old implicit `images_json` behavior is removed. Missing batch descriptors
resolve to safe `per_image` at runtime, but bundled resources declare batch mode
explicitly. The old top-level `max_images_per_call` descriptor field is no
longer accepted.
```

- [ ] **Step 2: Write puffer-resources update spec**

Create `specs/puffer-resources/92.md`:

```markdown
# Image Provider Batch Descriptor Governance

## Scope

Bundled image provider descriptors now declare explicit batch execution policy.

## Change

- All bundled image executions declare `batch.mode`.
- Current OpenAI, xAI, Zhipu, BytePlus, MiniMax, MiniMax CN, OpenRouter, and
  Vercel AI Gateway image descriptors start in `per_image` mode.
- Vercel model-level image execution overrides also declare `per_image`.
- Zhipu no longer uses the old top-level `max_images_per_call` field.
- Governance tests require explicit batch mode in bundled image descriptors.

## Compatibility

New provider descriptors default to `per_image` at runtime when batch metadata
is missing, but bundled resources keep the mode explicit to make provider
behavior auditable.
```

- [ ] **Step 3: Commit update specs**

Run:

```bash
git add specs/puffer-core/255.md specs/puffer-resources/92.md
git commit -m "docs(media): record image batch planner changes"
```

## Task 7: Full Verification

**Files:**
- No code changes unless verification exposes a concrete compile or test issue.

- [ ] **Step 1: Run focused test suite**

Run:

```bash
cargo test -p puffer-provider-registry media_descriptor -- --nocapture
cargo test -p puffer-core images_json -- --nocapture
cargo test -p puffer-core minimax_image -- --nocapture
cargo test -p puffer-core chat_image_output -- --nocapture
cargo test -p puffer-core runtime::media::planner -- --nocapture
cargo test -p puffer-resources image_catalog_governance -- --nocapture
```

Expected: PASS.

- [ ] **Step 2: Run workspace test suite**

Run:

```bash
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 3: Review final diff**

Run:

```bash
git diff --stat
git diff --check
git status --short
```

Expected:

- `git diff --check` prints no whitespace errors.
- `git status --short` shows only intended files before the final commit.

- [ ] **Step 4: Final commit**

Run:

```bash
git add crates/puffer-provider-registry/src/model.rs crates/puffer-core/runtime/media crates/puffer-core/media_runtime_tests.rs crates/puffer-resources/tests/image_catalog_governance.rs resources/providers specs/puffer-core/255.md specs/puffer-resources/92.md
git commit -m "fix(media): plan exact image generation batches"
```

Expected: commit succeeds.

## Execution Notes

- Do not restart Puffer services for unit-test-only validation.
- If this plan is executed against a running desktop app, rebuild and restart after the final workspace tests pass.
- Keep all real bundled providers in `per_image` mode in this implementation. Enabling exact mode for a real provider is a separate resource-governance change after verification.
