# Chat Image Generation Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a chat composer image button that selects a connected provider image model and routes `ImageGeneration` tool calls through that selected model.

**Architecture:** Keep this as a routing feature. Provider metadata exposes a narrow `supports_image_generation` model flag and an image generation endpoint path; the desktop composer stores a session-local image route; `AppState` carries the effective route for the active turn; `ImageGeneration` resolves provider credentials and writes the generated image through the existing tool-card flow.

**Tech Stack:** Rust workspace crates (`puffer-provider-registry`, `puffer-cli`, `puffer-core`), Svelte 5 desktop frontend, Vitest, Cargo unit tests, existing provider registry/auth store/tool execution stack.

---

## File Map

- `crates/puffer-provider-registry/src/model.rs`
  - Add `supports_image_generation` to `ModelDescriptor`.
  - Add `ImageGenerationEndpoint` and `ProviderDescriptor::image_generation`.
- `crates/puffer-provider-registry/src/discovery.rs`
  - Preserve static image-generation models across chat model discovery.
- `resources/providers/openai.yaml`
  - Add OpenAI image endpoint and curated image models.
- `resources/providers/byteplus.yaml`
  - Add BytePlus image endpoint and curated Seedream image model entries.
- `crates/puffer-cli/src/desktop_api_types.rs`
  - Add `supports_tools`, `supports_image_generation`, and provider `supports_image_generation` DTO fields.
- `crates/puffer-cli/src/desktop_api.rs`
  - Populate provider `supports_image_generation` from descriptor endpoint metadata.
- `crates/puffer-cli/src/daemon.rs`
  - Emit DTO image fields.
  - Parse and apply `imageGeneration` turn options.
- `crates/puffer-core/state.rs`
  - Add `ImageGenerationRoute` and `AppState::image_generation_route`.
- `crates/puffer-core/runtime/system_prompt.rs`
  - Add concise selected image route guidance.
- `crates/puffer-core/runtime/tool_executor.rs`
  - Special-case `ImageGeneration` so it receives provider/auth context without widening all workflow tool signatures.
- `crates/puffer-core/runtime/claude_tools/mod.rs`
  - Remove the old image-generation workflow arm after the executor special-case owns dispatch.
- `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
  - Remove env-only routing.
  - Resolve image endpoint, credential, and selected model from `AppState`, `ProviderRegistry`, and `AuthStore`.
- `resources/tools/image_generation.yaml`
  - Update description to describe selected chat image route.
- `apps/puffer-desktop/src/lib/api/desktop.ts`
  - Add image route option and DTO fields.
- `apps/puffer-desktop/src/lib/types.ts`
  - Add `supportsImageGeneration` to `ProviderSummary`.
- `apps/puffer-desktop/src/lib/providerIds.ts`
  - Add image-generation provider availability helper.
- `apps/puffer-desktop/src/lib/providerIds.test.ts`
  - Test image-generation provider availability helper.
- `apps/puffer-desktop/src/lib/screens/agent/composerState.ts`
  - Add pure image route helpers.
- `apps/puffer-desktop/src/lib/screens/agent/composerState.test.ts`
  - Test image route serialization/filter helpers.
- `apps/puffer-desktop/src/lib/screens/agent/ModelPicker.svelte`
  - Add provider/model filter props and icon/label props.
- `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
  - Add image route state, image button, and `composerOptions()` wiring.
- `apps/puffer-desktop/src/lib/design/Icon.svelte`
  - Add a lucide image icon.
- `apps/puffer-desktop/src-tauri/src/lib.rs`
  - Pass `imageGeneration` through the Tauri command fallback.

---

### Task 1: Provider Metadata And Discovery

**Files:**
- Modify: `crates/puffer-provider-registry/src/model.rs`
- Modify: `crates/puffer-provider-registry/src/discovery.rs`
- Modify: `resources/providers/openai.yaml`
- Modify: `resources/providers/byteplus.yaml`

- [ ] **Step 1: Write failing provider metadata tests**

Add this test module to the bottom of `crates/puffer-provider-registry/src/model.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_descriptor_defaults_image_generation_support_to_false() {
        let value = serde_json::json!({
            "id": "gpt-5.4",
            "display_name": "GPT-5.4",
            "provider": "openai",
            "api": "openai-responses",
            "context_window": 1000,
            "max_output_tokens": 100
        });

        let model: ModelDescriptor = serde_json::from_value(value).unwrap();

        assert!(!model.supports_image_generation);
    }

    #[test]
    fn provider_descriptor_parses_image_generation_endpoint() {
        let value = serde_json::json!({
            "id": "openai",
            "display_name": "OpenAI",
            "base_url": "https://api.openai.com",
            "default_api": "openai-responses",
            "image_generation": { "path": "/v1/images/generations" },
            "models": []
        });

        let provider: ProviderDescriptor = serde_json::from_value(value).unwrap();

        assert_eq!(
            provider.image_generation.as_ref().map(|endpoint| endpoint.path.as_str()),
            Some("/v1/images/generations")
        );
    }
}
```

