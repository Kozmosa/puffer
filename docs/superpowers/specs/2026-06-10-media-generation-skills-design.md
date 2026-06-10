# Media Generation Skills Design

## Summary

Move image and video generation usage guidance out of the runtime fallback system
prompt and into two focused, model-invocable skills:

- `image-generation`
- `video-generation`

This is intentionally scoped to the existing `ImageGeneration` and
`VideoGeneration` tools. It does not introduce a generic routing framework for
tool-specific prompt guidance.

## Goals

- Keep the shared runtime system prompt focused on general agent behavior.
- Load media-generation instructions only when the user's request matches image
  or video generation.
- Preserve direct tool execution through the existing `ImageGeneration` and
  `VideoGeneration` tool definitions and executors.
- Make the trigger descriptions explicit enough that the model can reliably
  select the right skill before responding.
- Keep the implementation small and testable.
- Keep all media execution behavior in the existing media tools.

## Non-Goals

- No backward-compatibility shim for the removed fallback prompt text.
- No migration framework for every tool-specific instruction.
- No new media execution path.
- No verified Lambda Skill conversion.
- No changes to provider request construction, tool schema generation, or media
  provider runtime behavior.
- No `agents/openai.yaml`, scripts, references, or assets for these skills.

## Current State

`crates/puffer-core/runtime/system_prompt.rs` contains fallback prompt bullets
that tell the model to call `ImageGeneration` and `VideoGeneration` for media
requests. Normal runtime prompt loading prefers `resources/prompts/system-base.yaml`,
which already does not contain these media bullets. The fallback prompt and its
tests still encode the media-specific behavior.

The actual tools already live independently:

- `resources/tools/image_generation.yaml`
- `resources/tools/video_generation.yaml`
- `crates/puffer-core/runtime/claude_tools/mod.rs`

The existing `Skill` tool can load a model-invocable skill when the skill
description matches the user's request.

For non-verified skills, `allowed-tools` is metadata and instruction, not a new
runtime enforcement boundary. Verified Lambda Skills install stricter gates, but
these media skills are intentionally plain skills.

## Design

Add two bundled skills under `resources/skills`.

### `image-generation`

Path: `resources/skills/image-generation/SKILL.md`

Frontmatter:

- `name: image-generation`
- `description`: explicitly matches user requests to create or generate images
  and instructs the agent to use the `ImageGeneration` tool.
- `allowed-tools`: `ImageGeneration`
- `user-invocable: true`
- `disable-model-invocation: false`

Body guidance:

- Use `ImageGeneration` for image generation requests.
- Use one tool call for one logical image-generation request.
- For multiple images from one prompt, set `count` instead of issuing repeated
  single-image calls.
- Treat `prompt` as literal text unless it names a workspace-relative file.
- Use `promptReference` only when the request supplies additional prompt context.
- If the tool fails or media runtime is unavailable, report that plainly.
- Do not hand-author SVG, ASCII art, placeholder files, or other substitutes and
  present them as generated images.

### `video-generation`

Path: `resources/skills/video-generation/SKILL.md`

Frontmatter:

- `name: video-generation`
- `description`: explicitly matches user requests to create or generate
  text-to-video clips and instructs the agent to use the `VideoGeneration` tool.
- `allowed-tools`: `VideoGeneration`
- `user-invocable: true`
- `disable-model-invocation: false`

Body guidance:

- Use `VideoGeneration` for text-to-video generation requests.
- Use one tool call for one logical video-generation request.
- Treat `prompt` as literal text unless it names a workspace-relative file.
- Pass only scalar values through `parameters`.
- State clearly that this tool is text-to-video only when the user asks for
  reference-image, first-frame, last-frame, or image-to-video behavior.
- If the tool fails or media runtime is unavailable, report that plainly.
- Do not imply a video was created unless the tool returns a persisted artifact.

## Request Flow

1. The runtime lists model-invocable skills in the session guidance when the
   `Skill` tool is available.
2. For an image request, the model selects `image-generation` with the `Skill`
   tool, then follows the loaded skill instructions and calls `ImageGeneration`.
3. For a text-to-video request, the model selects `video-generation` with the
   `Skill` tool, then follows the loaded skill instructions and calls
   `VideoGeneration`.
4. Tool schemas, permissions, provider selection, media artifact persistence, and
   failure handling remain in the existing tool/runtime code.

Because the new skills are prompt-only skills, they should not try to narrow the
active request tool set or install a Lambda gate. The selected skill tells the
model what to do; the existing media tool definitions and runtime continue to
validate actual inputs.

## Prompt Changes

