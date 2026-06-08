# Exact Media Runtime V1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify image and video media generation settings, capabilities, validation, execution, and artifacts around the existing exact media runtime.

**Architecture:** Reuse `crates/puffer-core/runtime/media` and extend its existing exact image path to video instead of creating a new media platform. Persist image/video defaults with one selection shape, resolve descriptor-backed capabilities by kind, dispatch through existing adapter modules, and keep desktop UI parameter-driven from capabilities.

**Tech Stack:** Rust workspace crates (`puffer-config`, `puffer-provider-registry`, `puffer-core`, `puffer-cli`), Tauri Rust backend, Svelte desktop UI, Playwright desktop tests, YAML provider resources.

---

## Scope Check

In scope:

- One unified media settings shape for image and video.
- Descriptor-backed static video capabilities.
- Replicate video as the first executable video adapter because `replicate_video.rs` already exists.
- Existing exact image adapters continue working through compatibility wrappers.
- Desktop settings UI renders all media parameters from capabilities.
- Generated video artifacts are persisted and represented as metadata-first attachments.

Out of scope:

- New workspace crate.
- Generic async adapter trait.
- Background scheduler for detached video continuation.
- Dynamic video model discovery.
- Provider-specific Svelte controls.
- Audio, edit, extend, image-to-video, or video-to-video operations.
- Live provider tests.

## File Structure

Core contracts:

- Modify `crates/puffer-config/src/lib.rs`
  - Owns persisted media config shape.
- Modify `apps/puffer-desktop/src-tauri/src/dtos.rs`
  - Owns desktop/Tauri media DTOs.
- Modify `crates/puffer-cli/src/desktop_api_types.rs`
  - Owns daemon JSON DTOs.
- Modify `apps/puffer-desktop/src/lib/types.ts`
  - Owns frontend settings and capability types.
- Modify `apps/puffer-desktop/tests/support/fakeDaemon.ts`
  - Owns fake settings normalization for Playwright.
- Modify `crates/puffer-cli/src/desktop_api.rs`
  - Converts persisted media settings into daemon snapshots.
- Modify `crates/puffer-cli/src/daemon.rs`
  - Parses unified media config patches.
- Modify `apps/puffer-desktop/src-tauri/src/backend.rs`
  - Converts persisted media settings into Tauri snapshots.

Provider capability resolution:

- Modify `crates/puffer-provider-registry/src/model.rs`
  - Owns provider media descriptor schema and validation.
- Modify `crates/puffer-provider-registry/src/model_tests.rs`
  - Owns descriptor parsing and validation tests.
- Modify `crates/puffer-core/runtime/media/resolver.rs`
  - Owns descriptor-to-capability resolution and selection validation.
- Modify `crates/puffer-core/runtime/media/capabilities.rs`
  - Owns shared media capability and selection support structs.

Runtime execution:

- Modify `crates/puffer-core/media_runtime.rs`
  - Owns public exact media runtime facade used by daemon/Tauri.
- Modify `crates/puffer-core/runtime/media/jobs.rs`
  - Owns durable job shape.
- Modify `crates/puffer-core/runtime/media/replicate_video.rs`
  - Owns Replicate video request mapping and lifecycle.
- Modify `crates/puffer-core/media_runtime_tests.rs`
  - Owns facade-level media runtime tests.

Daemon and Tauri integration:

- Modify `crates/puffer-cli/src/daemon.rs`
  - Owns daemon `list_media_capabilities` and `generate_media`.
- Modify `crates/puffer-cli/src/desktop_api.rs`
  - Owns settings snapshot conversion.
- Modify `apps/puffer-desktop/src-tauri/src/backend.rs`
  - Owns Tauri fallback backend media methods.
- Modify `apps/puffer-desktop/src-tauri/src/media_capabilities.rs`
  - Owns Tauri capability DTO conversion.

Desktop UI:

- Modify `apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte`
  - Owns parameter-driven settings modal.
- Modify `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
  - Owns generated media attachment presentation and `/video` routing surface.
- Modify `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
  - Owns Playwright coverage for settings and generated attachments.
- Modify `apps/puffer-desktop/src/lib/mockData.ts`
  - Owns local desktop fixture shape.

Docs:

- Create `specs/puffer-config/08.md`
- Create `specs/puffer-provider-registry/08.md`
- Create `specs/puffer-core/256.md`
- Create `specs/puffer-cli/165.md`
- Create `specs/puffer-desktop/680.md`

Current workspace note: there may be unstaged UI draft edits in
`MediaSettingsModal.svelte` and `chat-session-ui.spec.ts`. Before implementation,
inspect them and either reuse them as the starting point for Task 5 or separate
them from the branch. Do not reset user work.

---

### Task 1: Unified Media Settings Contract