- [ ] **Step 2: Write failing discovery preservation test**

Add this helper and test to the existing test module in `crates/puffer-provider-registry/src/discovery.rs`:

```rust
fn discovered_test_model(id: &str, api: &str, image: bool) -> ModelDescriptor {
    ModelDescriptor {
        id: id.to_string(),
        display_name: id.to_string(),
        provider: "openai".to_string(),
        api: api.to_string(),
        context_window: 1000,
        max_output_tokens: 100,
        supports_reasoning: false,
        input: vec![crate::Modality::Text],
        cost: None,
        compat: None,
        supports_image_generation: image,
    }
}

#[test]
fn merge_discovered_models_preserves_static_image_models() {
    let mut existing = vec![
        discovered_test_model("gpt-5", "openai-responses", false),
        discovered_test_model("gpt-image-1", "openai-images", true),
    ];
    let discovered = vec![discovered_test_model(
        "gpt-5.4",
        "openai-responses",
        false,
    )];

    merge_discovered_models(&mut existing, discovered);

    let ids = existing.iter().map(|model| model.id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids, vec!["gpt-5.4", "gpt-image-1"]);
}
```

- [ ] **Step 3: Run provider registry tests and verify failure**

Run:

```bash
cargo test -p puffer-provider-registry model_descriptor_defaults_image_generation_support_to_false
cargo test -p puffer-provider-registry merge_discovered_models_preserves_static_image_models
```

Expected: FAIL because `supports_image_generation` and `image_generation` do not exist.

- [ ] **Step 4: Add narrow image metadata**

In `crates/puffer-provider-registry/src/model.rs`, add this field to `ModelDescriptor` after `supports_reasoning`:

```rust
    /// Whether this model can generate images through the provider's image
    /// generation endpoint. This is output capability, not input modality.
    #[serde(default)]
    pub supports_image_generation: bool,
```

Add this provider endpoint type near `ProviderDescriptor`:

```rust
/// OpenAI-compatible image generation endpoint metadata for a provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageGenerationEndpoint {
    pub path: String,
}
```

Add this field to `ProviderDescriptor` after `chat_completions_path`:

```rust
    #[serde(default)]
    pub image_generation: Option<ImageGenerationEndpoint>,
```

Update `crates/puffer-provider-registry/src/lib.rs` to export `ImageGenerationEndpoint`.

- [ ] **Step 5: Update discovery-created models**

In `parse_discovered_models`, add the new field:

```rust
            supports_image_generation: false,
```

In `merge_discovered_models`, preserve static image models:

```rust
    let previous = std::mem::take(existing);
    let preserved_images = previous
        .iter()
        .filter(|model| model.supports_image_generation)
        .filter(|model| !discovered.iter().any(|next| next.id == model.id))
        .cloned()
        .collect::<Vec<_>>();
    for model in discovered {
        if let Some(current) = previous.iter().find(|current| current.id == model.id) {
            existing.push(current.clone());
        } else {
            existing.push(model);
        }
    }
    existing.extend(preserved_images);
```

- [ ] **Step 6: Add provider resource metadata**

In `resources/providers/openai.yaml`, add:

```yaml
image_generation:
  path: /v1/images/generations
```

Add curated image model entries:

```yaml
  - id: gpt-image-1
    display_name: GPT Image 1
    provider: openai
    api: openai-images
    context_window: 0
    max_output_tokens: 0
    supports_reasoning: false
    supports_image_generation: true
```

In `resources/providers/byteplus.yaml`, add:

```yaml
image_generation:
  path: /images/generations
models:
  - id: doubao-seedream-4-0-250828
    display_name: Doubao Seedream 4.0
    provider: byteplus
    api: openai-images
    context_window: 0
    max_output_tokens: 0
    supports_reasoning: false
    supports_image_generation: true
```

If `byteplus.yaml` already has a `models:` section after implementation, merge the model into that section instead of adding a second key.

- [ ] **Step 7: Run provider registry tests**

Run:

```bash
cargo test -p puffer-provider-registry
```

Expected: PASS.

- [ ] **Step 8: Commit provider metadata**

Run:

```bash
git add crates/puffer-provider-registry/src/model.rs crates/puffer-provider-registry/src/lib.rs crates/puffer-provider-registry/src/discovery.rs resources/providers/openai.yaml resources/providers/byteplus.yaml
git commit -m "feat(provider): add image generation model metadata"
```

---

### Task 2: Desktop DTOs And Turn Route State

**Files:**
- Modify: `crates/puffer-cli/src/desktop_api_types.rs`
- Modify: `crates/puffer-cli/src/desktop_api.rs`
- Modify: `crates/puffer-cli/src/daemon.rs`
- Modify: `crates/puffer-core/state.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/lib.rs`

- [ ] **Step 1: Write failing daemon DTO test**

Add this test near existing `model_descriptor_dto_*` tests in `crates/puffer-cli/src/daemon.rs`:

