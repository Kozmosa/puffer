# Agent VideoGeneration Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or the local execute-plan skill to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make agent chat video requests call a real `VideoGeneration` workflow tool instead of ending after an intention-only assistant reply.

**Architecture:** Add a dedicated text-to-video workflow tool that reuses the existing exact media runtime and configured `media.video` selection. Keep image generation intact. Generalize only the transcript attachment helper enough to attach successful video artifacts.

**Tech Stack:** Rust (`puffer-core`, `puffer-resources`, `puffer-cli`), YAML tool resources, Cargo tests.

**Spec:** `docs/superpowers/specs/2026-06-08-agent-video-generation-tool-design.md`

---

## Recheck Outcome

The design was tightened before planning:

- Do not rename `ImageGenerationMediaContext`.
- Add a sibling `VideoGenerationMediaContext`.
- Do not add a generic `MediaGeneration` tool.
- Do not add image-to-video fields or request plumbing.
- Do not expose desktop `generate_media` as a model tool.
- Do not add frontend prompt classification.

This plan fixes the missing agent tool surface only. Existing `openai_video`,
Relaydance capability resolution, settings persistence, and video polling stay
the execution substrate.

## File Structure

- Create: `resources/tools/video_generation.yaml`
  - Declares the model-facing `VideoGeneration` workflow tool.
- Modify: `crates/puffer-resources/src/loader.rs`
  - Adds bundled tool contract tests.
- Modify: `crates/puffer-core/runtime/system_prompt.rs`
  - Adds video-generation prompt guidance and tests.
