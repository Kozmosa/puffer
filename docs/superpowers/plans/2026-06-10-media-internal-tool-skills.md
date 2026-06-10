# Media Internal Tool Skills Implementation Plan

Status: superseded for the remaining helper-invocation fix by
`docs/superpowers/plans/2026-06-10-media-internal-helper-skill-plan.md`.
This plan records the earlier internal-tool migration work; do not use its raw
`puffer internal-tool ...` skill-command tasks for new implementation.

> For agentic workers: implement task by task and keep checkbox status current.
> Do not implement a parallel media-specific internal tool mechanism. Reuse the
> existing internal tool resource, CLI, Bash broker, permission, and execution
> path.

Goal: Move agent image and video generation from model-facing
`ImageGeneration` / `VideoGeneration` tool calls to skill-guided foreground
Bash commands that invoke existing-style internal tools.

Spec: `docs/superpowers/specs/2026-06-10-media-internal-tool-skill-design.md`

Non-negotiable constraint: do not modify
`crates/puffer-core/runtime/system_prompt.rs`.

## Recheck Outcome

Current code already has the internal tool structure this work should use:

- `resources/internal_tools/*.yaml` are loaded into
  `LoadedResources.internal_tools`.
- `ToolRegistry::from_resources` registers those definitions through
  `register_internal`.
- `puffer internal-tool ...` dispatches through `InternalToolCommand` and
  `run_internal_tool_command`.
- Internal CLI commands call `require_internal_tool_execution_from_env`.
- Foreground Bash starts the existing broker and drains
  `InternalToolExecutionRequest` messages.
- `resolve_internal_tool_permission` and
  `execute_internal_tool_request` own parent-side permission and execution.

Important gaps found during the recheck:

- Ordinary `SKILL.md` `allowed-tools` is not enforcement. Only verified Lambda
  Skills install request-scoped filters. Security comes from removing direct
  media tools from the model-facing registry and using internal tool
  permission checks.
- `ToolRegistry::internal_definition` resolves exact ids and declared YAML
  aliases. The moved media internal tool YAML must declare CLI aliases such as
  `image-generation` and `video-generation`.
- The current Bash internal execution call does not pass providers, auth store,
  or exact media discovery cache. That context must be threaded into the
  existing parent execution path.
- New generated media timeline events will be successful `Bash` tool calls.
  The media JSON is nested in the Bash output JSON's `stdout` field.
- The daemon's deterministic `generate_media` RPC is separate from model-facing
  tool execution. Leave it alone unless a compile or test conflict appears.
- `crates/puffer-core/runtime/claude_tools/mod.rs` is already near the 1000
  line repo limit. Keep additions there very small; put media internal
  execution logic in `runtime/internal_tool_permissions.rs` or a focused helper
  module if needed.

## File Structure

Resources and skills:

- Move: `resources/tools/image_generation.yaml` to
  `resources/internal_tools/image_generation.yaml`
- Move: `resources/tools/video_generation.yaml` to
  `resources/internal_tools/video_generation.yaml`
- Modify: `resources/skills/image-generation/SKILL.md`
- Modify: `resources/skills/video-generation/SKILL.md`
- Modify: `crates/puffer-resources/tests/media_generation_skills.rs`
- Modify: `crates/puffer-resources/tests/video_tool_schema.rs`
- Modify: resource loader tests in `crates/puffer-resources/src/loader.rs`
- Modify: `crates/puffer-tools/src/registry_visibility_tests.rs`

CLI and internal tool descriptors:

- Modify: `crates/puffer-tools/src/internal_tools.rs`
- Modify: `crates/puffer-cli/src/cli_args.rs`
- Modify: `crates/puffer-cli/src/internal_tools.rs`
- Modify or create one small CLI helper module if needed to keep files below
  repo size limits. Do not create a new internal tool framework.

Parent runtime execution:

- Modify: `crates/puffer-core/runtime/claude_tools/mod.rs`
- Modify: `crates/puffer-core/runtime/internal_tool_permissions.rs`
- Add focused tests under the existing puffer-core test modules.