```rust
#[test]
fn model_descriptor_dto_exposes_image_generation_support() {
    let mut provider = provider_with_models("openai", vec!["gpt-image-1"]);
    provider.models[0].api = "openai-images".to_string();
    provider.models[0].supports_image_generation = true;

    let dto = model_descriptor_dto(ModelPreferenceFamily::OpenAi, &provider.models[0]);
    let value = serde_json::to_value(dto).unwrap();

    assert_eq!(value["supportsImageGeneration"], true);
    assert_eq!(value["supportsTools"], false);
}
```

- [ ] **Step 2: Write failing turn option parsing test**

Add this test near existing `TurnRequestOptions::from_params` tests:

```rust
#[test]
fn turn_options_parse_image_generation_route() {
    let options = TurnRequestOptions::from_params(&json!({
        "imageGeneration": {
            "providerId": "codex",
            "modelId": "gpt-image-1"
        }
    }))
    .expect("turn options");

    assert_eq!(
        options.image_generation.as_ref().map(|route| route.provider_id.as_str()),
        Some("openai")
    );
    assert_eq!(
        options.image_generation.as_ref().map(|route| route.model_id.as_str()),
        Some("gpt-image-1")
    );
}
```

Add this test beside it:

```rust
#[test]
fn turn_options_reject_partial_image_generation_route() {
    let error = TurnRequestOptions::from_params(&json!({
        "imageGeneration": {
            "providerId": "openai"
        }
    }))
    .unwrap_err()
    .to_string();

    assert!(error.contains("imageGeneration.modelId"));
}
```

- [ ] **Step 3: Run daemon tests and verify failure**

Run:

```bash
cargo test -p puffer-cli model_descriptor_dto_exposes_image_generation_support
cargo test -p puffer-cli turn_options_parse_image_generation_route
cargo test -p puffer-cli turn_options_reject_partial_image_generation_route
```

Expected: FAIL because DTO fields and turn route parsing do not exist.

- [ ] **Step 4: Add route type to core state**

In `crates/puffer-core/state.rs`, add:

```rust
/// Selected provider/model route for ImageGeneration tool calls in this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageGenerationRoute {
    pub provider_id: String,
    pub model_id: String,
}
```

Add this field to `AppState`:

```rust
    pub image_generation_route: Option<ImageGenerationRoute>,
```

Initialize it to `None` in `AppState::new` and preserve it in `from_session_record` by using the default `None` value for restored sessions.

- [ ] **Step 5: Add DTO fields**

In `crates/puffer-cli/src/desktop_api_types.rs`, add fields to `ModelDescriptorDto`:

```rust
    pub(crate) supports_tools: bool,
    pub(crate) supports_image_generation: bool,
```

Add `supports_image_generation: bool` to `ProviderSummaryDto`.

In `model_descriptor_dto`, set:

```rust
        supports_tools: !model.supports_image_generation,
        supports_image_generation: model.supports_image_generation,
```

In `crates/puffer-cli/src/desktop_api.rs`, set `supports_image_generation` in
`provider_summary` from `entry.descriptor.image_generation.is_some()`.

- [ ] **Step 6: Parse and apply image route options**

In `crates/puffer-cli/src/daemon.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopImageGenerationRoute {
    provider_id: String,
    model_id: String,
}
```

Add this field to `TurnRequestOptions`:

```rust
    image_generation: Option<DesktopImageGenerationRoute>,
```

Add this helper:

```rust
fn image_generation_route_from_params(params: &Value) -> Result<Option<DesktopImageGenerationRoute>> {
    let Some(value) = params
        .get("imageGeneration")
        .or_else(|| params.get("image_generation")) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let provider_id = optional_trimmed_value(value, &["providerId", "provider_id"])
        .map(|provider_id| canonical_desktop_provider_id(&provider_id))
        .context("imageGeneration.providerId is required")?;
    let model_id = optional_trimmed_value(value, &["modelId", "model_id"])
        .context("imageGeneration.modelId is required")?;
    Ok(Some(DesktopImageGenerationRoute { provider_id, model_id }))
}
```

Change `TurnRequestOptions::from_params` to return `Result<Self>` and set:

```rust
            image_generation: image_generation_route_from_params(params)?,
```

Change the `start_turn` call site from:

```rust
let turn_options = TurnRequestOptions::from_params(&params);
```

to:

```rust
let turn_options = TurnRequestOptions::from_params(&params)?;
```

Update existing daemon tests that call `TurnRequestOptions::from_params(...)`
to append `.expect("turn options")`.

In `apply_turn_request_options`, after normal model routing, validate the selected image model and write:

```rust
    if let Some(route) = options.image_generation.as_ref() {
        let provider = providers
            .provider(&route.provider_id)
            .with_context(|| format!("provider {} not found", route.provider_id))?;
        let model = provider
            .models
            .iter()
            .find(|model| model.id == route.model_id)
            .with_context(|| {
                format!(
                    "image generation model {}/{} not found",
                    route.provider_id, route.model_id
                )
            })?;
        if !model.supports_image_generation {
            anyhow::bail!(
                "model {}/{} does not support image generation",
                route.provider_id,
                route.model_id
            );
        }
        app_state.image_generation_route = Some(puffer_core::ImageGenerationRoute {
            provider_id: route.provider_id.clone(),
            model_id: route.model_id.clone(),
        });
    }
```

