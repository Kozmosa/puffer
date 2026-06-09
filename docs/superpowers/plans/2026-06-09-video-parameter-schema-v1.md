# Video Parameter Schema v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace provider-native video parameter names with canonical Puffer video parameter semantics while keeping static discovery and provider-specific request mapping.

**Architecture:** Add a small `wire_type` enum to provider media parameters, propagate it through media capabilities, and update BytePlus serialization to use descriptor metadata for numeric duration. Rename executable video provider resource parameters to `duration_seconds`, `aspect_ratio`, and `resolution`, then make desktop video settings use those canonical names directly.

**Tech Stack:** Rust, serde/serde_yaml, existing provider registry and media resolver, Svelte 5, TypeScript, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-06-09-video-parameter-schema-v1-design.md`

---

## Scope Check

In scope:

- Video provider parameter semantics for BytePlus and Relaydance.
- A `string`/`number` wire type for media parameter descriptors.
- Static YAML descriptors only.
- Desktop video settings lookup by canonical parameter names.
- Focused governance, runtime, and desktop tests.
- Component update specs for touched crates/apps.

Out of scope:

- Runtime video provider discovery.
- Option-level display labels.
- BytePlus smart-duration sentinel values such as `-1`.
- Image parameter migration.
- Saved-settings migration from old provider-native names.
- Free-form numeric input, nested objects, booleans, URLs, media references, or dependency expressions.

## File Structure

- Modify: `crates/puffer-provider-registry/src/model.rs`
  - Add `MediaParameterWireType`.
  - Add `wire_type` to `MediaParameterSpec` with default `string`.
- Modify: `crates/puffer-provider-registry/src/lib.rs`
  - Re-export `MediaParameterWireType`.
- Modify: `crates/puffer-provider-registry/src/model_tests.rs`
  - Cover wire type parsing and defaulting.
- Modify: `crates/puffer-core/media_runtime_tests.rs`
  - Add the default string wire type to existing `MediaParameterSpec` literals.
- Modify: `crates/puffer-core/runtime/media/images_json_tests.rs`
  - Add the default string wire type to existing `MediaParameterSpec` literals.
- Modify: `crates/puffer-core/runtime/media/minimax_image.rs`
  - Add the default string wire type to existing test-only `MediaParameterSpec` literals.
- Modify: `crates/puffer-core/runtime/media/resolver.rs`
  - Add the default string wire type to existing `MediaParameterSpec` literals.
- Modify: `crates/puffer-core/runtime/media/capabilities.rs`
  - Carry `wire_type` in `MediaCapabilityParameter`.
- Modify: `crates/puffer-core/runtime/media/resolver.rs`
  - Copy `wire_type` from descriptors into capabilities.
  - Update struct literals in tests.
- Modify: `crates/puffer-core/runtime/media/byteplus_video.rs`
  - Encode BytePlus request parameter values from `wire_type`.
  - Reject invalid numeric wire values before HTTP submission.
- Modify: `crates/puffer-core/media_runtime_tests.rs`
  - Update test parameter names and expected saved parameter keys.
- Modify: `resources/providers/byteplus.yaml`
  - Rename video parameter names to canonical semantics.
  - Add `wire_type: number` only for BytePlus `duration_seconds`.
- Modify: `resources/providers/relaydance.yaml`
  - Rename video parameter names to canonical semantics.
  - Keep Relaydance wire values as strings.
- Modify: `crates/puffer-resources/tests/image_catalog_governance.rs`
  - Assert semantic names, request fields, wire types, values, and defaults.
- Modify: `apps/puffer-desktop/src/lib/types.ts`
  - Add `wireType` to media capability parameter type.
- Modify: `apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte`
  - Replace parameter-name heuristics with canonical video parameter names.
- Modify: `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
  - Update fake video capabilities and saved settings to canonical parameter names.
- Create: `specs/puffer-provider-registry/09.md`
- Create: `specs/puffer-resources/100.md`
- Create: `specs/puffer-core/265.md`
- Create: `specs/puffer-desktop/689.md`

## Task 1: Provider Registry Wire Type

**Files:**
- Modify: `crates/puffer-provider-registry/src/model.rs`
- Modify: `crates/puffer-provider-registry/src/lib.rs`
- Modify: `crates/puffer-provider-registry/src/model_tests.rs`
- Modify: `crates/puffer-core/media_runtime_tests.rs`
- Modify: `crates/puffer-core/runtime/media/images_json_tests.rs`
- Modify: `crates/puffer-core/runtime/media/minimax_image.rs`
- Modify: `crates/puffer-core/runtime/media/resolver.rs`