Timeline attachment parsing:

- Modify: `crates/puffer-cli/src/desktop_api.rs`
- Modify: `apps/puffer-desktop/src-tauri/src/backend.rs`
- Update related tests and fake timeline fixtures that currently seed direct
  `ImageGeneration` / `VideoGeneration` tool events.

Component update specs during implementation:

- Create next unused `specs/puffer-resources/NN.md`
- Create next unused `specs/puffer-tools/NN.md`
- Create next unused `specs/puffer-cli/NN.md`
- Create next unused `specs/puffer-core/NN.md`
- Create next unused `specs/puffer-desktop/NN.md` if the Tauri/backend
  timeline behavior changes

## Task 1: Move Media Tool Resources To Internal Tools

Files:

- `resources/tools/image_generation.yaml`
- `resources/tools/video_generation.yaml`
- `resources/internal_tools/image_generation.yaml`
- `resources/internal_tools/video_generation.yaml`
- `crates/puffer-resources/src/loader.rs`
- `crates/puffer-resources/tests/video_tool_schema.rs`
- `crates/puffer-tools/src/registry_visibility_tests.rs`

- [x] Move, not copy, the two media YAML manifests into
  `resources/internal_tools/`.
- [x] Preserve canonical ids `ImageGeneration` and `VideoGeneration`.
- [x] Add explicit aliases:
  - image: `image-generation`, `imagegen`
  - video: `video-generation`, `videogen`
- [x] Keep handlers as `runtime:workflow:image_generation` and
  `runtime:workflow:video_generation`.
- [x] Keep approval and sandbox policy aligned with the current direct tools.
- [x] Update loader tests so image/video are asserted in
  `loaded.internal_tools`, not `loaded.tools`.
- [x] Update `video_tool_schema.rs` to read the internal tool YAML path.
- [x] Update registry visibility tests:
  - `registry.internal_definition("ImageGeneration")` exists.
  - alias lookup through `image-generation` and `video-generation` works.
  - `registry.definition("ImageGeneration")` and
    `registry.definition("VideoGeneration")` return none.

Verification:

- [ ] `cargo test -p puffer-resources media_generation` (blocked by missing
  `resources/providers/minicpm5.yaml` in
  `crates/puffer-resources/tests/image_catalog_governance.rs`; focused
  `--test media_generation_skills` and `--lib image_generation` pass)
- [ ] `cargo test -p puffer-resources video_generation_tool_schema` (blocked by
  the same missing `minicpm5.yaml` fixture; focused `--test video_tool_schema`
  and `--lib video_generation` pass)
- [x] `cargo test -p puffer-tools registry_visibility`

## Task 2: Rewrite Existing Media Skills

Files:

- `resources/skills/image-generation/SKILL.md`
- `resources/skills/video-generation/SKILL.md`
- `crates/puffer-resources/tests/media_generation_skills.rs`

- [x] Change both skills to `allowed-tools: Bash`.
- [x] Update descriptions so they no longer say "before calling the
  ImageGeneration/VideoGeneration tool".
- [x] Instruct the model to use foreground Bash only.
- [x] Instruct the model to set an explicit long Bash timeout, within the
  current Bash cap.
- [x] Use explicit CLI commands:
  - `puffer internal-tool image-generation --prompt ... --count ...`
  - `puffer internal-tool video-generation --prompt ...`
- [x] Preserve image behavior:
  - one logical request maps to one command
  - `--count` carries multi-image requests
  - prompt file paths are passed through `--prompt`
  - no handcrafted fallback art is presented as generated output
- [x] Preserve video behavior:
  - text-to-video only
  - scalar parameters only
  - no success claim without a persisted video artifact
- [x] Update tests to assert Bash guidance and internal CLI references.
- [x] Add test text that documents `allowed-tools` as guidance, not the
  enforcement boundary, if a nearby test location is suitable.

Verification:

- [x] `cargo test -p puffer-resources media_generation_skills`

## Task 3: Add Existing-Style Internal CLI Commands

Files:

