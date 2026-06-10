# BytePlus Video References Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add BytePlus video generation support for ordered public HTTPS and `asset://` image references while keeping Relaydance prompt-only.

**Architecture:** Carry `imageReferences: string[]` from the internal tool/CLI boundary through `VideoGenerationInput` and `ExactMediaGenerationRequest` into the BytePlus adapter. Validate only URL shape and scheme, never download or stage files in this change. Relaydance rejects non-empty image references before any HTTP request.

**Tech Stack:** Rust, `serde`, `serde_json`, `anyhow`, `clap`, existing Puffer media runtime and fake HTTP tests.

---

Spec: `docs/superpowers/specs/2026-06-10-byteplus-video-references-design.md`

## Recheck Outcome

The design was narrowed before planning:

- Use `imageReferences: string[]`, not a generic `references` object.
- Accept only `https://...` and `asset://...`.
- Reject local paths in this implementation; local images need a later staging design.
- Do not add first-frame, last-frame, video, audio, role, or source enums.
- Keep Relaydance prompt-only until provider-specific support is confirmed.

## File Structure

- `resources/internal_tools/video_generation.yaml` - expose `imageReferences` in the VideoGeneration internal tool schema and remove "text-to-video only" wording.
- `resources/skills/video-generation/SKILL.md` - teach agents to pass `--image-reference` for public HTTPS or `asset://` image references and to reject local paths.
- `crates/puffer-resources/tests/video_tool_schema.rs` - assert the internal tool schema accepts scalar parameters and `imageReferences`.
- `crates/puffer-resources/tests/media_generation_skills.rs` - assert the video skill advertises image-reference support without local-path support.
- `crates/puffer-cli/src/media_internal_tools.rs` - add repeated `--image-reference` args and serialize them to workflow JSON as `imageReferences`.
- `crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs` - parse, validate, and forward `imageReferences`.
- `crates/puffer-core/media_runtime.rs` - add `image_references` to `ExactMediaGenerationRequest`.
- `crates/puffer-core/media_runtime_video.rs` - pass image references to BytePlus and reject them for Relaydance/Replicate.
- `crates/puffer-core/runtime/media/byteplus_video.rs` - include image references as `content[]` image URL items.
- `specs/puffer-resources/108.md` - component update spec for resource and skill schema changes.
- `specs/puffer-cli/219.md` - component update spec for CLI argument and payload changes.
- `specs/puffer-core/275.md` - component update spec for runtime and adapter changes.

## Task 1: Update Resource Schema And Video Skill Guidance

**Files:**
- Modify: `resources/internal_tools/video_generation.yaml`
- Modify: `resources/skills/video-generation/SKILL.md`
- Modify: `crates/puffer-resources/tests/video_tool_schema.rs`
- Modify: `crates/puffer-resources/tests/media_generation_skills.rs`

- [ ] **Step 1: Write failing schema assertions**

Replace `crates/puffer-resources/tests/video_tool_schema.rs` with:

```rust
#[test]
fn video_generation_tool_schema_accepts_scalar_parameter_values() {
    let tool: serde_json::Value = serde_yaml::from_str(include_str!(
        "../../../resources/internal_tools/video_generation.yaml"
    ))
    .expect("VideoGeneration internal tool YAML");
    let parameter_types = tool["input_schema"]["properties"]["parameters"]["additionalProperties"]
        ["oneOf"]
        .as_array()
        .expect("parameter scalar types");
    let types = parameter_types
        .iter()
        .filter_map(|value| value.get("type").and_then(serde_json::Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        types,
        std::collections::BTreeSet::from(["boolean", "number", "string"])
    );
}

#[test]
fn video_generation_tool_schema_accepts_image_references() {
    let tool: serde_json::Value = serde_yaml::from_str(include_str!(
        "../../../resources/internal_tools/video_generation.yaml"
    ))
    .expect("VideoGeneration internal tool YAML");

    let image_references = &tool["input_schema"]["properties"]["imageReferences"];
    assert_eq!(image_references["type"], "array");
    assert_eq!(image_references["items"]["type"], "string");
    assert!(
        image_references["description"]
            .as_str()
            .is_some_and(|value| value.contains("https://") && value.contains("asset://"))
    );
}
```