- [ ] **Step 7: Pass route through Tauri fallback**

In `apps/puffer-desktop/src-tauri/src/lib.rs`, add an `image_generation: Option<Value>` argument to the `run_agent_turn` command and include it in the backend request JSON:

```rust
            "imageGeneration": image_generation,
```

This keeps the existing `desktop.ts` fallback path aligned with daemon RPC.

- [ ] **Step 8: Run focused tests**

Run:

```bash
cargo test -p puffer-cli model_descriptor_dto_exposes_image_generation_support
cargo test -p puffer-cli turn_options_parse_image_generation_route
cargo test -p puffer-cli turn_options_reject_partial_image_generation_route
cargo test -p puffer-core state
```

Expected: PASS.

- [ ] **Step 9: Commit DTO and route state**

Run:

```bash
git add crates/puffer-cli/src/desktop_api_types.rs crates/puffer-cli/src/desktop_api.rs crates/puffer-cli/src/daemon.rs crates/puffer-core/state.rs apps/puffer-desktop/src-tauri/src/lib.rs
git commit -m "feat(chat): carry image generation route"
```

---

### Task 3: Prompt Guidance And ImageGeneration Execution

**Files:**
- Modify: `crates/puffer-core/runtime/system_prompt.rs`
- Modify: `crates/puffer-core/runtime/tool_executor.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/mod.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
- Modify: `resources/tools/image_generation.yaml`

- [ ] **Step 1: Write failing system prompt test**

Add this test to `crates/puffer-core/runtime/system_prompt.rs` tests:

```rust
#[test]
fn system_prompt_includes_selected_image_generation_route() {
    let resources = LoadedResources::default();
    let mut state = state();
    state.image_generation_route = Some(crate::state::ImageGenerationRoute {
        provider_id: "openai".to_string(),
        model_id: "gpt-image-1".to_string(),
    });

    let prompt =
        render_runtime_system_prompt(&state, &resources, "gpt-5", &BTreeSet::new()).unwrap();

    assert!(prompt.contains("Selected image generation route: openai/gpt-image-1"));
}
```

- [ ] **Step 2: Write failing image execution config tests**

In `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`, add tests:

```rust
#[test]
fn resolves_openai_image_generation_endpoint() {
    let provider = image_provider(
        "openai",
        "https://api.openai.com",
        "/v1/images/generations",
        "gpt-image-1",
    );

    assert_eq!(
        image_generation_url(&provider).unwrap(),
        "https://api.openai.com/v1/images/generations"
    );
}

#[test]
fn resolves_byteplus_image_generation_endpoint() {
    let provider = image_provider(
        "byteplus",
        "https://ark.ap-southeast.bytepluses.com/api/v3",
        "/images/generations",
        "doubao-seedream-4-0-250828",
    );

    assert_eq!(
        image_generation_url(&provider).unwrap(),
        "https://ark.ap-southeast.bytepluses.com/api/v3/images/generations"
    );
}

#[test]
fn rejects_missing_image_generation_route() {
    let state = test_state(None);
    let providers = ProviderRegistry::new();
    let auth = AuthStore::default();

    let error = resolve_image_execution_config(&state, &providers, &auth)
        .unwrap_err()
        .to_string();

    assert!(error.contains("select an image generation model"));
}

#[test]
fn rejects_oauth_image_generation_credentials() {
    let state = test_state(Some(crate::state::ImageGenerationRoute {
        provider_id: "openai".to_string(),
        model_id: "gpt-image-1".to_string(),
    }));
    let mut providers = ProviderRegistry::new();
    providers.register(image_provider(
        "openai",
        "https://api.openai.com",
        "/v1/images/generations",
        "gpt-image-1",
    ));
    let mut auth = AuthStore::default();
    auth.set_oauth("openai", OAuthCredential::default());

    let error = resolve_image_execution_config(&state, &providers, &auth)
        .unwrap_err()
        .to_string();

    assert!(error.contains("does not support OAuth credentials"));
}

#[test]
fn rejects_non_image_generation_models() {
    let state = test_state(Some(crate::state::ImageGenerationRoute {
        provider_id: "openai".to_string(),
        model_id: "gpt-5.4".to_string(),
    }));
    let mut providers = ProviderRegistry::new();
    let mut provider = image_provider(
        "openai",
        "https://api.openai.com",
        "/v1/images/generations",
        "gpt-5.4",
    );
    provider.models[0].supports_image_generation = false;
    providers.register(provider);
    let mut auth = AuthStore::default();
    auth.set_api_key("openai", "test-key");

    let error = resolve_image_execution_config(&state, &providers, &auth)
        .unwrap_err()
        .to_string();

    assert!(error.contains("does not support image generation"));
}
```

Add local test helpers in the same test module:

```rust
fn test_state(route: Option<crate::state::ImageGenerationRoute>) -> AppState {
    let mut state = crate::runtime::tests::state();
    state.image_generation_route = route;
    state
}