- [ ] **Step 1: Add failing wire-type tests**

Add these tests to `crates/puffer-provider-registry/src/model_tests.rs` near the existing media descriptor tests:

```rust
#[test]
fn media_parameter_wire_type_defaults_to_string() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  video:
    execution:
      adapter: replicate_video
      path: /v1/predictions
    models:
      - id: owner/model-version
        operations: [generate]
        parameters:
          - name: duration_seconds
            label: Duration
            values: ["5", "8"]
            default: "5"
            request_field: duration
"#,
    );

    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("parse provider");
    let parameter = &provider
        .media
        .as_ref()
        .and_then(|media| media.video.as_ref())
        .expect("video media")
        .models[0]
        .parameters[0];

    assert_eq!(parameter.wire_type, MediaParameterWireType::String);
    provider
        .validate_media_descriptors()
        .expect("default wire type validates");
}

#[test]
fn media_parameter_wire_type_parses_number() {
    let yaml = provider_with_media_yaml(
        r#"
media:
  video:
    execution:
      adapter: byteplus_video
      path: /contents/generations/tasks
    models:
      - id: dreamina-seedance-2-0-260128
        operations: [generate]
        parameters:
          - name: duration_seconds
            label: Duration
            values: ["4", "5"]
            default: "5"
            request_field: duration
            wire_type: number
"#,
    );

    let provider: ProviderDescriptor = serde_yaml::from_str(&yaml).expect("parse provider");
    let parameter = &provider
        .media
        .as_ref()
        .and_then(|media| media.video.as_ref())
        .expect("video media")
        .models[0]
        .parameters[0];

    assert_eq!(parameter.wire_type, MediaParameterWireType::Number);
    provider
        .validate_media_descriptors()
        .expect("number wire type validates");
}
```

- [ ] **Step 2: Run the focused failing tests**

Run:

```bash
cargo test -p puffer-provider-registry media_parameter_wire_type
```

Expected: fail because `MediaParameterWireType` and `MediaParameterSpec::wire_type` do not exist.

- [ ] **Step 3: Add the registry enum and field**

In `crates/puffer-provider-registry/src/model.rs`, add this enum near `MediaParameterSpec`:

```rust
/// Describes how a media parameter value is encoded into provider JSON.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaParameterWireType {
    /// Encode the selected value as a JSON string.
    #[default]
    String,
    /// Parse the selected value and encode it as a JSON number.
    Number,
}
```

Update `MediaParameterSpec`:

```rust
pub struct MediaParameterSpec {
    pub name: String,
    pub label: String,
    pub values: Vec<String>,
    pub default: String,
    #[serde(default)]
    pub request_field: Option<String>,
    #[serde(default)]
    pub wire_type: MediaParameterWireType,
}
```

- [ ] **Step 4: Re-export the enum**

In `crates/puffer-provider-registry/src/lib.rs`, add `MediaParameterWireType` to the existing public re-export list beside `MediaParameterSpec`.

- [ ] **Step 5: Update Rust struct literals**

Run:

```bash
rg -n "MediaParameterSpec \\{" crates
```

For every Rust `MediaParameterSpec` struct literal that does not set a numeric provider value, add:

```rust
wire_type: MediaParameterWireType::String,
```

Use `MediaParameterWireType::Number` only in the new test that explicitly verifies numeric parsing. Add `MediaParameterWireType` to the relevant `use puffer_provider_registry::{ ... }` lists in:

- `crates/puffer-provider-registry/src/model_tests.rs`
- `crates/puffer-core/media_runtime_tests.rs`
- `crates/puffer-core/runtime/media/images_json_tests.rs`
- `crates/puffer-core/runtime/media/minimax_image.rs`
- `crates/puffer-core/runtime/media/resolver.rs`

- [ ] **Step 6: Verify and commit**

Run:

```bash
cargo test -p puffer-provider-registry media_parameter_wire_type
cargo test -p puffer-provider-registry
cargo test -p puffer-core --no-run
```

Expected: all commands pass or compile successfully.

Commit:

```bash
git add crates/puffer-provider-registry/src/model.rs crates/puffer-provider-registry/src/lib.rs crates/puffer-provider-registry/src/model_tests.rs crates/puffer-core/media_runtime_tests.rs crates/puffer-core/runtime/media/images_json_tests.rs crates/puffer-core/runtime/media/minimax_image.rs crates/puffer-core/runtime/media/resolver.rs
git commit -m "feat: add media parameter wire type"
```