Remove the two media-specific bullets from the fallback
`SYSTEM_PROMPT_TEMPLATE` in `crates/puffer-core/runtime/system_prompt.rs`.

Do not add replacement text to `resources/prompts/system-base.yaml`; the routing
signal should come from the model-invocable skill summaries and the loaded skill
body.

## Testing

Update tests that currently assert media guidance exists in the fallback system
prompt.

Add focused tests for:

- The new skill files parse as valid bundled skills.
- `image-generation` is model-invocable.
- `video-generation` is model-invocable.
- The rendered runtime system prompt lists both skills when `Skill` is enabled.
- The image skill body contains the `ImageGeneration` tool name, one-call rule,
  `count` rule, and no-fake-artifact rule.
- The video skill body contains the `VideoGeneration` tool name, one-call rule,
  text-to-video limitation, and persisted-artifact requirement.

Do not add provider-level tests. Tool request construction is unchanged.

Do not add tests to `crates/puffer-resources/src/loader.rs`; that file is already
large. Prefer a focused integration test under `crates/puffer-resources/tests/`
for bundled skill content and the existing `system_prompt.rs` tests for prompt
summary behavior.

## Stability and Performance

This design intentionally trades one extra `Skill` call for a smaller default
system prompt and more targeted media guidance. The extra step is acceptable
because media generation is an explicit user-requested workflow, not a hot path
inside normal coding turns.

Stability depends on concise skill descriptions. They should mention the exact
user intent and exact tool name so the model can select the skill without
needing broader inference.

## Overdesign Guardrails

- Only add two skills.
- Do not introduce shared media-skill helpers.
- Do not add new metadata fields to skills.
- Do not duplicate full tool schemas in skill bodies.
- Do not change the `Skill` tool or model-invocable skill listing logic.
- Do not create a generic prompt-migration abstraction.
- Do not add separate reference files for these skills.
- Do not introduce a new media routing layer.

## Execution Plan

### Phase 1: Add the two media skills

Files:

- `resources/skills/image-generation/SKILL.md`
- `resources/skills/video-generation/SKILL.md`

Tasks:

- Add concise frontmatter with explicit trigger descriptions.
- Keep each skill body short and tool-specific.
- Use only `ImageGeneration` in the image skill `allowed-tools`.
- Use only `VideoGeneration` in the video skill `allowed-tools`.
- Do not add `agents/openai.yaml`, references, scripts, or shared helper files.

Verification:

- Confirm both files use valid frontmatter.
- Confirm both names are lowercase hyphen-case.
- Confirm both set `disable-model-invocation: false`.

Exit criteria:

- Both skills can be loaded as bundled resources and describe only their matching
  media tool.

### Phase 2: Remove fallback system prompt media bullets

Files:

- `crates/puffer-core/runtime/system_prompt.rs`

Tasks:

- Remove the two image/video generation bullets from `SYSTEM_PROMPT_TEMPLATE`.
- Remove or replace tests that assert those bullets appear in the fallback
  system prompt.
- Preserve unrelated system prompt behavior and tests.

Verification:

- Run the focused `puffer-core` system prompt tests.
- Confirm the fallback prompt no longer contains `ImageGeneration` or
  `VideoGeneration` unless supplied by loaded skill summaries.

Exit criteria:

- The shared fallback system prompt no longer carries media-tool instructions.

### Phase 3: Add focused regression coverage

Files:

- `crates/puffer-resources/tests/media_generation_skills.rs`
- `crates/puffer-core/runtime/system_prompt.rs`

Tasks:

- Add a resource-level test that checks both bundled skill files for required
  names, model invocation setting, allowed tools, and key body snippets.
- Add or update a runtime prompt test that builds `LoadedResources` with the two
  skills and `Skill` enabled, then asserts the model-invocable skill summary
  lists both skill names and descriptions.
- Keep tests string-focused; do not start provider sessions or media runtimes.

Verification:

- `cargo test -p puffer-resources media_generation_skills`
- `cargo test -p puffer-core runtime_system_prompt`

Exit criteria:

- Focused tests prove the migration without exercising unrelated provider or
  media execution paths.

### Phase 4: Final validation

Tasks:

- Review the final diff for scope creep.
- Confirm no tool schema, provider request, or media runtime code changed.
- Confirm no generic migration abstraction was introduced.

Verification:

- `cargo test -p puffer-resources media_generation_skills`
- `cargo test -p puffer-core runtime_system_prompt`
- Optionally run `cargo test -p puffer-core -p puffer-resources` if the focused
  suites pass and time permits.

Exit criteria:

- The change is limited to two skills, fallback prompt cleanup, and focused
  tests.