fn image_provider(id: &str, base_url: &str, path: &str, model_id: &str) -> ProviderDescriptor {
    ProviderDescriptor {
        id: id.to_string(),
        display_name: id.to_string(),
        base_url: base_url.to_string(),
        default_api: "openai-completions".to_string(),
        auth_modes: vec![AuthMode::ApiKey],
        headers: Default::default(),
        query_params: Default::default(),
        chat_completions_path: None,
        image_generation: Some(ImageGenerationEndpoint {
            path: path.to_string(),
        }),
        discovery: None,
        models: vec![image_model(id, model_id)],
    }
}

fn image_model(provider: &str, id: &str) -> ModelDescriptor {
    ModelDescriptor {
        id: id.to_string(),
        display_name: id.to_string(),
        provider: provider.to_string(),
        api: "openai-images".to_string(),
        context_window: 0,
        max_output_tokens: 0,
        supports_reasoning: false,
        input: vec![Modality::Text],
        supports_image_generation: true,
        cost: None,
        compat: None,
    }
}
```

- [ ] **Step 3: Run focused tests and verify failure**

Run:

```bash
cargo test -p puffer-core system_prompt_includes_selected_image_generation_route
cargo test -p puffer-core resolves_openai_image_generation_endpoint
cargo test -p puffer-core resolves_byteplus_image_generation_endpoint
cargo test -p puffer-core rejects_oauth_image_generation_credentials
cargo test -p puffer-core rejects_non_image_generation_models
```

Expected: FAIL because route prompt and image execution config do not exist.

- [ ] **Step 4: Add prompt guidance**

In `build_environment_section`, after the model line, add:

```rust
    if let Some(route) = state.image_generation_route.as_ref() {
        items.push(format!(
            "Selected image generation route: {}/{}. When the user asks you to create an image, call ImageGeneration; the tool will use this route.",
            route.provider_id, route.model_id
        ));
    }
```

- [ ] **Step 5: Replace env-only image routing with provider resolution**

In `image_generation.rs`:

- Remove `DEFAULT_MODEL`.
- Remove `openai_api_key()`.
- Add imports for `AuthStore`, `OAuthCredential` in tests, `ProviderRegistry`,
  `StoredCredential`, `ProviderDescriptor`, and `ImageGenerationEndpoint`.
- Add:

```rust
struct ImageExecutionConfig {
    url: String,
    model: String,
    bearer_token: Option<String>,
}

fn resolve_image_execution_config(
    state: &AppState,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
) -> Result<ImageExecutionConfig> {
    let route = state
        .image_generation_route
        .as_ref()
        .context("select an image generation model before using ImageGeneration")?;
    let provider = providers
        .provider(&route.provider_id)
        .with_context(|| format!("provider {} not found", route.provider_id))?;
    let model = provider
        .models
        .iter()
        .find(|model| model.id == route.model_id)
        .with_context(|| {
            format!(
                "image generation model {}/{} not found",
                route.provider_id, route.model_id
            )
        })?;
    if !model.supports_image_generation {
        bail!(
            "model {}/{} does not support image generation",
            route.provider_id,
            route.model_id
        );
    }
    let bearer_token = match auth_store.get(provider.id.as_str()) {
        Some(StoredCredential::ApiKey { key }) => Some(key.trim().to_string()),
        Some(StoredCredential::OAuth(_)) => {
            bail!("ImageGeneration does not support OAuth credentials for provider {}", provider.id)
        }
        None if provider.auth_modes.is_empty() => None,
        None => bail!("no credentials configured for provider {}", provider.id),
    };
    Ok(ImageExecutionConfig {
        url: image_generation_url(provider)?,
        model: route.model_id.clone(),
        bearer_token,
    })
}

fn image_generation_url(provider: &ProviderDescriptor) -> Result<String> {
    let endpoint = provider
        .image_generation
        .as_ref()
        .context("provider does not expose an image generation endpoint")?;
    Ok(format!(
        "{}{}",
        provider.base_url.trim_end_matches('/'),
        normalized_image_generation_path(&provider.base_url, &endpoint.path)
    ))
}

fn normalized_image_generation_path(base_url: &str, path: &str) -> String {
    if base_url.trim_end_matches('/').ends_with("/v1") && path.starts_with("/v1/") {
        path[3..].to_string()
    } else {
        path.to_string()
    }
}