## Task 2: Capability Propagation And BytePlus Encoding

**Files:**
- Modify: `crates/puffer-core/runtime/media/capabilities.rs`
- Modify: `crates/puffer-core/runtime/media/resolver.rs`
- Modify: `crates/puffer-core/runtime/media/byteplus_video.rs`
- Modify: `crates/puffer-core/media_runtime_tests.rs`

- [ ] **Step 1: Add failing BytePlus wire-type tests**

In `crates/puffer-core/runtime/media/byteplus_video.rs`, update the test module imports to include `MediaParameterWireType`, then add:

```rust
#[test]
fn byteplus_request_body_uses_parameter_wire_type_for_numbers() {
    let request = byteplus_video_request_from_parameters(
        "dreamina-seedance-2-0-260128".to_string(),
        "animate a calm lake".to_string(),
        &[MediaCapabilityParameter {
            name: "duration_seconds".to_string(),
            label: "Duration".to_string(),
            values: vec!["4".to_string(), "5".to_string()],
            default: "5".to_string(),
            request_field: Some("duration".to_string()),
            wire_type: MediaParameterWireType::Number,
        }],
        &BTreeMap::from([("duration_seconds".to_string(), "5".to_string())]),
    )
    .expect("request");

    let body = request.request_body();

    assert_eq!(body["duration"], json!(5));
}

#[test]
fn byteplus_rejects_invalid_number_wire_value_before_http() {
    let error = byteplus_video_request_from_parameters(
        "dreamina-seedance-2-0-260128".to_string(),
        "animate a calm lake".to_string(),
        &[MediaCapabilityParameter {
            name: "duration_seconds".to_string(),
            label: "Duration".to_string(),
            values: vec!["fast".to_string()],
            default: "fast".to_string(),
            request_field: Some("duration".to_string()),
            wire_type: MediaParameterWireType::Number,
        }],
        &BTreeMap::new(),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("duration_seconds"));
    assert!(error.contains("number"));
}
```

- [ ] **Step 2: Run the focused failing tests**

Run:

```bash
cargo test -p puffer-core byteplus_request_body_uses_parameter_wire_type_for_numbers
cargo test -p puffer-core byteplus_rejects_invalid_number_wire_value_before_http
```

Expected: fail because `MediaCapabilityParameter` does not carry `wire_type`.

- [ ] **Step 3: Carry wire type in capabilities**

In `crates/puffer-core/runtime/media/capabilities.rs`, add:

```rust
use puffer_provider_registry::MediaParameterWireType;
```

Then update `MediaCapabilityParameter`:

```rust
pub(crate) struct MediaCapabilityParameter {
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) values: Vec<String>,
    pub(crate) default: String,
    pub(crate) request_field: Option<String>,
    pub(crate) wire_type: MediaParameterWireType,
}
```

In `crates/puffer-core/runtime/media/resolver.rs`, update `parameter_from_descriptor`:

```rust
fn parameter_from_descriptor(parameter: &MediaParameterSpec) -> MediaCapabilityParameter {
    MediaCapabilityParameter {
        name: parameter.name.clone(),
        label: parameter.label.clone(),
        values: parameter.values.clone(),
        default: parameter.default.clone(),
        request_field: parameter.request_field.clone(),
        wire_type: parameter.wire_type,
    }
}
```

- [ ] **Step 4: Make BytePlus request params typed**

In `crates/puffer-core/runtime/media/byteplus_video.rs`, replace `params: Vec<(String, String)>` with a typed value pair:

```rust
pub(crate) struct BytePlusVideoRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) params: Vec<(String, Value)>,
}
```

Update the request-body loop:

```rust
for (field, value) in &self.params {
    body.insert(field.trim().to_string(), value.clone());
}
```

Replace `byteplus_request_value` with:

```rust
fn byteplus_request_value(parameter: &MediaCapabilityParameter, value: &str) -> Result<Value> {
    let value = value.trim();
    match parameter.wire_type {
        MediaParameterWireType::String => Ok(json!(value)),
        MediaParameterWireType::Number => value
            .parse::<u64>()
            .map(Value::from)
            .with_context(|| {
                format!(
                    "video generation parameter {}={} must be a number",
                    parameter.name, value
                )
            }),
    }
}
```

