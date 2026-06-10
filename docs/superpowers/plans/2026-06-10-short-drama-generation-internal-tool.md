# Short Drama Generation Internal Tool Implementation Plan

Spec:
`docs/superpowers/specs/2026-06-10-short-drama-generation-internal-tool-design.md`

## Goal

Add a product-grade `ShortDramaGeneration` internal tool that creates an
inspectable short-drama project package and serially generates one video clip
per shot through the existing video media runtime.

## Architecture

The companion skill performs agent-side story planning and writes a strict JSON
plan file. The internal tool stays deterministic: validate, persist package
files, call `VideoGeneration` directly in Rust, update `manifest.json`, and
return a compact summary.

No runtime-internal LLM calls, keyframe generation, final MP4 composition,
background queue, provider adapter, retry command, or generic workflow engine.

## Tech Stack

Rust, `serde`, `serde_json`, `clap`, existing Puffer internal tool resources,
existing internal permission broker, existing exact video media runtime, and
focused Cargo tests with fake video execution.

## Recheck Outcome

- Plan JSON is required; the tool does not ask the model to plan or repair.
- `generateKeyframes`, `imageReferences`, and `artifacts.json` are out of v1.
- Optional video parameters are scalar values forwarded unchanged to
  `VideoGeneration.parameters`.
- `partial` and post-package `failed` statuses return JSON summaries instead
  of throwing away inspectable state.
- A private test helper should inject a fake video executor. Do not add a public
  trait, new crate, or broad runtime abstraction.

## File Structure