fn image_http_client(proxy: &puffer_config::ProxyConfig, url: &str) -> Client {
    crate::blocking_client_for_url(
        proxy,
        crate::HttpPurpose::Model,
        url,
        Duration::from_millis(DEFAULT_TIMEOUT_MS),
    )
    .unwrap_or_else(|_| Client::new())
}
```

Change `execute_image_generation` signature to:

```rust
pub fn execute_image_generation(
    state: &mut AppState,
    providers: &ProviderRegistry,
    auth_store: &AuthStore,
    cwd: &Path,
    input: Value,
) -> Result<String>
```

Build the request with `execution.model`, create the client with
`image_http_client(&state.config.network.proxy, &execution.url)`, post to
`execution.url`, and apply bearer auth only when `bearer_token` is `Some`.
Use the same client for `image_bytes_from_response` so HTTPS image downloads
also honor the configured proxy.

- [ ] **Step 6: Special-case ImageGeneration in tool executor**

In `crates/puffer-core/runtime/tool_executor.rs`, before calling `claude_tools::execute_tool`, add:

```rust
            Ok(None) if definition.id == "ImageGeneration" => {
                let output = claude_tools::workflow::image_generation::execute_image_generation(
                    state,
                    providers,
                    auth_store,
                    cwd,
                    execution_input,
                )?;
                Ok(successful_runtime_tool(tool_id, output))
            }
            Ok(None) => claude_tools::execute_tool(
```

In `crates/puffer-core/runtime/claude_tools/mod.rs`, remove the old
`"ImageGeneration"` match arm from `execute_workflow_tool`. The
`tool_executor.rs` special-case above is the only `ImageGeneration` dispatch
path after this task.

- [ ] **Step 7: Update tool manifest wording**

In `resources/tools/image_generation.yaml`, replace the description with:

```yaml
description: |-
  Generate one image through the chat session's selected image generation
  provider/model and write the resulting image into the workspace.

  Use this when the user asks to create an image and a concrete prompt is
  available. The tool reads a workspace-relative prompt file when the prompt
  value names one; otherwise it treats prompt as literal text. Output paths
  must be workspace-relative.
```

- [ ] **Step 8: Run core tests**

Run:

```bash
cargo test -p puffer-core system_prompt_includes_selected_image_generation_route
cargo test -p puffer-core image_generation
```

Expected: PASS.

- [ ] **Step 9: Commit tool routing**

Run:

```bash
git add crates/puffer-core/runtime/system_prompt.rs crates/puffer-core/runtime/tool_executor.rs crates/puffer-core/runtime/claude_tools/mod.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs resources/tools/image_generation.yaml
git commit -m "feat(tool): route image generation through providers"
```

---

### Task 4: Frontend Image Route Selection

**Files:**
- Modify: `apps/puffer-desktop/src/lib/api/desktop.ts`
- Modify: `apps/puffer-desktop/src/lib/types.ts`
- Modify: `apps/puffer-desktop/src/lib/providerIds.ts`
- Create: `apps/puffer-desktop/src/lib/providerIds.test.ts`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/composerState.ts`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/composerState.test.ts`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/ModelPicker.svelte`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
- Modify: `apps/puffer-desktop/src/lib/design/Icon.svelte`

- [ ] **Step 1: Write failing provider helper tests**

Create `apps/puffer-desktop/src/lib/providerIds.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { providerIsAvailableForImageGeneration } from "./providerIds";

describe("providerIsAvailableForImageGeneration", () => {
  const provider = {
    id: "byteplus",
    defaultApi: "openai-completions",
    modelCount: 1,
    authModes: ["api_key"],
    supportsImageGeneration: true,
  };

  it("accepts connected providers with an image endpoint", () => {
    expect(providerIsAvailableForImageGeneration(provider, ["byteplus"])).toBe(true);
  });

  it("rejects providers without an image endpoint", () => {
    expect(
      providerIsAvailableForImageGeneration(
        { ...provider, supportsImageGeneration: false },
        ["byteplus"]
      )
    ).toBe(false);
  });

  it("rejects unconnected providers that require auth", () => {
    expect(providerIsAvailableForImageGeneration(provider, [])).toBe(false);
  });

  it("accepts no-auth image providers even when they are not agent providers", () => {
    expect(
      providerIsAvailableForImageGeneration(
        {
          ...provider,
          id: "local-image",
          defaultApi: "none",
          authModes: [],
          supportsImageGeneration: true,
        },
        []
      )
    ).toBe(true);
  });
});
```

- [ ] **Step 2: Write failing composer helper tests**

Append to `apps/puffer-desktop/src/lib/screens/agent/composerState.test.ts`:

```ts
import {
  imageGenerationRouteForOptions,
  imageModelFilter,
  parseImageGenerationRoute,
  serializeImageGenerationRoute,
} from "./composerState";

describe("image generation route helpers", () => {
  it("serializes and parses valid image routes", () => {
    const raw = serializeImageGenerationRoute({
      providerId: "openai",
      modelId: "gpt-image-1",
    });

    expect(parseImageGenerationRoute(raw)).toEqual({
      providerId: "openai",
      modelId: "gpt-image-1",
    });
  });

  it("drops invalid stored routes", () => {
    expect(parseImageGenerationRoute("{bad json")).toBeNull();
    expect(parseImageGenerationRoute(JSON.stringify({ providerId: "openai" }))).toBeNull();
  });

  it("filters image generation models only", () => {
    expect(imageModelFilter({ supportsImageGeneration: true } as any)).toBe(true);
    expect(imageModelFilter({ supportsImageGeneration: false } as any)).toBe(false);
  });

  it("omits empty routes from turn options", () => {
    expect(imageGenerationRouteForOptions(null)).toBeNull();
    expect(
      imageGenerationRouteForOptions({
        providerId: "openai",
        modelId: "gpt-image-1",
      })
    ).toEqual({ providerId: "openai", modelId: "gpt-image-1" });
  });
});
```

- [ ] **Step 3: Run frontend tests and verify failure**

Run:

```bash
cd apps/puffer-desktop
npm run test:unit -- src/lib/providerIds.test.ts src/lib/screens/agent/composerState.test.ts
```

Expected: FAIL because helper exports do not exist.

- [ ] **Step 4: Add TypeScript DTO fields**

In `apps/puffer-desktop/src/lib/api/desktop.ts`, add:

```ts
export type AgentImageGenerationRoute = {
  providerId: string;
  modelId: string;
};
```

Add to `AgentTurnOptions`:

```ts
  imageGeneration?: AgentImageGenerationRoute | null;
```

Add to `ModelDescriptorInfo`:

```ts
  supportsImageGeneration?: boolean;
```

Ensure `supportsTools?: boolean` stays present.

In `apps/puffer-desktop/src/lib/types.ts`, add to `ProviderSummary`:

```ts
  supportsImageGeneration?: boolean;
```

- [ ] **Step 5: Add image provider availability helper**

In `apps/puffer-desktop/src/lib/providerIds.ts`, add:

```ts
function providerRunsImageGenerationWithoutAuth(
  provider: { authModes?: readonly string[] | null } | null | undefined
): boolean {
  const authModes = provider?.authModes;
  return (
    Array.isArray(authModes) &&
    (authModes.length === 0 || authModes.some((mode) => mode.trim().toLowerCase() === "native"))
  );
}

export function providerIsAvailableForImageGeneration(
  provider: ProviderAuthCapability & { supportsImageGeneration?: boolean | null } | null | undefined,
  authenticatedProviderIds: Iterable<string | null | undefined>
): boolean {
  return (
    Boolean(provider?.supportsImageGeneration) &&
    (providerRunsImageGenerationWithoutAuth(provider) ||
      providerIdInSet(provider?.id, authenticatedProviderIds))
  );
}
```

- [ ] **Step 6: Add composer pure helpers**

In `apps/puffer-desktop/src/lib/screens/agent/composerState.ts`, add:

```ts
import type { AgentImageGenerationRoute, ModelDescriptorInfo } from "../../api/desktop";

export function imageModelFilter(model: Pick<ModelDescriptorInfo, "supportsImageGeneration">): boolean {
  return model.supportsImageGeneration === true;
}

export function serializeImageGenerationRoute(route: AgentImageGenerationRoute): string {
  return JSON.stringify(route);
}

export function parseImageGenerationRoute(raw: string | null): AgentImageGenerationRoute | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (
      parsed &&
      typeof parsed.providerId === "string" &&
      parsed.providerId.trim() &&
      typeof parsed.modelId === "string" &&
      parsed.modelId.trim()
    ) {
      return {
        providerId: parsed.providerId.trim(),
        modelId: parsed.modelId.trim(),
      };
    }
  } catch {
    return null;
  }
  return null;
}

export function imageGenerationRouteForOptions(
  route: AgentImageGenerationRoute | null
): AgentImageGenerationRoute | null {
  if (!route?.providerId.trim() || !route.modelId.trim()) return null;
  return {
    providerId: route.providerId.trim(),
    modelId: route.modelId.trim(),
  };
}
```

- [ ] **Step 7: Generalize `ModelPicker` minimally**

In `ModelPicker.svelte`, add props:

```ts
    providerFilter?: (
      provider: SettingsSnapshot["providers"][number],
      authenticatedProviderIds: string[]
    ) => boolean;
    modelFilter?: (model: ModelDescriptorInfo) => boolean;
    triggerIcon?: IconName;
    emptyLabel?: string;
```

Default them to existing behavior:

```ts
    providerFilter = providerIsAvailableForAgent,
    modelFilter = modelSupportsAgentTools,
    triggerIcon = "sparkles",
    emptyLabel = "models",
```

Use `providerFilter` in `availableProviders`, `modelFilter` in `filteredEntries`, default selection, and `pick()`. Replace the hardcoded icon with `triggerIcon`.

- [ ] **Step 8: Add image icon**

In `Icon.svelte`, import:

```ts
import ImageIcon from "lucide-svelte/icons/image";
```

Add `"image"` to `IconName` and map it:

```ts
image: ImageIcon,
```

- [ ] **Step 9: Wire route state into `ConversationView`**

In `ConversationView.svelte`:

- import the new helpers
- add `selectedImageRoute`
- load/save it with `sessionPreferenceKey(session.id, "image-generation-route")`
- add an image `ModelPicker` beside the main model picker
- include `imageGeneration: imageGenerationRouteForOptions(selectedImageRoute)` in `composerOptions()`

The image picker should call:

```svelte
<ModelPicker
  snapshot={settingsSnapshot}
  currentProvider={selectedImageRoute?.providerId ?? null}
  currentModel={selectedImageRoute?.modelId ?? null}
  contextKey={session?.id ? `${session.id}:image` : null}
  allowProviderSwitch={true}
  disabled={turnRunning || !session}
  providerFilter={providerIsAvailableForImageGeneration}
  modelFilter={imageModelFilter}
  triggerIcon="image"
  emptyLabel="image models"
  onChange={pickImageModel}
/>
```

- [ ] **Step 10: Run frontend tests and check**

Run:

```bash
cd apps/puffer-desktop
npm run test:unit -- src/lib/providerIds.test.ts src/lib/screens/agent/composerState.test.ts
npm run check
```

Expected: PASS.

- [ ] **Step 11: Commit frontend route selection**

Run:

```bash
git add apps/puffer-desktop/src/lib/api/desktop.ts apps/puffer-desktop/src/lib/types.ts apps/puffer-desktop/src/lib/providerIds.ts apps/puffer-desktop/src/lib/providerIds.test.ts apps/puffer-desktop/src/lib/screens/agent/composerState.ts apps/puffer-desktop/src/lib/screens/agent/composerState.test.ts apps/puffer-desktop/src/lib/screens/agent/ModelPicker.svelte apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte apps/puffer-desktop/src/lib/design/Icon.svelte
git commit -m "feat(desktop): add chat image model picker"
```

---

### Task 5: End-To-End Validation

**Files:**
- No new source files expected.
- Modify only if tests reveal an issue in files touched by Tasks 1-4.

- [ ] **Step 1: Run focused Rust tests**

Run:

```bash
cargo test -p puffer-provider-registry
cargo test -p puffer-cli model_descriptor_dto_exposes_image_generation_support
cargo test -p puffer-cli turn_options_parse_image_generation_route
cargo test -p puffer-cli turn_options_reject_partial_image_generation_route
cargo test -p puffer-core image_generation
cargo test -p puffer-core system_prompt_includes_selected_image_generation_route
```

Expected: PASS.

- [ ] **Step 2: Run focused frontend tests**

Run:

```bash
cd apps/puffer-desktop
npm run test:unit -- src/lib/providerIds.test.ts src/lib/screens/agent/composerState.test.ts
npm run check
```

Expected: PASS.

- [ ] **Step 3: Run broader workspace checks**

Run:

```bash
cargo test --workspace
```

Expected: PASS. If unrelated long-running integration tests are already known to require unavailable external services, record the exact failing test names and run the closest package-level checks instead.

- [ ] **Step 4: Manual desktop validation**

Start the desktop app:

```bash
cd apps/puffer-desktop
npm run dev
```

In another shell, start the Tauri shell when validating the desktop window:

```bash
./apps/puffer-desktop/src-tauri/target/debug/corbina
```

Manual checks:

- Chat composer shows the image picker button.
- Image picker lists only image-generation models from connected providers.
- Main chat model picker does not list image-only models.
- Selecting an image model persists for the current session.
- Sending a normal text prompt does not force image generation.
- Asking for an image causes the agent to call `ImageGeneration`.
- The tool writes a workspace image file and renders the existing image tool card.

- [ ] **Step 5: Final commit if validation required fixes**

If validation required fixes, commit them:

```bash
git add crates/puffer-provider-registry/src/model.rs crates/puffer-provider-registry/src/lib.rs crates/puffer-provider-registry/src/discovery.rs resources/providers/openai.yaml resources/providers/byteplus.yaml crates/puffer-cli/src/desktop_api_types.rs crates/puffer-cli/src/desktop_api.rs crates/puffer-cli/src/daemon.rs crates/puffer-core/state.rs crates/puffer-core/runtime/system_prompt.rs crates/puffer-core/runtime/tool_executor.rs crates/puffer-core/runtime/claude_tools/mod.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs resources/tools/image_generation.yaml apps/puffer-desktop/src/lib/api/desktop.ts apps/puffer-desktop/src/lib/types.ts apps/puffer-desktop/src/lib/providerIds.ts apps/puffer-desktop/src/lib/providerIds.test.ts apps/puffer-desktop/src/lib/screens/agent/composerState.ts apps/puffer-desktop/src/lib/screens/agent/composerState.test.ts apps/puffer-desktop/src/lib/screens/agent/ModelPicker.svelte apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte apps/puffer-desktop/src/lib/design/Icon.svelte apps/puffer-desktop/src-tauri/src/lib.rs
git commit -m "fix(chat): stabilize image generation routing"
```

If no fixes were needed, do not create an empty commit.

---

## Scope Guard

Do not implement these in this plan:

- OpenAI Responses native image-generation server tool.
- Image editing or reference-image uploads.
- Streaming partial image events.
- A generic media provider registry.
- Client-side prompt classification.
- Frontend direct calls to image APIs.
- Compatibility with old env-only image routing.

The route is considered complete when the existing `ImageGeneration` tool uses the selected connected provider/model and the regular chat model picker remains protected from image-only models.