- Create: `crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs`
  - Implements the text-to-video workflow tool.
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/mod.rs`
  - Exports the new workflow module.
- Modify: `crates/puffer-core/runtime/claude_tools/mod.rs`
  - Passes video media context and dispatches `VideoGeneration`.
- Modify: `crates/puffer-cli/src/desktop_api.rs`
  - Converts generated attachment synthesis from image-only to image/video.
- Create: `specs/puffer-core/256.md`
  - Component update spec for the agent video workflow tool.
- Create: `specs/puffer-resources/93.md`
  - Component update spec for the bundled video tool resource.
- Create: `specs/puffer-cli/165.md`
  - Component update spec for transcript generated-video attachments.

Do not modify desktop Svelte files unless the Rust DTO tests prove the existing
generated-media attachment UI cannot render video metadata.

---

## Task 1: Resource And Prompt Contract

**Files:**
- Modify: `crates/puffer-resources/src/loader.rs`
- Modify: `crates/puffer-core/runtime/system_prompt.rs`
- Create: `resources/tools/video_generation.yaml`

- [ ] **Step 1: Add the failing bundled tool resource test**

Add a test near the existing bundled `ImageGeneration` tool resource test:

```rust
#[test]
fn bundled_video_generation_tool_is_text_to_video_only() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    let paths = ConfigPaths::discover(&root);

    let loaded = load_tool_resources(&paths, &FsTestRunner).unwrap();
    let tool = loaded
        .tools
        .iter()
        .find(|tool| tool.value.id == "VideoGeneration")
        .expect("VideoGeneration tool");

    assert_eq!(tool.value.handler, "runtime:workflow:video_generation");
    assert!(tool.value.description.contains("text-to-video"));
    assert!(!tool.value.description.contains("image-to-video"));

    let schema = tool.value.input_schema.as_ref().expect("input schema");
    let required = schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .expect("required array");
    assert!(required.iter().any(|value| value == "prompt"));
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&serde_json::Value::Bool(false))
    );
    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("properties object");
    assert!(!properties.contains_key("image"));
    assert!(!properties.contains_key("referenceImage"));
    assert!(!properties.contains_key("firstFrame"));
    assert!(!properties.contains_key("lastFrame"));
}
```

- [ ] **Step 2: Run the focused resource test and verify failure**

```bash
cargo test -p puffer-resources bundled_video_generation_tool_is_text_to_video_only
```

- [ ] **Step 3: Add `resources/tools/video_generation.yaml`**

Create the YAML tool resource with:

- `id` and `name`: `VideoGeneration`
- `handler`: `runtime:workflow:video_generation`
- `approval_policy`: `ask`
- `sandbox_policy`: `network`
- required `prompt`
- optional `parameters` as string-valued scalar overrides
- optional `purpose`
- `additionalProperties: false`

Keep the description explicit that this is text-to-video only. Do not include
image/reference/frame fields.

- [ ] **Step 4: Add failing system prompt tests**

In `crates/puffer-core/runtime/system_prompt.rs`, extend the prompt tests so
they assert:

- prompt contains `use the VideoGeneration tool`
- prompt contains `text-to-video only`
- prompt contains `existing image` or `reference image`

- [ ] **Step 5: Run the focused prompt test and verify failure**

```bash
cargo test -p puffer-core system_prompt
```

- [ ] **Step 6: Update the system prompt**

Add one concise video-generation rule. It must say:

- use `VideoGeneration` for video creation/generation requests
- one logical video request means one `VideoGeneration` call
- existing image/reference/first-frame/last-frame video requests are not
  supported by the current agent tool
- video failures or unavailable capability should be reported plainly

- [ ] **Step 7: Re-run focused tests**

```bash
cargo test -p puffer-resources bundled_video_generation_tool_is_text_to_video_only
cargo test -p puffer-core system_prompt
```

---

## Task 2: VideoGeneration Workflow Tool

**Files:**
- Create: `crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/mod.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/mod.rs`

- [ ] **Step 1: Add failing workflow boundary tests**

In the new `video_generation.rs` test module, add tests for:

- missing `state.config.media.video` returns
  `video media provider/model/adapter is not configured`
- empty prompt returns `VideoGeneration prompt is required`
- unknown input fields are rejected by serde
- parameter overrides merge with saved `media.video.parameters`

Use the existing `ImageGeneration` tests as style reference, but do not move
shared helpers unless the compiler forces it.

- [ ] **Step 2: Run focused workflow tests and verify failure**

```bash
cargo test -p puffer-core video_generation
```

- [ ] **Step 3: Implement `VideoGenerationInput` and request building**

Implement only the boundary layer:

- `prompt: String`
- `parameters: BTreeMap<String, String>` defaulting to empty
- `purpose: Option<String>`
- `deny_unknown_fields`
- `VideoGenerationMediaContext`
- prompt text resolution for literal text or workspace-relative prompt file
- saved video parameters plus tool overrides
- `ExactMediaGenerationRequest { kind: "video", count: 1, ... }`

Do not add count, image path, URL, frame, seed, scheduler, or provider-specific
fields.

- [ ] **Step 4: Add failing success-path runtime test**

Add a test that uses a fake OpenAI-video/Relaydance-style HTTP server and a
temporary media workspace to prove:

- `execute_video_generation` calls exact video generation
- result JSON includes `kind = "video"`
- result JSON includes one artifact
- result JSON includes provider, model, status, and merged parameters

Use existing media runtime fake server patterns where possible.

- [ ] **Step 5: Implement execution**

Call `generate_exact_media_with_cache` with the context providers, auth store,
cwd, request, and discovery cache. Convert artifacts to the tool output shape.

- [ ] **Step 6: Wire module export and dispatcher**

Implement:

- `pub mod video_generation;`
- `execute_tool` constructs `VideoGenerationMediaContext` alongside the existing
  image context.
- `execute_workflow_tool_with_media_context` accepts separate optional image and
  video contexts.
- match arm `"VideoGeneration"` calls
  `workflow::video_generation::execute_video_generation`.

Do not introduce a generic context trait, enum, or `MediaGeneration` workflow
tool.

- [ ] **Step 7: Add dispatcher regression test**

Add or extend the dispatcher test so a `ToolDefinition` with id
`VideoGeneration` and handler `runtime:workflow:video_generation` reaches the
new workflow and receives media context.

- [ ] **Step 8: Re-run focused core tests**

```bash
cargo test -p puffer-core video_generation
cargo test -p puffer-core dispatcher_passes_media_context
```

---

## Task 3: Transcript Generated Video Attachments

**Files:**
- Modify: `crates/puffer-cli/src/desktop_api.rs`

- [ ] **Step 1: Add failing timeline test for `VideoGeneration`**

Add a test beside the existing persisted `ImageGeneration` attachment tests:

- transcript has successful `ToolInvocation { tool_id: "VideoGeneration" }`
- output has `jobId`, `kind = "video"`, and one `video/mp4` artifact
- next assistant message receives one generated-media attachment
- attachment id is `generated-video:<artifact_id>`
- attachment name is `Generated video`
- attachment kind is `video`
- attachment extension is `MP4`
- attachment source remains `GeneratedMedia`

- [ ] **Step 2: Run the focused puffer-cli test and verify failure**

```bash
cargo test -p puffer-cli video_generation
```

- [ ] **Step 3: Generalize attachment synthesis minimally**

Replace image-only synthesis with a helper that handles only:

- `ImageGeneration` -> existing image behavior unchanged
- `VideoGeneration` -> video metadata attachment

Rules:

- Keep existing `generated-image:<artifact_id>` ids for image outputs.
- Add `generated-video:<artifact_id>` ids for video outputs.
- Infer video kind from MIME prefix `video/`.
- Support common video extensions: `video/mp4` -> `MP4`,
  `video/webm` -> `WEBM`, otherwise `VIDEO`.
- Do not read video bytes into transcript memory.
- Do not add a frontend video player in this task.

- [ ] **Step 4: Re-run focused puffer-cli tests**

```bash
cargo test -p puffer-cli generated
cargo test -p puffer-cli video_generation
```

---

## Task 4: Component Update Specs

**Files:**
- Create: `specs/puffer-core/256.md`
- Create: `specs/puffer-resources/93.md`
- Create: `specs/puffer-cli/165.md`

- [ ] **Step 1: Write puffer-core update spec**

Document:

- new `VideoGeneration` workflow tool module
- prompt guidance
- dispatcher wiring
- exact media runtime reuse
- text-to-video boundary

- [ ] **Step 2: Write puffer-resources update spec**

Document:

- new bundled `resources/tools/video_generation.yaml`
- schema constraints
- no image/reference/frame inputs

- [ ] **Step 3: Write puffer-cli update spec**

Document:

- generated video transcript attachments
- no legacy aliases or compatibility shims for absent video tool names
- existing image attachment ids and metadata remain the current contract
- metadata-first video behavior

---

## Task 5: Verification Gate

**Files:**
- No production files unless earlier tasks reveal a direct issue.

- [ ] **Step 1: Run focused tests**

```bash
cargo test -p puffer-resources bundled_video_generation_tool_is_text_to_video_only
cargo test -p puffer-core video_generation
cargo test -p puffer-core system_prompt
cargo test -p puffer-cli video_generation
```

- [ ] **Step 2: Run broader affected crate tests**

```bash
cargo test -p puffer-resources
cargo test -p puffer-core
cargo test -p puffer-cli generated
```

- [ ] **Step 3: Inspect code size and public docs**

Check:

- no Rust source file exceeds 1000 lines
- every new public Rust function has a docstring
- no new non-ASCII text in Rust/YAML files unless already present in surrounding
  docs/tests
- no image-to-video fields slipped into schemas or Rust structs

- [ ] **Step 4: Final diff review**

```bash
git diff --stat
git diff -- resources/tools/video_generation.yaml
git diff -- crates/puffer-core/runtime/system_prompt.rs
git diff -- crates/puffer-core/runtime/claude_tools/mod.rs
git diff -- crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs
git diff -- crates/puffer-cli/src/desktop_api.rs
```

Confirm there are no unrelated edits before committing.