**Files:**
- Modify: `crates/puffer-config/src/lib.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/dtos.rs`
- Modify: `crates/puffer-cli/src/desktop_api_types.rs`
- Modify: `crates/puffer-cli/src/desktop_api.rs`
- Modify: `crates/puffer-cli/src/daemon.rs`
- Modify: `apps/puffer-desktop/src/lib/types.ts`
- Modify: `apps/puffer-desktop/src/lib/mockData.ts`
- Modify: `apps/puffer-desktop/tests/support/fakeDaemon.ts`
- Modify: `apps/puffer-desktop/src-tauri/src/backend.rs`

- [ ] **Step 1: Write failing Rust config tests**

Add tests near the existing `puffer-config` config tests:

```rust
#[test]
fn media_config_parses_unified_image_and_video_selections() {
    let toml = r#"
app_name = "Puffer"
theme = "system"
mascot = { id = "none", display_name = "None", enabled = false }
ui = { no_alt_screen = false, tmux_golden_mode = false }

[media.image]
provider_id = "openai"
model_id = "gpt-image-1"
operation = "generate"
adapter = "images_json"
parameters = { size = "1024x1024", quality = "auto", output_format = "png" }

[media.video]
provider_id = "replicate"
model_id = "owner/video-version"
operation = "generate"
adapter = "replicate_video"
parameters = { aspect_ratio = "16:9", duration = "5" }
"#;

    let parsed: PufferConfig = toml::from_str(toml).expect("config parses");
    assert_eq!(parsed.media.image.as_ref().unwrap().adapter, "images_json");
    assert_eq!(
        parsed.media.video.as_ref().unwrap().parameters.get("duration"),
        Some(&"5".to_string())
    );
}

#[test]
fn media_config_defaults_to_unconfigured_selections() {
    let config = MediaConfig::default();
    assert!(config.image.is_none());
    assert!(config.video.is_none());
}
```

- [ ] **Step 2: Run config tests to verify failure**

Run:

```bash
cargo test -p puffer-config media_config_
```

Expected: fail because `MediaConfig.image` and `MediaConfig.video` are not
`Option<MediaGenerationConfig>` yet.

- [ ] **Step 3: Implement unified config structs**