- [ ] **Step 2: Write failing skill assertions**

In `crates/puffer-resources/tests/media_generation_skills.rs`, replace the body assertions in `video_generation_skill_guides_foreground_bash_internal_tool_use` with:

```rust
    assert!(body.contains("foreground Bash"));
    assert!(body.contains("explicit long Bash timeout"));
    assert!(body.contains("puffer internal-tool video-generation --prompt"));
    assert!(body.contains("--parameters-json"));
    assert!(body.contains("--image-reference"));
    assert!(body.contains("https://"));
    assert!(body.contains("asset://"));
    assert!(body.contains("local paths"));
    assert!(body.contains("scalar"));
    assert!(body.contains("allowed-tools is guidance"));
    assert!(body.contains("persisted video artifact"));
    assert!(!body.contains("text-to-video only"));
```

- [ ] **Step 3: Run focused tests and confirm failure**

Run:

```bash
cargo test -p puffer-resources video_generation_tool_schema_accepts_image_references video_generation_skill_guides_foreground_bash_internal_tool_use
```

Expected: FAIL because `imageReferences` and `--image-reference` are not documented yet.

- [ ] **Step 4: Update the internal tool YAML**

In `resources/internal_tools/video_generation.yaml`, change the description to remove the text-only limitation and add `imageReferences` under `input_schema.properties`:

```yaml
description: |-
  Generate one video clip through Puffer's configured video media settings and
  write the resulting video artifact into the workspace media folder.

  Use one VideoGeneration call for one logical video-generation request. The
  tool reads a workspace-relative prompt file when the prompt value names one;
  otherwise it treats prompt as literal text. BytePlus supports ordered image
  references through public https:// URLs or approved asset:// URLs. Local image
  paths are not accepted by this tool; upload or stage them before passing an
  image reference.
```

Add this property next to `parameters`:

```yaml
    imageReferences:
      type: array
      description: Ordered public https:// image URLs or approved asset:// image references. Prompts should refer to them as image 1, image 2, and so on. Local paths and base64/data URLs are not supported.
      items:
        type: string
```

- [ ] **Step 5: Update the video skill**

Replace `resources/skills/video-generation/SKILL.md` body bullets with:

```markdown
Use foreground Bash only. allowed-tools is guidance, not the enforcement boundary; media generation is enforced by the internal tool permission path.

- Run `puffer internal-tool video-generation --prompt ...` for one logical video-generation request.
- Set an explicit long Bash timeout within the current Bash cap before running the command.
- Treat `--prompt` as literal text unless it names a workspace-relative file; prompt file paths should be passed through `--prompt`.
- Pass `--parameters-json` only for requested scalar overrides: strings, numbers, or booleans.
- Pass one `--image-reference` for each public `https://` image URL or approved `asset://` BytePlus asset the user wants to use. Keep the order stable and refer to them in the prompt as image 1, image 2, and so on.
- Do not pass local paths, `file://` URLs, base64 strings, or data URLs as image references. Ask the user to stage or upload the local image first.
- Use `--purpose` only when the request supplies a purpose that should be preserved in result metadata.
- Relaydance is prompt-only in Puffer. If the selected provider is Relaydance and the user asks for image references, report that the configured provider does not support image references.
- If video generation fails or the media runtime is unavailable, report that plainly.
- Do not imply a video was created unless the tool returns a persisted video artifact.
```

- [ ] **Step 6: Run focused tests and confirm pass**

Run:

```bash
cargo test -p puffer-resources video_generation_tool_schema media_generation_skills
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add resources/internal_tools/video_generation.yaml resources/skills/video-generation/SKILL.md crates/puffer-resources/tests/video_tool_schema.rs crates/puffer-resources/tests/media_generation_skills.rs
git commit -m "feat(resources): expose video image references"
```

## Task 2: Add CLI Image Reference Arguments

**Files:**
- Modify: `crates/puffer-cli/src/media_internal_tools.rs`

- [ ] **Step 1: Write failing CLI payload test**

In `crates/puffer-cli/src/media_internal_tools.rs`, update `video_generation_cli_builds_workflow_input` to include repeated image references:

```rust
    #[test]
    fn video_generation_cli_builds_workflow_input() {
        let cli = Cli::parse_from([
            "puffer",
            "internal-tool",
            "video-generation",
            "--prompt",
            "clip prompt",
            "--purpose",
            "storyboard",
            "--parameters-json",
            "{\"duration_seconds\":5,\"camera_fixed\":false}",
            "--image-reference",
            "https://example.com/person.png",
            "--image-reference",
            "asset://approved-person",
        ]);
        let Some(Command::InternalTool {
            command: InternalToolCommand::VideoGeneration(args),
        }) = cli.subcommand
        else {
            panic!("expected video generation internal tool command");
        };

        let input = video_generation_input(&args).expect("video input");

        assert_eq!(
            input,
            json!({
                "prompt": "clip prompt",
                "purpose": "storyboard",
                "imageReferences": [
                    "https://example.com/person.png",
                    "asset://approved-person"
                ],
                "parameters": {
                    "duration_seconds": 5,
                    "camera_fixed": false
                }
            })
        );
    }