- `crates/puffer-tools/src/internal_tools.rs`
- `crates/puffer-cli/src/cli_args.rs`
- `crates/puffer-cli/src/internal_tools.rs`
- `crates/puffer-cli/src/subscriber_tools.rs` or a small shared helper module

- [x] Add media descriptors to `INTERNAL_CLI_TOOLS` so shell helpers and
  `puffer internal-tool aliases` include `imagegen` and `videogen`.
- [x] Add `InternalToolCommand` variants for `image-generation` and
  `video-generation`.
- [x] Add CLI aliases `imagegen` and `videogen` only if they can be expressed
  through clap subcommand aliases without adding custom dispatch.
- [x] Keep CLI args explicit, not a generic JSON passthrough:
  - image: `--prompt`, required `--count`, optional `--aspect`,
    optional `--prompt-reference`, optional `--purpose`, optional
    `--retry-from-error-json`
  - video: `--prompt`, optional `--purpose`, optional `--parameters-json`
- [x] Parse `--retry-from-error-json` and `--parameters-json` at the CLI
  boundary and let the existing backend validate media-specific rules.
- [x] Build the same input JSON shape currently accepted by the workflow
  handlers.
- [x] Send requests through the existing
  `require_internal_tool_execution_from_env` path.
- [x] Print successful parent output exactly once, with no human wrapper.
- [x] Fail outside a parent Bash broker with the existing required-endpoint
  error.
- [x] Share the existing parent-execution helper pattern. If extraction is
  needed, make it a tiny helper; do not add a command registry or plugin system.

Verification:

- [x] Unit tests for media CLI argument parsing and JSON payload construction.
- [x] `cargo test -p puffer-tools internal_tool_shell_helpers`
- [x] `cargo test -p puffer-cli internal_tool`

## Task 4: Thread Media Context Through Parent Internal Execution

Files:

- `crates/puffer-core/runtime/claude_tools/mod.rs`
- `crates/puffer-core/runtime/internal_tool_permissions.rs`

- [x] Extend `execute_internal_tool_request` and its result helper to accept
  `ProviderRegistry`, `AuthStore`, and an exact media discovery cache reference
  or value.
- [x] At the Bash branch call site, build the discovery cache the same way the
  direct workflow branch currently does:
  - clone `state.exact_media_discovery_cache`
  - fall back to `ExactMediaDiscoveryCache::empty`
- [x] Keep the change in `claude_tools/mod.rs` small: pass context into the
  existing call and avoid moving workflow dispatch there.
- [x] In `internal_tool_permissions.rs`, keep existing email, telegram, and
  request-user-browser-action mappings intact.
- [x] Add explicit media branches keyed by `canonical_tool_name`:
  - `imagegeneration`
  - `videogeneration`
- [x] For media branches, call the existing workflow functions directly:
  - `workflow::image_generation::execute_image_generation`
  - `workflow::video_generation::execute_video_generation`
- [x] Build `ImageGenerationMediaContext` and `VideoGenerationMediaContext`
  from the current providers, auth store, and discovery cache.
- [x] Do not expose or make public a generic
  `execute_workflow_tool_with_media_context`.
- [x] Preserve internal permission behavior: every execution request still
  resolves permission before running the backend.

Verification:

- [x] Focused puffer-core tests for internal execution mapping.
- [x] A media internal execution request receives media context rather than
  failing with "media runtime is not configured" when test providers/auth are
  supplied.
- [x] Existing email, telegram, and request-user-browser-action internal
  execution tests still pass.

## Task 5: Parse Generated Media Attachments From Bash Output

Files:

- `crates/puffer-cli/src/desktop_api.rs`
- `apps/puffer-desktop/src-tauri/src/backend.rs`
- related fake daemon and timeline tests

- [x] Change generated-media extraction helpers to receive `tool_id`, raw
  `input`, and raw `output`.
- [x] For new sessions, only synthesize generated media when:
  - `tool_id == "Bash"`
  - the Bash input JSON command is a supported media internal command or helper
    alias
  - the Bash output JSON parses
  - the parsed Bash `stdout` parses as media result JSON