Update the parameter loop in `byteplus_video_request_from_parameters`:

```rust
for parameter in capability_parameters {
    let Some(field) = parameter.request_field.clone() else {
        continue;
    };
    let value = selected
        .get(&parameter.name)
        .cloned()
        .unwrap_or_else(|| parameter.default.clone());
    params.push((field, byteplus_request_value(parameter, &value)?));
}
```

- [ ] **Step 5: Update affected test literals**

Run:

```bash
rg -n "MediaCapabilityParameter \\{" crates/puffer-core
```

For each test literal, add `wire_type: MediaParameterWireType::String` unless the test intentionally checks BytePlus numeric duration.

- [ ] **Step 6: Verify and commit**

Run:

```bash
cargo test -p puffer-core byteplus_request_body_uses_parameter_wire_type_for_numbers
cargo test -p puffer-core byteplus_rejects_invalid_number_wire_value_before_http
cargo test -p puffer-core byteplus_request_body_encodes_duration_as_number
cargo test -p puffer-core media_runtime
```

Expected: all commands pass.

Commit:

```bash
git add crates/puffer-core/runtime/media/capabilities.rs crates/puffer-core/runtime/media/resolver.rs crates/puffer-core/runtime/media/byteplus_video.rs crates/puffer-core/media_runtime_tests.rs
git commit -m "feat: propagate video parameter wire types"
```

## Task 3: Canonical Provider Video Catalog

**Files:**
- Modify: `resources/providers/byteplus.yaml`
- Modify: `resources/providers/relaydance.yaml`
- Modify: `crates/puffer-resources/tests/image_catalog_governance.rs`

- [ ] **Step 1: Update governance tests first**

In `crates/puffer-resources/tests/image_catalog_governance.rs`, import `MediaParameterWireType` and add a wire-type-aware helper without changing the existing image helper:

```rust
fn assert_select_parameter_with_wire_type(
    model: &MediaModelDescriptor,
    name: &str,
    label: &str,
    values: &[&str],
    default: &str,
    request_field: &str,
    wire_type: MediaParameterWireType,
) {
    let parameter = model
        .parameters
        .iter()
        .find(|parameter| parameter.name == name)
        .unwrap_or_else(|| panic!("{} should declare parameter {name}", model.id));
    assert_eq!(parameter.label, label);
    assert_eq!(
        parameter.values,
        values.iter().map(|value| value.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(parameter.default, default);
    assert_eq!(parameter.request_field.as_deref(), Some(request_field));
    assert_eq!(parameter.wire_type, wire_type);
}
```

Keep the existing `assert_select_parameter` helper for image and string-default cases by making it delegate:

```rust
fn assert_select_parameter(
    model: &MediaModelDescriptor,
    name: &str,
    label: &str,
    values: &[&str],
    default: &str,
    request_field: &str,
) {
    assert_select_parameter_with_wire_type(
        model,
        name,
        label,
        values,
        default,
        request_field,
        MediaParameterWireType::String,
    );
}
```

Update BytePlus video expectations to:

```rust
assert_select_parameter_with_wire_type(
    model,
    "duration_seconds",
    "Duration",
    SEEDANCE_VIDEO_DURATIONS,
    "5",
    "duration",
    MediaParameterWireType::Number,
);
assert_select_parameter_with_wire_type(
    model,
    "aspect_ratio",
    "Aspect ratio",
    SEEDANCE_VIDEO_RATIOS,
    "adaptive",
    "ratio",
    MediaParameterWireType::String,
);
```

Update Relaydance video expectations to use `duration_seconds`, `aspect_ratio`, request field `seconds` or `metadata.ratio`, and `MediaParameterWireType::String`.

- [ ] **Step 2: Run the failing governance test**

Run:

```bash
cargo test -p puffer-resources --test image_catalog_governance
```

Expected: fail because provider YAML still uses `duration` and `ratio`.

- [ ] **Step 3: Update BytePlus YAML**

In `resources/providers/byteplus.yaml`, update video parameters:

```yaml
- name: duration_seconds
  label: Duration
  values: ["4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15"]
  default: "5"
  request_field: duration
  wire_type: number
- name: aspect_ratio
  label: Aspect ratio
  values: ["16:9", "4:3", "1:1", "3:4", "9:16", "21:9", "adaptive"]
  default: "adaptive"
  request_field: ratio
```

Apply the same canonical names to both BytePlus video models. Keep resolution unchanged.