```

- [ ] **Step 2: Run focused CLI test and confirm failure**

Run:

```bash
cargo test -p puffer-cli video_generation_cli_builds_workflow_input
```

Expected: FAIL because `--image-reference` is not a known argument.

- [ ] **Step 3: Add repeated CLI argument**

Add this field to `VideoGenerationArgs`:

```rust
    /// Ordered public https:// image URLs or approved asset:// references.
    #[arg(long = "image-reference")]
    pub(crate) image_references: Vec<String>,
```

Update `video_generation_input` to include non-empty references:

```rust
    if !args.image_references.is_empty() {
        object.insert(
            "imageReferences".to_string(),
            Value::Array(
                args.image_references
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
```

- [ ] **Step 4: Update direct struct construction in tests**

In `media_cli_rejects_invalid_json_payloads`, add `image_references: Vec::new(),` to the `VideoGenerationArgs` literal:

```rust
        let video = VideoGenerationArgs {
            prompt: "clip".to_string(),
            purpose: None,
            parameters_json: Some("{".to_string()),
            image_references: Vec::new(),
        };
```

- [ ] **Step 5: Run focused CLI tests and confirm pass**

Run:

```bash
cargo test -p puffer-cli media_internal_tools
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/puffer-cli/src/media_internal_tools.rs
git commit -m "feat(cli): pass video image references"
```

## Task 3: Add Workflow And Exact Runtime Image References

**Files:**
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs`
- Modify: `crates/puffer-core/media_runtime.rs`
- Modify: `crates/puffer-core/media_runtime_tests.rs`

- [ ] **Step 1: Write failing workflow parsing and validation tests**

In `crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs`, add these tests near the existing input parsing tests:

```rust
    #[test]
    fn parses_video_generation_image_references() {
        let input = serde_json::from_value::<VideoGenerationInput>(json!({
            "prompt": "animate image 1",
            "imageReferences": [
                "https://example.com/person.png",
                "asset://approved-person"
            ]
        }))
        .unwrap();

        assert_eq!(
            input.image_references,
            vec![
                "https://example.com/person.png".to_string(),
                "asset://approved-person".to_string()
            ]
        );
    }

    #[test]
    fn validates_video_generation_image_references() {
        let values = vec![
            " https://example.com/person.png ".to_string(),
            "asset://approved-person".to_string(),
        ];

        let validated = validate_video_image_references(&values).expect("valid refs");

        assert_eq!(
            validated,
            vec![
                "https://example.com/person.png".to_string(),
                "asset://approved-person".to_string()
            ]
        );
    }

    #[test]
    fn rejects_invalid_video_generation_image_references() {
        for value in [
            "",
            "http://example.com/person.png",
            "file:///tmp/person.png",
            "person.png",
            "/tmp/person.png",
            "data:image/png;base64,AAAA",
            "asset://",
        ] {
            let error = validate_video_image_references(&[value.to_string()])
                .unwrap_err()
                .to_string();
            assert!(error.contains("imageReferences[0]"), "{value}: {error}");
            assert!(error.contains("https:// or asset://"), "{value}: {error}");
        }
    }
```

- [ ] **Step 2: Update existing `VideoGenerationInput` literals in tests**

Add `image_references: Vec::new(),` to every direct `VideoGenerationInput` literal in this file. For example:

```rust
            VideoGenerationInput {
                prompt: "  ".to_string(),
                image_references: Vec::new(),
                parameters: BTreeMap::new(),
                purpose: None,
            },
```

- [ ] **Step 3: Run focused tests and confirm failure**

Run:

```bash
cargo test -p puffer-core -- parses_video_generation_image_references validates_video_generation_image_references rejects_invalid_video_generation_image_references
```

Expected: FAIL because `image_references` and `validate_video_image_references` do not exist.

- [ ] **Step 4: Add input and request fields**

In `VideoGenerationInput`, add:

```rust
    #[serde(default)]
    image_references: Vec<String>,
```

In `VideoRequest`, add:

```rust
    image_references: Vec<String>,
```

In `build_video_request`, validate and store references:

```rust
    let image_references = validate_video_image_references(&input.image_references)?;
```

and add `image_references,` to the `VideoRequest` initializer.

- [ ] **Step 5: Add validation helper**

Add this helper near `deserialize_scalar_parameters`:

```rust
pub(crate) fn validate_video_image_references(values: &[String]) -> Result<Vec<String>> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| validate_video_image_reference(index, value))
        .collect()
}

fn validate_video_image_reference(index: usize, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("VideoGeneration imageReferences[{index}] must be an https:// or asset:// URL");
    }
    if let Some(asset_id) = trimmed.strip_prefix("asset://") {
        if !asset_id.trim().is_empty() {
            return Ok(trimmed.to_string());
        }
        bail!("VideoGeneration imageReferences[{index}] must be an https:// or asset:// URL");
    }
    let parsed = url::Url::parse(trimmed)
        .map_err(|_| anyhow::anyhow!("VideoGeneration imageReferences[{index}] must be an https:// or asset:// URL"))?;
    if parsed.scheme() == "https" {
        return Ok(trimmed.to_string());
    }
    bail!("VideoGeneration imageReferences[{index}] must be an https:// or asset:// URL")
}
```

Ensure the file imports `anyhow::anyhow` or uses `anyhow::anyhow!` fully qualified as shown.

- [ ] **Step 6: Add exact request field**

In `crates/puffer-core/media_runtime.rs`, add this field to `ExactMediaGenerationRequest` after `prompt`:

```rust
    pub image_references: Vec<String>,
```

In `exact_media_request`, forward it:

```rust
        image_references: request.image_references.clone(),
```

Update the two `ExactMediaGenerationRequest` literals in
`crates/puffer-core/media_runtime_tests.rs` with:

```rust
        image_references: Vec::new(),
```

- [ ] **Step 7: Assert exact request carries image references**

In `build_request_merges_saved_parameters_and_tool_overrides`, include an image reference in the input and assert it is retained:

```rust
                image_references: vec!["https://example.com/person.png".to_string()],
```

Add:

```rust
        assert_eq!(
            request.image_references,
            vec!["https://example.com/person.png".to_string()]
        );
```

- [ ] **Step 8: Run puffer-core focused workflow tests**

Run:

```bash
cargo test -p puffer-core video_generation
```

Expected: PASS for workflow tests after all `ExactMediaGenerationRequest` literals are updated.

- [ ] **Step 9: Commit**

```bash
git add crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs crates/puffer-core/media_runtime.rs crates/puffer-core/media_runtime_tests.rs
git commit -m "feat(core): carry video image references"
```

## Task 4: Serialize BytePlus Image References And Reject Others

**Files:**
- Modify: `crates/puffer-core/runtime/media/byteplus_video.rs`
- Modify: `crates/puffer-core/media_runtime_video.rs`

- [ ] **Step 1: Write failing BytePlus request body tests**

In `crates/puffer-core/runtime/media/byteplus_video.rs`, add these tests near `byteplus_request_body_contains_model_and_prompt`:

```rust
    #[test]
    fn byteplus_request_body_includes_public_image_references() {
        let request = BytePlusVideoRequest {
            model: "dreamina-seedance-2-0-fast-260128".to_string(),
            prompt: "animate image 1".to_string(),
            image_references: vec!["https://example.com/person.png".to_string()],
            params: vec![],
        };
        let body = request.request_body();

        assert_eq!(body["content"][0]["type"], json!("text"));
        assert_eq!(body["content"][1]["type"], json!("image_url"));
        assert_eq!(
            body["content"][1]["image_url"]["url"],
            json!("https://example.com/person.png")
        );
        assert!(body["content"][1].get("role").is_none());
    }

    #[test]
    fn byteplus_request_body_includes_asset_image_references() {
        let request = BytePlusVideoRequest {
            model: "dreamina-seedance-2-0-fast-260128".to_string(),
            prompt: "animate image 1".to_string(),
            image_references: vec!["asset://approved-person".to_string()],
            params: vec![],
        };
        let body = request.request_body();

        assert_eq!(
            body["content"][1]["image_url"]["url"],
            json!("asset://approved-person")
        );
    }
```

Update all existing `BytePlusVideoRequest` literals in tests with:

```rust
            image_references: Vec::new(),
```

- [ ] **Step 2: Write failing Relaydance rejection test**

In `crates/puffer-core/media_runtime_video.rs`, add:

```rust
    #[test]
    fn relaydance_rejects_video_image_references() {
        let request = ExactMediaGenerationRequest {
            kind: "video".to_string(),
            provider_id: "relaydance".to_string(),
            model_id: "doubao-seedance-2-0-720p".to_string(),
            operation: "generate".to_string(),
            adapter: RELAYDANCE_VIDEO_ADAPTER.to_string(),
            prompt: "animate image 1".to_string(),
            image_references: vec!["https://example.com/person.png".to_string()],
            parameters: BTreeMap::new(),
            count: 1,
        };

        let error = generate_relaydance_video(
            &ProviderRegistry::new(),
            &AuthStore::default(),
            tempfile::tempdir().unwrap().path(),
            &request,
            &MediaCapability {
                provider_id: "relaydance".to_string(),
                model_id: "doubao-seedance-2-0-720p".to_string(),
                operation: MediaOperation::Generate,
                adapter: RELAYDANCE_VIDEO_ADAPTER.to_string(),
                parameters: Vec::new(),
            },
            BTreeMap::new(),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("relaydance"));
        assert!(error.contains("image references"));
    }
```

If private imports are missing in the test module, add:

```rust
    use puffer_provider_registry::{AuthStore, MediaOperation, ProviderRegistry};
```

- [ ] **Step 3: Run focused tests and confirm failure**

Run:

```bash
cargo test -p puffer-core -- byteplus_request_body_includes_public_image_references byteplus_request_body_includes_asset_image_references relaydance_rejects_video_image_references
```

Expected: FAIL because request structs and runtime routing do not yet handle image references.

- [ ] **Step 4: Update BytePlus request struct and request body**

Add `image_references` to `BytePlusVideoRequest`:

```rust
pub(crate) struct BytePlusVideoRequest {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) image_references: Vec<String>,
    pub(crate) params: Vec<(String, Value)>,
}
```

Replace the hardcoded content insertion in `request_body` with:

```rust
        let mut content = vec![json!({
            "type": "text",
            "text": self.prompt.trim()
        })];
        for reference in &self.image_references {
            content.push(json!({
                "type": "image_url",
                "image_url": {
                    "url": reference.trim()
                }
            }));
        }
        body.insert("content".to_string(), Value::Array(content));
```

In `validate`, reject empty image references defensively:

```rust
        for (index, reference) in self.image_references.iter().enumerate() {
            if reference.trim().is_empty() {
                bail!("video image reference {index} is empty");
            }
        }
```

Change `byteplus_video_request_from_parameters` signature to:

```rust
pub(crate) fn byteplus_video_request_from_parameters(
    model_id: String,
    prompt: String,
    image_references: Vec<String>,
    capability_parameters: &[MediaCapabilityParameter],
    selected: &BTreeMap<String, String>,
) -> Result<BytePlusVideoRequest>
```

and set `image_references` in the request initializer.

- [ ] **Step 5: Update media runtime routing**

In `generate_replicate_video`, add an early rejection before adapter construction:

```rust
    if !request.image_references.is_empty() {
        bail!("provider {} does not support video image references", request.provider_id);
    }
```

In `generate_relaydance_video`, add the same early rejection before `build_relaydance_adapter`.

In `generate_byteplus_video`, pass references into the BytePlus request builder:

```rust
    let video_request = byteplus_video_request_from_parameters(
        request.model_id.clone(),
        request.prompt.clone(),
        request.image_references.clone(),
        &capability.parameters,
        &parameters,
    )?;
```

- [ ] **Step 6: Update BytePlus test literals and call sites**

Every direct `BytePlusVideoRequest` literal needs `image_references: Vec::new(),` unless the test is specifically exercising image references.

Every `byteplus_video_request_from_parameters` test call needs a third argument:

```rust
            Vec::new(),
```

- [ ] **Step 7: Run focused media tests**

Run:

```bash
cargo test -p puffer-core -- byteplus_request_body relaydance_rejects_video_image_references
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/puffer-core/runtime/media/byteplus_video.rs crates/puffer-core/media_runtime_video.rs
git commit -m "feat(media): send byteplus video image references"
```

## Task 5: Component Specs And Verification

**Files:**
- Create: `specs/puffer-resources/108.md`
- Create: `specs/puffer-cli/219.md`
- Create: `specs/puffer-core/275.md`

- [ ] **Step 1: Write puffer-resources update spec**

Create `specs/puffer-resources/108.md`:

```markdown
# Video Generation Image References

The `VideoGeneration` internal tool schema now accepts `imageReferences`, an
ordered array of provider-usable image reference strings. Values are public
`https://` image URLs or approved BytePlus `asset://` URLs.

The `video-generation` skill now tells agents to pass image references with
repeated `--image-reference` arguments, preserve order, and refer to assets in
prompts as image 1, image 2, and so on. Local paths, `file://`, base64, and
data URLs remain unsupported by the generation tool because local image support
requires a separate staging layer.

Relaydance remains prompt-only in Puffer guidance.
```

- [ ] **Step 2: Write puffer-cli update spec**

Create `specs/puffer-cli/219.md`:

```markdown
# Video Internal CLI Image References

`puffer internal-tool video-generation` now accepts repeated
`--image-reference` arguments. The CLI preserves argument order and serializes
non-empty values into workflow JSON as `imageReferences`.

The CLI boundary does not validate URL schemes; the core workflow owns
provider-neutral validation and error messages. Existing `--prompt`,
`--purpose`, and `--parameters-json` behavior is unchanged.
```

- [ ] **Step 3: Write puffer-core update spec**

Create `specs/puffer-core/275.md`:

```markdown
# BytePlus Video Image References

Video generation runtime requests now carry ordered `image_references` from the
workflow layer through `ExactMediaGenerationRequest`. The workflow validates
that each image reference is either an `https://` URL or a non-empty
`asset://` URL and rejects local paths, base64, data URLs, and other schemes
without network access.

The BytePlus video adapter serializes each image reference as an
`image_url` item appended after the text content item. Text-only BytePlus
requests keep the existing payload shape and silent-video default.

Relaydance and Replicate reject non-empty image references before any provider
HTTP call. Provider errors from BytePlus still keep the existing provider,
adapter, model, and task context.
```

- [ ] **Step 4: Run focused test set**

Run:

```bash
cargo test -p puffer-resources video_generation_tool_schema media_generation_skills
cargo test -p puffer-cli media_internal_tools
cargo test -p puffer-core video_generation
cargo test -p puffer-core -- byteplus_request_body relaydance_rejects_video_image_references
```

Expected: all commands PASS.

- [ ] **Step 5: Run broader crate tests if time permits**

Run:

```bash
cargo test -p puffer-resources
cargo test -p puffer-cli
cargo test -p puffer-core
```

Expected: PASS, or document any pre-existing unrelated failures with the exact failing test names and messages.

- [ ] **Step 6: Commit specs and any final fixes**

```bash
git add specs/puffer-resources/108.md specs/puffer-cli/219.md specs/puffer-core/275.md
git commit -m "docs: record video image reference updates"
```

## Self-Review Notes

- Spec coverage: resource schema, skill guidance, CLI payload, workflow parsing, validation, exact runtime request, BytePlus serialization, Relaydance rejection, tests, and component specs are all covered.
- Overdesign check: no generic media reference enum, no role/source/kind fields, no local staging backend, no base64 support, and no video/audio references.
- Performance check: no public URL download, no local file read, no provider call before Relaydance rejection.