- Add: `resources/internal_tools/short_drama_generation.yaml`
- Add: `resources/skills/short-drama-generation/SKILL.md`
- Modify: `crates/puffer-resources/src/loader.rs`
- Add or modify: `crates/puffer-resources/tests/short_drama_tool_schema.rs`
- Modify: `crates/puffer-resources/tests/media_generation_skills.rs`
- Modify: `crates/puffer-tools/src/internal_tools.rs`
- Modify: `crates/puffer-tools/src/registry_visibility_tests.rs`
- Modify: `crates/puffer-cli/src/media_internal_tools.rs`
- Modify: `crates/puffer-cli/src/cli_args.rs`
- Modify: `crates/puffer-core/runtime/internal_tool_permissions.rs`
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/mod.rs`
- Add: `crates/puffer-core/runtime/claude_tools/workflow/short_drama_generation.rs`
- Modify: `crates/puffer-core/media_runtime_internal_tools.rs`
- Modify: `crates/puffer-core/media_runtime_generated_preview_tests.rs`
- Add: `specs/puffer-resources/110.md`
- Add: `specs/puffer-tools/15.md`
- Add: `specs/puffer-cli/220.md`
- Add: `specs/puffer-core/277.md`

## Task 1: Add Resource Schema And Companion Skill

Files:

- `resources/internal_tools/short_drama_generation.yaml`
- `resources/skills/short-drama-generation/SKILL.md`
- `crates/puffer-resources/src/loader.rs`
- `crates/puffer-resources/tests/short_drama_tool_schema.rs`
- `crates/puffer-resources/tests/media_generation_skills.rs`
- `specs/puffer-resources/110.md`

- [ ] Add failing tests that assert `ShortDramaGeneration` loads as an internal
      tool and is absent from model-facing tools.
- [ ] Assert aliases include `short-drama-generation` and `shortdrama`.
- [ ] Assert the schema requires `brief` and `plan`, rejects unknown top-level
      fields, and documents scalar `videoParameters`.
- [ ] Add skill assertions that the companion skill documents `shortdrama`,
      `--plan-file`, partial output handling, and does not mention direct
      `puffer internal-tool` usage.
- [ ] Add `resources/internal_tools/short_drama_generation.yaml` with handler
      `runtime:workflow:short_drama_generation`, approval policy `ask`,
      sandbox policy `network`, media display grouping, and a strict schema.
- [ ] Add `resources/skills/short-drama-generation/SKILL.md` with thin
      agent-side planning guidance. Keep it Bash-based and user-invocable.
- [ ] Add `specs/puffer-resources/110.md` describing resource contract,
      visibility, schema, skill behavior, and non-goals.
- [ ] Run `cargo test -p puffer-resources short_drama`.

## Task 2: Add Internal Helper Descriptor

Files:

- `crates/puffer-tools/src/internal_tools.rs`
- `crates/puffer-tools/src/registry_visibility_tests.rs`
- `specs/puffer-tools/15.md`

- [ ] Add failing helper tests showing `shortdrama()` is emitted and targets
      `puffer internal-tool short-drama-generation "$@"`.
- [ ] Add registry visibility assertions for `ShortDramaGeneration`,
      `short-drama-generation`, and `shortdrama` as internal-only definitions.
- [ ] Add the descriptor with `id: "short-drama-generation"`, alias
      `shortdrama`, and skill name `short-drama-generation`.
- [ ] Keep the descriptor static and CLI-only; do not add a generic dispatcher.
- [ ] Add `specs/puffer-tools/15.md` covering helper alias and internal-only
      visibility.
- [ ] Run `cargo test -p puffer-tools internal_tool_shell_helpers`.
- [ ] Run `cargo test -p puffer-tools registry_visibility`.

## Task 3: Add CLI Boundary

Files:

- `crates/puffer-cli/src/media_internal_tools.rs`
- `crates/puffer-cli/src/cli_args.rs`
- `specs/puffer-cli/220.md`

- [ ] Add failing CLI tests for:
      `shortdrama --brief ... --plan-file plan.json`.
- [ ] Add tests for scalar `--video-parameters-json`, invalid JSON, non-scalar
      values, missing plan files, malformed plan files, and missing parent
      internal execution endpoint.
- [ ] Add `ShortDramaGenerationArgs` with `--brief`, `--plan-file`,
      optional `--video-parameters-json`, and optional `--purpose`.
- [ ] Read `--plan-file` as a workspace-relative safe path; reject absolute
      paths and parent-directory traversal.
- [ ] Parse the plan file as JSON and build the parent execution payload.
- [ ] Add hidden `InternalToolCommand::ShortDramaGeneration` with command name
      `short-drama-generation` and alias `shortdrama`.
- [ ] Add `run_short_drama_generation` using the existing
      `execute_parent_internal_tool` helper.
- [ ] Add `specs/puffer-cli/220.md` covering CLI args, validation boundary, and
      payload shape.
- [ ] Run `cargo test -p puffer-cli short_drama`.
- [ ] Run `cargo test -p puffer-cli internal_tool`.

## Task 4: Add Core Workflow Executor

Files:

- `crates/puffer-core/runtime/internal_tool_permissions.rs`
- `crates/puffer-core/runtime/claude_tools/workflow/mod.rs`
- `crates/puffer-core/runtime/claude_tools/workflow/short_drama_generation.rs`
- `specs/puffer-core/277.md`

- [ ] Add failing tests for valid package creation with a fake video executor.
- [ ] Add validation tests for empty brief, missing required plan fields,
      duplicate or malformed shot ids, over-limit shot counts, over-limit
      duration metadata, non-scalar video parameters, and missing video media
      configuration.
- [ ] Add persistence tests for `manifest.json`, `script.md`, `shots.json`,
      prompt files, and manifest updates before each fake video call.
- [ ] Add status tests for all-success `succeeded`, mixed `partial`, and
      all-shot-failure post-package `failed`.
- [ ] Implement serde input structs with `deny_unknown_fields`.
- [ ] Implement plan validation and prompt rendering in the workflow module.
- [ ] Generate a collision-resistant id from title slug, date/time, and a short
      random suffix. Refuse to overwrite an existing project directory.
- [ ] Write files under `.puffer/media/short-dramas/<id>/`.
- [ ] Write manifest updates using temp file plus rename.
- [ ] Call `video_generation::execute_video_generation` directly in production.
      Use a private helper that accepts a video-executor closure for tests.
- [ ] Parse video generation JSON and record artifact metadata.
- [ ] Classify retryable errors conservatively from error text.
- [ ] Route canonical `shortdramageneration` in internal permission execution
      after normal generic permission resolution.
- [ ] Add `specs/puffer-core/277.md` covering runtime contracts, manifest
      schema, failure semantics, and test injection.
- [ ] Run `cargo test -p puffer-core short_drama_generation`.

## Task 5: Add Generated Media Preview Integration

Files:

- `crates/puffer-core/media_runtime_internal_tools.rs`
- `crates/puffer-core/media_runtime_generated_preview_tests.rs`

- [ ] Add failing tests showing `shortdrama --brief ... --plan-file ...` is
      detected as a generated video helper.
- [ ] Add tests showing raw
      `puffer internal-tool short-drama-generation ...` is not detected.
- [ ] Keep shell-control-operator rejection behavior unchanged.
- [ ] Map `shortdrama` to the video attachment extraction path so returned
      summary artifacts can appear like generated videos.
- [ ] Do not add broad shell parsing or special project-package UI.
- [ ] Run `cargo test -p puffer-core generated_media_internal_command_kind`.
- [ ] Run `cargo test -p puffer-core generated_media_timeline_attachments`.

## Task 6: Focused Verification

Run:

```sh
cargo test -p puffer-resources short_drama
cargo test -p puffer-tools internal_tool_shell_helpers
cargo test -p puffer-tools registry_visibility
cargo test -p puffer-cli short_drama
cargo test -p puffer-cli internal_tool
cargo test -p puffer-core short_drama_generation
cargo test -p puffer-core generated_media_internal_command_kind
cargo test -p puffer-core generated_media_timeline_attachments
```

Final checks:

- [ ] `rg "puffer internal-tool short-drama-generation" resources/skills`
      returns no matches.
- [ ] `rg "generateKeyframes|keyframe|artifacts.json" docs/superpowers/specs/2026-06-10-short-drama-generation-internal-tool-design.md resources/skills/short-drama-generation/SKILL.md`
      returns no matches except explicit non-goal wording if present.
- [ ] `git diff` contains only the intended resources, Rust tests/code,
      component specs, design spec, and plan changes.
- [ ] No default CI test calls a live media provider.