- [ ] **Step 4: Update Relaydance YAML**

In `resources/providers/relaydance.yaml`, update every video model that currently uses `duration` or `ratio`:

```yaml
- name: duration_seconds
  label: Duration
  values: ["4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15"]
  default: "5"
  request_field: seconds
- name: aspect_ratio
  label: Aspect ratio
  values: ["16:9", "4:3", "1:1", "3:4", "9:16", "21:9", "adaptive"]
  default: "16:9"
  request_field: metadata.ratio
```

For Seedance 1.5 models, keep the existing shorter duration values and only rename `name`.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo test -p puffer-resources --test image_catalog_governance
cargo test -p puffer-core connected_static_video_descriptors_are_available_when_authenticated
```

Expected: governance passes, and the core capability test passes after any test expectations are updated to canonical names.

Commit:

```bash
git add resources/providers/byteplus.yaml resources/providers/relaydance.yaml crates/puffer-resources/tests/image_catalog_governance.rs crates/puffer-core/runtime/media/resolver.rs crates/puffer-core/media_runtime_tests.rs
git commit -m "feat: canonicalize video provider parameters"
```

## Task 4: Desktop Semantic Parameter Lookup

**Files:**
- Modify: `apps/puffer-desktop/src/lib/types.ts`
- Modify: `apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte`
- Modify: `apps/puffer-desktop/tests/chat-session-ui.spec.ts`

- [ ] **Step 1: Update fake capability tests first**

In `apps/puffer-desktop/tests/chat-session-ui.spec.ts`, update fake video parameters and expected saved settings:

```ts
{
  name: "aspect_ratio",
  label: "Aspect ratio",
  values: ["16:9", "9:16"],
  default: "16:9",
  requestField: "metadata.ratio",
  wireType: "string"
},
{
  name: "duration_seconds",
  label: "Duration",
  values: ["5", "8", "12"],
  default: "8",
  requestField: "seconds",
  wireType: "string"
}
```

Expected saved video settings should use:

```ts
parameters: {
  aspect_ratio: "9:16",
  duration_seconds: "12"
}
```

- [ ] **Step 2: Run the focused failing desktop test**

Run from `apps/puffer-desktop`:

```bash
npx playwright test tests/chat-session-ui.spec.ts -g "video generation settings saves configurable video defaults"
```

Expected: fail where the modal still reads `duration` rather than `duration_seconds`.

- [ ] **Step 3: Update TypeScript capability parameter type**

In `apps/puffer-desktop/src/lib/types.ts`, update `MediaCapabilityParameterInfo`:

```ts
export type MediaCapabilityParameterInfo = {
  name: string;
  label: string;
  values: string[];
  default: string;
  requestField: string | null;
  wireType?: "string" | "number";
};
```

- [ ] **Step 4: Replace video parameter heuristics**

In `apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte`, replace the alias arrays with canonical names:

```ts
const VIDEO_ASPECT_RATIO_PARAMETER_NAME = "aspect_ratio";
const VIDEO_DURATION_PARAMETER_NAME = "duration_seconds";
const VIDEO_PRIMARY_PARAMETER_NAMES = new Set([
  VIDEO_ASPECT_RATIO_PARAMETER_NAME,
  VIDEO_DURATION_PARAMETER_NAME
]);
```

Replace `findCapabilityParameter` with a direct lookup:

```ts
function findCapabilityParameter(
  capability: MediaCapabilityInfo | null,
  name: string
): MediaCapabilityParameterInfo | null {
  if (!capability || capability.kind !== "video") return null;
  return capability.parameters.find((parameter) => parameter.name === name) ?? null;
}
```

Update derived values:

```ts
let aspectRatioParameter = $derived(
  findCapabilityParameter(selectedCapability, VIDEO_ASPECT_RATIO_PARAMETER_NAME)
);
let durationParameter = $derived(
  findCapabilityParameter(selectedCapability, VIDEO_DURATION_PARAMETER_NAME)
);
```

Update `isPrimaryVideoParameter`:

```ts
function isPrimaryVideoParameter(parameter: MediaCapabilityParameterInfo): boolean {
  return VIDEO_PRIMARY_PARAMETER_NAMES.has(parameter.name);
}
```

Remove `parameterMatchesNames`, `parameterNameCandidates`, `normalizedParameterNameSet`, `normalizedNameCandidates`, and `normalizeParameterName` after all call sites are gone.

- [ ] **Step 5: Update saved-parameter readers**

Change video saved-parameter lookup helpers to read canonical keys:

```ts
function videoAspectRatioFromParameters(value: Record<string, string>): string {
  return value[VIDEO_ASPECT_RATIO_PARAMETER_NAME] ?? "16:9";
}