Replace `ImageMediaConfig` and `VideoMediaConfig` with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MediaConfig {
    #[serde(default)]
    pub image: Option<MediaGenerationConfig>,
    #[serde(default)]
    pub video: Option<MediaGenerationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaGenerationConfig {
    #[serde(alias = "providerId")]
    pub provider_id: String,
    #[serde(alias = "modelId")]
    pub model_id: String,
    #[serde(default = "default_media_operation")]
    pub operation: String,
    pub adapter: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
}

fn default_media_operation() -> String {
    "generate".to_string()
}
```

Remove `default_video_aspect_ratio()` and `default_video_duration_seconds()`
after updating every reference in this task.

- [ ] **Step 4: Update Rust DTOs**

Use one DTO in both `apps/puffer-desktop/src-tauri/src/dtos.rs` and
`crates/puffer-cli/src/desktop_api_types.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSettingsDto {
    pub image: Option<MediaGenerationSettingsDto>,
    pub video: Option<MediaGenerationSettingsDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaGenerationSettingsDto {
    pub provider_id: String,
    pub model_id: String,
    pub operation: String,
    pub adapter: String,
    pub parameters: std::collections::BTreeMap<String, String>,
}
```

For `desktop_api_types.rs`, keep `pub(crate)` and derive both `Serialize` and
`Deserialize` for media DTOs so update patches and snapshots use the same
shape.

- [ ] **Step 5: Update frontend types and fixtures**

Change `apps/puffer-desktop/src/lib/types.ts`:

```ts
export type MediaSettings = {
  image: MediaGenerationSettings | null;
  video: MediaGenerationSettings | null;
};

export type MediaGenerationSettings = {
  providerId: string;
  modelId: string;
  operation: "generate";
  adapter: string;
  parameters: Record<string, string>;
};
```

Update mock defaults:

```ts
media: {
  image: null,
  video: null
}
```

Update `FakeDaemon` normalization to accept either `null` or a complete object:

```ts
function normalizeMediaSelection(value: unknown): FakeMediaSelection | null {
  if (!value || typeof value !== "object") return null;
  const record = value as JsonRecord;
  if (
    typeof record.providerId !== "string" ||
    typeof record.modelId !== "string" ||
    typeof record.adapter !== "string"
  ) {
    return null;
  }
  return {
    providerId: record.providerId,
    modelId: record.modelId,
    operation: "generate",
    adapter: record.adapter,
    parameters: normalizeStringRecord(record.parameters)
  };
}
```

Add the string-record helper used by the normalizer:

```ts
function normalizeStringRecord(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object") return {};
  return Object.fromEntries(
    Object.entries(value as JsonRecord)
      .filter(([, entry]) => typeof entry === "string")
      .map(([key, entry]) => [key, entry as string])
  );
}
```

Update `desktop_api.rs`, `daemon.rs`, and Tauri `backend.rs` snapshot/config
mapping enough for Rust code to compile with the unified DTO shape. Full
generation execution wiring remains Task 4.

- [ ] **Step 6: Run contract checks**

Run:

```bash
cargo test -p puffer-config media_config_
cargo test -p puffer-cli media_config
cargo test -p corbina media_config
pnpm --dir apps/puffer-desktop check
```

Expected: config and DTO mapping tests pass, and frontend check passes for the
updated settings shape.

- [ ] **Step 7: Commit unified settings contract**

```bash
git add crates/puffer-config/src/lib.rs \
  apps/puffer-desktop/src-tauri/src/dtos.rs \
  crates/puffer-cli/src/desktop_api_types.rs \
  crates/puffer-cli/src/desktop_api.rs \
  crates/puffer-cli/src/daemon.rs \
  apps/puffer-desktop/src/lib/types.ts \
  apps/puffer-desktop/src/lib/mockData.ts \
  apps/puffer-desktop/tests/support/fakeDaemon.ts \
  apps/puffer-desktop/src-tauri/src/backend.rs
git commit -m "feat(media): unify image and video settings"
```

---

### Task 2: Provider Descriptor And Video Capability Resolution

**Files:**
- Modify: `crates/puffer-provider-registry/src/model.rs`
- Modify: `crates/puffer-provider-registry/src/model_tests.rs`
- Modify: `crates/puffer-core/runtime/media/capabilities.rs`
- Modify: `crates/puffer-core/runtime/media/resolver.rs`
- Modify: `crates/puffer-core/media_runtime_tests.rs`

- [ ] **Step 1: Write failing provider registry tests**

Add tests in `model_tests.rs`:

```rust
#[test]
fn provider_media_descriptor_accepts_video_models() {
    let yaml = r#"
id: replicate
display_name: Replicate
base_url: https://api.replicate.com
default_api: openai-responses
auth_modes: [api_key]
media:
  video:
    execution:
      adapter: replicate_video
      path: /v1/predictions
    models:
      - id: owner/model-version
        display_name: Video Model
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
"#;

    let provider: ProviderDescriptor = serde_yaml::from_str(yaml).expect("parse provider");
    provider.validate_media_descriptors().expect("valid media descriptor");
    assert!(provider.media.unwrap().video.is_some());
}

#[test]
fn provider_media_descriptor_rejects_invalid_video_parameter_default() {
    let yaml = r#"
id: replicate
display_name: Replicate
base_url: https://api.replicate.com
default_api: openai-responses
auth_modes: [api_key]
media:
  video:
    execution:
      adapter: replicate_video
      path: /v1/predictions
    models:
      - id: owner/model-version
        operations: [generate]
        parameters:
          - name: duration
            label: Duration
            values: ["5"]
            default: "8"
"#;

    let provider: ProviderDescriptor = serde_yaml::from_str(yaml).expect("parse provider");
    let error = provider.validate_media_descriptors().unwrap_err().to_string();
    assert!(error.contains("media.video.models[0].parameters[0].default"));
}
```

- [ ] **Step 2: Run registry tests to verify failure**

Run:

```bash
cargo test -p puffer-provider-registry provider_media_descriptor_
```

Expected: fail because `ProviderMediaDescriptor.video` and
`MediaExecutionKind::ReplicateVideo` do not exist.

- [ ] **Step 3: Generalize media descriptor structs**

In `model.rs`, replace image-only descriptor fields with:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderMediaDescriptor {
    #[serde(default)]
    pub image: Option<MediaKindDescriptor>,
    #[serde(default)]
    pub video: Option<MediaKindDescriptor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaKindDescriptor {
    #[serde(default)]
    pub discovery: Option<MediaDiscoveryDescriptor>,
    #[serde(default)]
    pub execution: Option<MediaExecutionDescriptor>,
    #[serde(default)]
    pub models: Vec<MediaModelDescriptor>,
}
```

Add to `MediaExecutionKind`:

```rust
ReplicateVideo,
```

Update validation locations to call:

```rust
validate_media_kind_descriptor("media.image", image, &mut errors);
validate_media_kind_descriptor("media.video", video, &mut errors);
```

- [ ] **Step 4: Write failing resolver tests**

Add resolver tests:

```rust
#[test]
fn connected_provider_with_replicate_video_descriptor_is_available() {
    let registry = registry_with(vec![provider(
        "replicate",
        vec![AuthMode::ApiKey],
        Some(video_media("owner/model-version")),
    )]);
    let auth = auth_store_with_api_key("replicate");

    let capabilities = resolve_media_capabilities(
        &registry,
        &auth,
        MediaKind::Video,
        MediaOperation::Generate,
        42,
        &MediaDiscoveryCache::default(),
    );

    assert_eq!(capabilities.len(), 1);
    assert_eq!(capabilities[0].adapter, "replicate_video");
    assert_eq!(capabilities[0].defaults.get("duration"), Some(&"5".to_string()));
}

#[test]
fn video_descriptor_with_image_adapter_is_not_available() {
    let registry = registry_with(vec![provider(
        "replicate",
        vec![AuthMode::ApiKey],
        Some(video_media_with_adapter(MediaExecutionKind::ImagesJson)),
    )]);
    let auth = auth_store_with_api_key("replicate");

    let capabilities = resolve_media_capabilities(
        &registry,
        &auth,
        MediaKind::Video,
        MediaOperation::Generate,
        42,
        &MediaDiscoveryCache::default(),
    );

    assert!(capabilities.is_empty());
}
```

- [ ] **Step 5: Implement video resolver support**

Refactor `resolve_media_capabilities()` so `MediaKind::Video` resolves static
descriptor models. Add adapter support functions:

```rust
fn execution_adapter_is_available_for_kind(kind: MediaKind, adapter: MediaExecutionKind) -> bool {
    matches!(
        (kind, adapter),
        (MediaKind::Image, MediaExecutionKind::ImagesJson)
            | (MediaKind::Image, MediaExecutionKind::ChatImageOutput)
            | (MediaKind::Image, MediaExecutionKind::MinimaxImage)
            | (MediaKind::Video, MediaExecutionKind::ReplicateVideo)
    )
}

pub(crate) fn adapter_id(adapter: MediaExecutionKind) -> &'static str {
    match adapter {
        MediaExecutionKind::ImagesJson => "images_json",
        MediaExecutionKind::ChatImageOutput => "chat_image_output",
        MediaExecutionKind::MinimaxImage => "minimax_image",
        MediaExecutionKind::ReplicateVideo => "replicate_video",
    }
}
```

Keep trusted discovery cache image-only by applying discovered models only when
`kind == MediaKind::Image`.

- [ ] **Step 6: Keep video descriptors test-scoped**

Do not add a bundled `resources/providers/replicate.yaml` in this slice. Use
the resolver test fixture to declare:

```yaml
adapter: replicate_video
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

- [ ] **Step 7: Run resolver and registry tests**

Run:

```bash
cargo test -p puffer-provider-registry provider_media_descriptor_
cargo test -p puffer-core media::resolver
```

Expected: both pass.

- [ ] **Step 8: Commit descriptor and resolver support**

```bash
git add crates/puffer-provider-registry/src/model.rs \
  crates/puffer-provider-registry/src/model_tests.rs \
  crates/puffer-core/runtime/media/capabilities.rs \
  crates/puffer-core/runtime/media/resolver.rs \
  crates/puffer-core/media_runtime_tests.rs
git commit -m "feat(media): resolve exact video capabilities"
```

---

### Task 3: Exact Media Runtime Facade

**Files:**
- Modify: `crates/puffer-core/media_runtime.rs`
- Modify: `crates/puffer-core/runtime/media/jobs.rs`
- Modify: `crates/puffer-core/runtime/media/replicate_video.rs`
- Modify: `crates/puffer-core/media_runtime_tests.rs`

- [ ] **Step 1: Write failing facade tests**

Add tests in `media_runtime_tests.rs`:

```rust
#[test]
fn exact_media_generation_rejects_unsupported_video_parameter() {
    let (registry, auth, cache, workspace) = replicate_video_runtime_fixture();
    let request = ExactMediaGenerationRequest {
        kind: "video".to_string(),
        provider_id: "replicate".to_string(),
        model_id: "owner/model-version".to_string(),
        operation: "generate".to_string(),
        adapter: "replicate_video".to_string(),
        prompt: "animate a logo".to_string(),
        parameters: BTreeMap::from([
            ("aspect_ratio".to_string(), "1:1".to_string()),
            ("duration".to_string(), "5".to_string()),
        ]),
        count: 1,
    };

    let error = generate_exact_media_with_cache(&registry, &auth, workspace.path(), request, &cache)
        .unwrap_err()
        .to_string();
    assert!(error.contains("video generation parameter unsupported: aspect_ratio=1:1"));
}

#[test]
fn exact_media_generation_rejects_unsupported_adapter_before_http() {
    let (registry, auth, cache, workspace) = replicate_video_runtime_fixture();
    let request = ExactMediaGenerationRequest {
        kind: "video".to_string(),
        provider_id: "replicate".to_string(),
        model_id: "owner/model-version".to_string(),
        operation: "generate".to_string(),
        adapter: "images_json".to_string(),
        prompt: "animate a logo".to_string(),
        parameters: BTreeMap::from([
            ("aspect_ratio".to_string(), "16:9".to_string()),
            ("duration".to_string(), "5".to_string()),
        ]),
        count: 1,
    };

    let error = generate_exact_media_with_cache(&registry, &auth, workspace.path(), request, &cache)
        .unwrap_err()
        .to_string();
    assert!(error.contains("selected video model unavailable"));
}
```

- [ ] **Step 2: Run facade tests to verify failure**

Run:

```bash
cargo test -p puffer-core exact_media_generation_
```

Expected: fail because `ExactMediaGenerationRequest` and
`generate_exact_media_with_cache()` do not exist.

- [ ] **Step 3: Extend job metadata**

In `jobs.rs`, add fields:

```rust
pub(crate) adapter: Option<String>,
pub(crate) parameters: std::collections::BTreeMap<String, String>,
```

Initialize them in `MediaJob::new()`:

```rust
adapter: None,
parameters: std::collections::BTreeMap::new(),
```

In adapters, set these fields immediately after `MediaJob::new()`:

```rust
job.adapter = Some(request.adapter.clone());
job.parameters = request.parameters.clone();
```

For Replicate, set:

```rust
job.adapter = Some("replicate_video".to_string());
job.parameters = BTreeMap::from([
    ("aspect_ratio".to_string(), request.aspect_ratio.clone()),
    ("duration".to_string(), request.duration_seconds.to_string()),
]);
```

- [ ] **Step 4: Add generic selection validation**

In `resolver.rs`, add:

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

Add `validate_media_generate_selection()` that resolves capabilities for the
selection kind and checks provider/model/adapter plus all selected parameters.
Keep `validate_image_generate_selection()` as a wrapper around the generic
function so existing image adapters remain readable.

- [ ] **Step 5: Add exact media facade types**

In `media_runtime.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactMediaGenerationResult {
    pub job_id: String,
    pub requested_count: u8,
    pub artifacts: Vec<ExactGeneratedArtifact>,
    pub kind: String,
    pub provider_id: String,
    pub model_id: String,
    pub status: String,
}
```

- [ ] **Step 6: Implement facade dispatch**

Add `generate_exact_media_with_cache()`:

```rust
pub fn generate_exact_media_with_cache(
    registry: &ProviderRegistry,
    auth_store: &AuthStore,
    workspace_root: &Path,
    request: ExactMediaGenerationRequest,
    discovery_cache: &ExactMediaDiscoveryCache,
) -> Result<ExactMediaGenerationResult> {
    match request.kind.trim() {
        "image" => generate_exact_image_from_media_request(
            registry,
            auth_store,
            workspace_root,
            request,
            discovery_cache,
        ),
        "video" => generate_exact_video_from_media_request(
            registry,
            auth_store,
            workspace_root,
            request,
            discovery_cache,
        ),
        kind => bail!("unsupported media kind `{kind}`"),
    }
}
```

Implement image dispatch by converting to `ExactImageGenerationRequest` and
calling the existing image function.

Implement video dispatch only for `replicate_video`. Convert capability
parameters to `ReplicateVideoRequest`:

```rust
fn replicate_video_request_from_parameters(
    model_id: String,
    prompt: String,
    parameters: BTreeMap<String, String>,
) -> Result<ReplicateVideoRequest> {
    let aspect_ratio = parameters
        .get("aspect_ratio")
        .cloned()
        .context("video generation parameter unsupported: aspect_ratio=")?;
    let duration_seconds = parameters
        .get("duration")
        .context("video generation parameter unsupported: duration=")?
        .parse::<u32>()
        .context("video generation parameter unsupported: duration")?;
    Ok(ReplicateVideoRequest {
        model: model_id,
        prompt,
        aspect_ratio,
        duration_seconds,
    })
}
```

- [ ] **Step 7: Run runtime tests**

Run:

```bash
cargo test -p puffer-core media_runtime
cargo test -p puffer-core replicate_video
```

Expected: pass.

- [ ] **Step 8: Commit runtime facade**

```bash
git add crates/puffer-core/media_runtime.rs \
  crates/puffer-core/runtime/media/jobs.rs \
  crates/puffer-core/runtime/media/replicate_video.rs \
  crates/puffer-core/media_runtime_tests.rs
git commit -m "feat(media): add exact media generation facade"
```

---

### Task 4: Daemon And Tauri Media API Wiring

**Files:**
- Modify: `crates/puffer-cli/src/daemon.rs`
- Modify: `crates/puffer-cli/src/desktop_api.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/backend.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/media_capabilities.rs`

- [ ] **Step 1: Write failing daemon tests**

Add tests near existing daemon media tests:

```rust
#[test]
fn daemon_list_media_capabilities_returns_video_capability() {
    let state = daemon_state_with_replicate_video_capability();
    let response = handle_list_media_capabilities(&state, &json!({"kind": "video"}))
        .expect("response");
    let capabilities = response["capabilities"].as_array().expect("capabilities");
    assert_eq!(capabilities.len(), 1);
    assert_eq!(capabilities[0]["adapter"], "replicate_video");
}

#[test]
fn daemon_generate_media_requires_video_adapter_setting() {
    let mut state = daemon_state_with_replicate_video_capability();
    state.config.media.video = Some(MediaGenerationConfig {
        provider_id: "replicate".to_string(),
        model_id: "owner/model-version".to_string(),
        operation: "generate".to_string(),
        adapter: "images_json".to_string(),
        parameters: BTreeMap::from([
            ("aspect_ratio".to_string(), "16:9".to_string()),
            ("duration".to_string(), "5".to_string()),
        ]),
    });

    let error = handle_generate_media(&state, &json!({"kind": "video", "prompt": "animate"}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("selected video model unavailable"));
}
```

- [ ] **Step 2: Run daemon tests to verify failure**

Run:

```bash
cargo test -p puffer-cli daemon_list_media_capabilities_returns_video_capability
cargo test -p puffer-cli daemon_generate_media_requires_video_adapter_setting
```

Expected: fail because daemon still maps video config through provider/model
plus aspect/duration.

- [ ] **Step 3: Convert settings snapshot mapping**

In `desktop_api.rs` and daemon settings serialization, map config selections:

```rust
fn media_selection_dto(
    selection: &Option<MediaGenerationConfig>,
) -> Option<MediaGenerationSettingsDto> {
    selection.as_ref().map(|selection| MediaGenerationSettingsDto {
        provider_id: selection.provider_id.clone(),
        model_id: selection.model_id.clone(),
        operation: selection.operation.clone(),
        adapter: selection.adapter.clone(),
        parameters: selection.parameters.clone(),
    })
}
```

- [ ] **Step 4: Convert `handle_generate_media`**

Build `ExactMediaGenerationRequest` from the selected media branch:

```rust
let selection = match kind {
    "image" => state.config.media.image.as_ref(),
    "video" => state.config.media.video.as_ref(),
    _ => bail!("unsupported media kind `{kind}`"),
}
.with_context(|| format!("{kind} media provider/model is not configured"))?;

let result = generate_exact_media_with_cache(
    &state.provider_registry,
    &state.auth_store,
    &state.workspace_root,
    ExactMediaGenerationRequest {
        kind: kind.to_string(),
        provider_id: selection.provider_id.clone(),
        model_id: selection.model_id.clone(),
        operation: selection.operation.clone(),
        adapter: selection.adapter.clone(),
        prompt,
        parameters: selection.parameters.clone(),
        count,
    },
    &state.media_discovery_cache,
)?;
```

Return the existing `GenerateMediaResult` JSON shape using the new result.

- [ ] **Step 5: Convert Tauri fallback backend**

Apply the same mapping in `apps/puffer-desktop/src-tauri/src/backend.rs`.
The fallback backend may keep using fake local image bytes for tests only, but
its DTO and validation path must use `adapter` and `parameters` for both kinds.

- [ ] **Step 6: Run API tests**

Run:

```bash
cargo test -p puffer-cli generate_media
cargo test -p corbina generate_media
```

Expected: daemon/Tauri media tests pass.

- [ ] **Step 7: Commit API wiring**

```bash
git add crates/puffer-cli/src/daemon.rs \
  crates/puffer-cli/src/desktop_api.rs \
  apps/puffer-desktop/src-tauri/src/backend.rs \
  apps/puffer-desktop/src-tauri/src/media_capabilities.rs
git commit -m "feat(media): wire exact video generation api"
```

---

### Task 5: Parameter-Driven Desktop Settings UI

**Files:**
- Modify: `apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
- Modify: `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
- Modify: `apps/puffer-desktop/tests/support/fakeDaemon.ts`

- [ ] **Step 1: Write failing Playwright tests**

Add tests:

```ts
test("composer video generation settings saves capability parameters", async ({ page }) => {
  const daemon = new FakeDaemon({
    mediaCapabilities: [configurableVideoCapability()],
    sessions: [mediaSettingsSession("session-video-configurable", "Configurable video settings")]
  });
  await daemon.install(page);
  await daemon.open(page);
  await openSession(page, /Configurable video settings/);

  await page.getByRole("button", { name: "Add content" }).click();
  await page.getByRole("menuitem", { name: "Video generation settings" }).click();

  const dialog = page.getByRole("dialog", { name: "Video generation settings" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Aspect ratio").selectOption("9:16");
  await dialog.getByLabel("Duration").selectOption("8");
  await dialog.getByRole("button", { name: "Save" }).click();

  const update = await daemon.waitForRequest(
    "update_config",
    (request) => "media" in request.params
  );
  expect(update.params.media.video).toEqual({
    providerId: "replicate",
    modelId: "owner/model-version",
    operation: "generate",
    adapter: "replicate_video",
    parameters: {
      aspect_ratio: "9:16",
      duration: "8"
    }
  });
});
```

Also assert no button named `Save video generation settings` exists:

```ts
await expect(dialog.getByRole("button", { name: "Save video generation settings" })).toHaveCount(0);
```

- [ ] **Step 2: Run focused UI tests to verify failure**

Run:

```bash
pnpm --dir apps/puffer-desktop exec playwright test tests/chat-session-ui.spec.ts -g "video generation settings"
```

Expected: fail because the modal still uses video-specific state or because
the fake daemon has not been converted to the unified settings shape.

- [ ] **Step 3: Convert modal state**

In `MediaSettingsModal.svelte`, use one selection state:

```ts
let providerId = $state(initialSaved?.providerId ?? "");
let modelId = $state(initialSaved?.modelId ?? "");
let adapter = $state(initialSaved?.adapter ?? "");
let operation = $state(initialSaved?.operation ?? "generate");
let parameters = $state<Record<string, string>>({ ...(initialSaved?.parameters ?? {}) });
```

Remove `aspectRatio` and `durationSeconds` state.

- [ ] **Step 4: Make parameter rendering generic**

Use the same parameter block for image and video:

```svelte
{#if selectedCapability}
  {#each selectedCapability.parameters as parameter (parameter.name)}
    {#if parameter.values.length > 1}
      <label class="pf-media-field">
        <span class="pf-field-label">{parameter.label}</span>
        <select
          class="sc-input"
          value={parameterValue(parameter)}
          onchange={(event) => setParameterValue(parameter.name, event.currentTarget.value)}
        >
          {#each parameter.values as option}
            <option value={option}>{parameterDisplayValue(parameter, option)}</option>
          {/each}
        </select>
      </label>
    {:else}
      {@render readOnlyField(parameter.label, parameterDisplayValue(parameter, parameterValue(parameter)))}
    {/if}
  {/each}
{/if}
```

Implement `parameterDisplayValue()` with only formatting, not behavior:

```ts
function parameterDisplayValue(parameter: MediaCapabilityParameterInfo, value: string): string {
  return parameter.name === "duration" && /^\d+$/.test(value) ? `${value}s` : value;
}
```

- [ ] **Step 5: Save unified media settings**

Build the saved selection:

```ts
function withCurrentSelection(): MediaGenerationSettings {
  const normalized = selectedCapability
    ? normalizeParameters(selectedCapability, parameters)
    : { ...parameters };
  return {
    providerId,
    modelId,
    operation: "generate",
    adapter,
    parameters: normalized
  };
}
```

For `kind === "image"`, save `{ image: withCurrentSelection(), video: settings.video }`.
For `kind === "video"`, save `{ image: settings.image, video: withCurrentSelection() }`.

- [ ] **Step 6: Run UI checks**

Run:

```bash
pnpm --dir apps/puffer-desktop check
pnpm --dir apps/puffer-desktop exec playwright test tests/chat-session-ui.spec.ts -g "generation settings"
```

Expected: pass.

- [ ] **Step 7: Commit desktop settings UI**

```bash
git add apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte \
  apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte \
  apps/puffer-desktop/tests/chat-session-ui.spec.ts \
  apps/puffer-desktop/tests/support/fakeDaemon.ts
git commit -m "feat(desktop): render media settings from capabilities"
```

---

### Task 6: Generated Video Attachment Metadata

**Files:**
- Modify: `crates/puffer-core/media_runtime.rs`
- Modify: `apps/puffer-desktop/src/lib/types.ts`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/MessageAttachmentPreviewStrip.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/AttachmentPreviewStrip.svelte`
- Modify: `apps/puffer-desktop/tests/chat-session-ui.spec.ts`

- [ ] **Step 1: Write failing metadata tests**

Add a core test:

```rust
#[test]
fn generated_video_attachment_metadata_is_metadata_only() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let service = MediaGenerationService::new(workspace.path());
    let artifact = MediaArtifact {
        id: "video-artifact".to_string(),
        job_id: "job-video".to_string(),
        kind: MediaKind::Video,
        path: workspace.path().join(".puffer/media/artifacts/video-artifact/video.mp4"),
        mime_type: "video/mp4".to_string(),
        byte_count: 12,
        metadata: serde_json::json!({ "provider": "replicate" }),
        created_at_ms: 42,
    };
    service.save_artifact(&artifact).expect("save artifact");

    let metadata = generated_media_attachment_metadata(workspace.path(), "video-artifact")
        .expect("metadata");
    assert_eq!(metadata.mime_type, "video/mp4");
    assert_eq!(metadata.byte_count, 12);
    assert_eq!(metadata.state, "available");
}
```

- [ ] **Step 2: Run metadata test to verify failure**

Run:

```bash
cargo test -p puffer-core generated_video_attachment_metadata_is_metadata_only
```

Expected: fail because generated media attachment metadata currently rejects
non-image artifacts.

- [ ] **Step 3: Generalize generated media metadata**

Update `generated_media_attachment_metadata_from_artifact()` to accept image and
video. For video:

```rust
if artifact.kind == MediaKind::Video {
    return Some(GeneratedMediaAttachmentMetadata {
        artifact_id: artifact.id,
        mime_type: artifact.mime_type,
        byte_count: artifact.byte_count,
        state: "available".to_string(),
        local_path: Some(artifact.path.display().to_string()),
        remote_source_url: media_artifact_remote_source_url(&artifact),
    });
}
```

Keep `read_generated_media_preview_by_artifact()` image-only. It should return
`Unsupported` for video.

- [ ] **Step 4: Update desktop attachment rendering**

When attachment `kind === "video"` and preview is unsupported, render a compact
metadata row with name, MIME type, and action buttons already used for local
files. Do not call preview byte loading for video thumbnails.

Use text:

```ts
const extension = attachment.kind === "video" ? "MP4" : attachment.extension;
```

Keep generated image previews unchanged.

- [ ] **Step 5: Run attachment tests**

Run:

```bash
cargo test -p puffer-core generated_media_attachment
pnpm --dir apps/puffer-desktop exec playwright test tests/chat-session-ui.spec.ts -g "generated.*attachment"
```

Expected: pass.

- [ ] **Step 6: Commit video attachment metadata**

```bash
git add crates/puffer-core/media_runtime.rs \
  apps/puffer-desktop/src/lib/types.ts \
  apps/puffer-desktop/src/lib/screens/agent/MessageAttachmentPreviewStrip.svelte \
  apps/puffer-desktop/src/lib/screens/agent/AttachmentPreviewStrip.svelte \
  apps/puffer-desktop/tests/chat-session-ui.spec.ts
git commit -m "feat(media): show generated video artifact metadata"
```

---

### Task 7: Component Specs And Final Verification

**Files:**
- Create: `specs/puffer-config/08.md`
- Create: `specs/puffer-provider-registry/08.md`
- Create: `specs/puffer-core/256.md`
- Create: `specs/puffer-cli/165.md`
- Create: `specs/puffer-desktop/680.md`

- [ ] **Step 1: Write component update specs**

Create each file with the listed title, summary, compatibility note, and test
commands. Expand the contracts section with the exact structs and commands
changed during implementation.

- `specs/puffer-config/08.md`
  - Title: `puffer-config Update 08: Unified Media Generation Defaults`
  - Summary: `MediaConfig now stores image and video defaults as optional MediaGenerationConfig selections with provider, model, operation, adapter, and parameters.`
  - Compatibility: `The old split VideoMediaConfig aspect_ratio/duration_seconds shape is intentionally removed.`
  - Tests: `cargo test -p puffer-config`
- `specs/puffer-provider-registry/08.md`
  - Title: `puffer-provider-registry Update 08: Image And Video Media Descriptors`
  - Summary: `ProviderMediaDescriptor now supports image and video kind descriptors and validates replicate_video execution metadata.`
  - Compatibility: `ImageMediaDescriptor is replaced by the shared MediaKindDescriptor target shape.`
  - Tests: `cargo test -p puffer-provider-registry provider_media_descriptor_`
- `specs/puffer-core/256.md`
  - Title: `puffer-core Update 256: Exact Media Runtime V1`
  - Summary: `The media runtime validates generic media selections, resolves descriptor-backed video capabilities, and dispatches image/video generation through existing adapters.`
  - Compatibility: `Existing exact image wrappers remain, while new generic media facade APIs become the daemon-facing path.`
  - Tests: `cargo test -p puffer-core media_runtime` and `cargo test -p puffer-core replicate_video`
- `specs/puffer-cli/165.md`
  - Title: `puffer-cli Update 165: Unified Media Daemon API`
  - Summary: `The daemon accepts unified media settings, lists exact video capabilities, and routes generate_media through the exact media runtime facade.`
  - Compatibility: `The daemon no longer accepts the old video aspectRatio/durationSeconds settings shape.`
  - Tests: `cargo test -p puffer-cli generate_media`
- `specs/puffer-desktop/680.md`
  - Title: `puffer-desktop Update 680: Parameter-Driven Media Settings`
  - Summary: `The desktop media settings modal renders image and video parameters from capabilities and stores unified selections.`
  - Compatibility: `Video settings no longer persist dedicated aspectRatio or durationSeconds fields.`
  - Tests: `pnpm --dir apps/puffer-desktop check` and `pnpm --dir apps/puffer-desktop exec playwright test tests/chat-session-ui.spec.ts -g "generation settings|generated.*attachment"`

- [ ] **Step 2: Run full verification**

Run:

```bash
cargo test -p puffer-config
cargo test -p puffer-provider-registry
cargo test -p puffer-core media
cargo test -p puffer-cli generate_media
pnpm --dir apps/puffer-desktop check
pnpm --dir apps/puffer-desktop exec playwright test tests/chat-session-ui.spec.ts -g "generation settings|generated.*attachment"
```

Expected: all pass.

- [ ] **Step 3: Run workspace gate when local time permits**

Run:

```bash
cargo test --workspace
```

Expected: pass. If unrelated pre-existing failures appear, record the failing
test names and the first error line in the final implementation handoff.

- [ ] **Step 4: Inspect final diff**

Run:

```bash
git diff --stat
git diff --check
git status --short
```

Expected:

- `git diff --check` prints no whitespace errors.
- `git status --short` contains only files related to exact media runtime V1.

- [ ] **Step 5: Commit specs and final polish**

```bash
git add specs/puffer-config/08.md \
  specs/puffer-provider-registry/08.md \
  specs/puffer-core/256.md \
  specs/puffer-cli/165.md \
  specs/puffer-desktop/680.md
git commit -m "docs(media): record exact media runtime component updates"
```

---

## Stop Conditions

Stop and revisit the design before continuing if implementation requires:

- a new workspace crate;
- a generic media workflow engine;
- a generic adapter trait shared by all providers;
- dynamic video model discovery;
- provider-specific desktop settings UI;
- detached background video polling;
- object storage;
- transcript-embedded media bytes.

These are outside V1 and indicate the slice has become too broad.