- [x] Recognize only supported command forms:
  - `puffer internal-tool image-generation ...`
  - `puffer internal-tool video-generation ...`
  - generated helper aliases `imagegen ...` and `videogen ...`
- [x] Do not parse arbitrary successful Bash JSON as generated media.
- [x] Reuse the existing image/video artifact constructors once the nested
  media JSON is parsed.
- [x] Update tests that currently seed direct `ImageGeneration` or
  `VideoGeneration` events to seed the new Bash shape.
- [x] Add negative tests for arbitrary Bash output containing media-shaped JSON.
- [x] Keep Tauri and CLI daemon timeline behavior aligned. If Tauri only needs
  image attachments today, either add the same image+video helper or explicitly
  document why video remains daemon-only in the component spec.
- [x] Do not refactor the desktop daemon's deterministic `generate_media` RPC.

Verification:

- [x] `cargo test -p puffer-cli timeline_synthesizes`
- [x] `cargo test -p puffer-cli generated_media`
- [x] `cargo test --manifest-path apps/puffer-desktop/src-tauri/Cargo.toml tauri_timeline_attaches_generated`
  or the closest focused Tauri backend test target available in this workspace.

## Task 6: Remove Direct Model-Facing Media Surface

Files:

- resource tests and any model-facing tool snapshots
- any app/e2e fixtures that assumed direct model tool availability

- [x] Search for remaining assumptions that `ImageGeneration` or
  `VideoGeneration` are model-facing tools.
- [x] Remove or update direct provider tool fixture expectations.
- [x] Keep backend media workflow implementation tests; they still validate the
  real media backend and should not be rewritten around CLI details.
- [x] Keep deterministic desktop `generate_media` tests if they do not depend
  on model-facing tool visibility.
- [x] Ensure prompt resources still load without relying on
  `system_prompt.rs` changes.

Verification:

- [x] Run focused searches for `ImageGeneration`, `VideoGeneration`,
  `image_generation.yaml`, and `video_generation.yaml`; remaining hits are
  either internal tool assertions, backend workflow ids, or media runtime
  tests, not model-facing tool or skill guidance. Old `resources/tools/...`
  path hits are limited to historical plan/spec references plus updated
  internal-tool path assertions.
- [ ] `cargo test -p puffer-resources` (blocked by missing
  `resources/providers/minicpm5.yaml` in
  `crates/puffer-resources/tests/image_catalog_governance.rs`)
- [x] `cargo test -p puffer-tools`
- [x] `cargo test -p puffer-cli`
- [x] `cargo test -p puffer-core`

## Task 7: Component Specs And Final Verification

Files:

- next unused `specs/puffer-resources/NN.md`
- next unused `specs/puffer-tools/NN.md`
- next unused `specs/puffer-cli/NN.md`
- next unused `specs/puffer-core/NN.md`
- next unused `specs/puffer-desktop/NN.md`, if timeline behavior changes

- [x] Document final resource visibility: media generation is internal only.
- [x] Document CLI command shape and broker requirement.
- [x] Document parent runtime context threading for media internal execution.
- [x] Document timeline attachment parsing from guarded Bash stdout.
- [x] Run formatting for touched Rust code.
- [x] Run the focused tests from prior tasks.
- [ ] If time allows, run `cargo test --workspace`. If not, report focused
  coverage and remaining risk. Not run because
  `cargo test -p puffer-resources` is blocked by the pre-existing missing
  `minicpm5.yaml` fixture.

Exit criteria:

- `ImageGeneration` and `VideoGeneration` no longer appear as model-facing
  provider tools.
- The media skills guide the model to foreground Bash internal CLI commands.
- `puffer internal-tool image-generation` and
  `puffer internal-tool video-generation` work only inside the parent Bash
  broker.
- Parent runtime media execution uses the same providers, auth store, and media
  discovery cache as the old direct workflow path.
- Generated media attachments render from the new Bash event shape.
- No `system_prompt.rs` changes are present.