function videoDurationFromParameters(value: Record<string, string>): number {
  const duration = value[VIDEO_DURATION_PARAMETER_NAME] ?? "5";
  return parseDurationSeconds(duration) ?? 5;
}
```

In `videoParametersFromCurrent`, keep the selected capability parameter names:

```ts
const aspectName = aspectRatioParameter?.name ?? VIDEO_ASPECT_RATIO_PARAMETER_NAME;
const durationName = durationParameter?.name ?? VIDEO_DURATION_PARAMETER_NAME;
```

- [ ] **Step 6: Verify and commit**

Run from `apps/puffer-desktop`:

```bash
npx vitest run src/lib/screens/agent/mediaCapabilityState.test.ts
npx playwright test tests/chat-session-ui.spec.ts -g "video generation settings"
```

Expected: both commands pass.

Commit:

```bash
git add apps/puffer-desktop/src/lib/types.ts apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte apps/puffer-desktop/tests/chat-session-ui.spec.ts
git commit -m "feat: use semantic video parameters in desktop settings"
```

## Task 5: Component Specs And Full Verification

**Files:**
- Create: `specs/puffer-provider-registry/09.md`
- Create: `specs/puffer-resources/100.md`
- Create: `specs/puffer-core/265.md`
- Create: `specs/puffer-desktop/689.md`

- [ ] **Step 1: Add provider-registry update spec**

Create `specs/puffer-provider-registry/09.md`:

```markdown
# Media Parameter Wire Type

Media parameter descriptors now carry a small `wire_type` enum with `string`
as the default and `number` for provider fields that must be encoded as JSON
numbers.

The field is declarative metadata on existing static media descriptors. It does
not add runtime discovery, free-form inputs, or nested parameter support.
```

- [ ] **Step 2: Add resources update spec**

Create `specs/puffer-resources/100.md`:

```markdown
# Canonical Video Parameter Names

BytePlus and Relaydance video descriptors now use canonical Puffer parameter
names: `duration_seconds`, `aspect_ratio`, and `resolution`.

Provider request fields remain explicit through `request_field`. BytePlus
duration uses `wire_type: number`; Relaydance keeps string wire values and
existing request nesting.
```

- [ ] **Step 3: Add core update spec**

Create `specs/puffer-core/265.md`:

```markdown
# Video Parameter Wire Encoding

Media capabilities now carry parameter wire type metadata from provider
descriptors. BytePlus video request construction uses that metadata to encode
duration as a JSON number and fails locally if a numeric value cannot parse.

Relaydance video serialization is unchanged except for canonical selected
parameter names.
```

- [ ] **Step 4: Add desktop update spec**

Create `specs/puffer-desktop/689.md`:

```markdown
# Semantic Video Settings Parameters

The video settings modal now reads canonical video parameter names directly
instead of matching provider-native aliases such as `ratio`, `duration`, and
`seconds`.

Saved video settings use `aspect_ratio` and `duration_seconds`. Existing stale
provider-native saved keys are treated as unsupported and normalized through the
selected capability defaults.
```

- [ ] **Step 5: Run full focused verification**

Run:

```bash
cargo test -p puffer-provider-registry media_parameter_wire_type
cargo test -p puffer-resources --test image_catalog_governance
cargo test -p puffer-core byteplus
cargo test -p puffer-core media_runtime
```

Run from `apps/puffer-desktop`:

```bash
npx vitest run src/lib/screens/agent/mediaCapabilityState.test.ts
npx playwright test tests/chat-session-ui.spec.ts -g "video generation settings"
```

Expected: all focused tests pass.

- [ ] **Step 6: Run workspace verification**

Run from the repo root:

```bash
cargo test --workspace
```

Expected: workspace Rust tests pass. Desktop Playwright coverage remains covered by the focused command in Step 5.

- [ ] **Step 7: Commit final specs and verification follow-up**

Run:

```bash
git add specs/puffer-provider-registry/09.md specs/puffer-resources/100.md specs/puffer-core/265.md specs/puffer-desktop/689.md
git commit -m "docs: record video parameter schema updates"
```

Final check:

```bash
git status --short
```

Expected: no output.
