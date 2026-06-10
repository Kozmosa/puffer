# Short Drama Generation Internal Tool Design

Date: 2026-06-10

## Summary

Puffer should add a product-grade `ShortDramaGeneration` internal tool for
one-click short-drama project package generation.

The first implementation should be a deterministic package-and-render
orchestrator. It should not call an LLM from inside the runtime. The companion
skill turns the user's natural-language brief into a strict short-drama plan,
then calls the `shortdrama` helper. The internal tool validates that plan,
persists inspectable project files, generates one video artifact per shot
through the existing `VideoGeneration` runtime path, updates a manifest after
each shot, and returns a compact JSON summary.

This keeps the product useful while avoiding a generic media workflow engine,
background queue, provider abstraction layer, video compositor, or hidden
agent-loop inside tool execution.

## Recheck Outcome

The design was narrowed before implementation planning:

- Do not perform runtime-internal LLM planning. Agent-side skill guidance owns
  story and shot planning.
- Do not generate keyframes in the first version. That would pull in image
  generation without a supported local-image-to-video handoff.
- Do not expose provider-neutral duration, aspect, or resolution fields that
  require cross-provider translation. Forward optional scalar video parameters
  exactly like `VideoGeneration`.
- Do not create `artifacts.json`; artifact references live in `manifest.json`
  and the returned summary.
- Do not add concurrency, automatic retries, final composition, subtitles,
  audio, a database, or a job queue.

## Product Contract

From the user's perspective, one natural-language request can create a short
drama package. Internally, the agent performs two deterministic steps:

1. The `short-drama-generation` skill writes a strict JSON plan from the brief.
2. The `shortdrama` helper sends that plan to the internal runtime tool.

The project package lives under:

```text
.puffer/media/short-dramas/<short_drama_id>/
  manifest.json
  script.md
  shots.json
  prompts/
    shot-001-video.md
    shot-002-video.md
```

The package includes:

- a stable generated short-drama id
- the original brief and normalized plan metadata
- readable script, character, style, and continuity notes
- a structured shot list
- one video prompt file per shot
- generated video artifact references for successful shots
- shot-level failure records for unsuccessful shots

The first implementation does not promise:

- final MP4 concatenation
- subtitles or burned captions
- voiceover, music, or audio mixing
- keyframe or image generation
- local image staging for video references
- automatic asset upload
- automatic retries or retry commands
- background recovery
- arbitrary long-form productions

## Internal Tool And Skill Boundary

The companion skill is agent-facing process guidance. It decides when the
workflow applies, maps the user's brief into the strict plan shape, writes the
plan file, calls `shortdrama`, and explains the resulting package.

The internal tool is deterministic product logic. It validates input, writes
package files, calls the existing video generation runtime, records partial
success, and returns machine-readable output.

The internal tool must not:

- ask the model to repair or rewrite the plan
- recursively invoke `shortdrama`, `videogen`, shell, or Bash
- synthesize placeholder artifacts
- hide failed shots behind a successful prose message

## Recommended User-Facing Flow

The skill writes a workspace-relative JSON plan file, then calls:

```bash
shortdrama \
  --brief "60 second vertical suspense short drama about ..." \
  --plan-file .puffer/tmp/short-drama-plan.json \
  --video-parameters-json '{"duration":"5","aspect_ratio":"9:16"}'
```

`--video-parameters-json` is optional. Its keys and scalar values are forwarded
unchanged to `VideoGeneration.parameters`. If a configured provider does not
support one of those parameters, the existing video runtime returns the error.

## Input Schema

The runtime payload should be explicit:

```json
{
  "brief": "string",
  "plan": {
    "title": "string",
    "logline": "string",
    "targetDurationSeconds": 60,
    "styleBible": "string",
    "continuityNotes": "string",
    "characters": [
      {
        "name": "string",
        "description": "string"
      }
    ],
    "shots": [
      {
        "shotId": "shot-001",
        "durationHintSeconds": 5,
        "scriptBeat": "string",
        "scene": "string",
        "camera": "string",
        "continuity": "string",
        "videoPrompt": "string"
      }
    ]
  },
  "videoParameters": {
    "duration": "5",
    "aspect_ratio": "9:16"
  },
  "purpose": "short-drama-package"
}
```

Rules:

- `brief`, `plan.title`, `plan.logline`, `plan.styleBible`,
  `plan.continuityNotes`, and `plan.shots` are required.
- `plan.shots` must contain 1 to 20 shots.
- Every shot requires `shotId`, `scriptBeat`, `scene`, `camera`, `continuity`,
  and `videoPrompt`.
- `shotId` must be unique and match `shot-001` style lowercase ids.
- `targetDurationSeconds` is metadata only and has an absolute cap of `180`.
- `durationHintSeconds` is metadata only and has an absolute cap of `30`.
- `videoParameters` accepts only string, number, or boolean values, matching
  `VideoGeneration.parameters`.
- The tool does not accept `imageReferences` in the first version. Users who
  need image-referenced video clips should use `videogen` directly.
- Prompt text limits:
  - `brief`: 4,000 characters
  - `styleBible`: 4,000 characters
  - `continuityNotes`: 4,000 characters
  - one `videoPrompt`: 8,000 characters

The CLI should accept `--plan-file` instead of requiring a huge shell argument.
It parses the file as JSON, sends the `plan` object in the internal execution
request, and preserves `--brief`, `--video-parameters-json`, and `--purpose`.

## State Model

Use one project-level status and one shot-level status.

Project statuses:

- `planned`: package files are written and media generation has not started.
- `running`: at least one shot is in progress or complete while other shots
  remain.
- `succeeded`: every shot has a video artifact.
- `partial`: at least one shot succeeded and at least one shot failed.
- `failed`: no shot produced a usable video artifact after package creation.

Shot statuses:

- `planned`
- `running`
- `succeeded`
- `failed`

Validation errors before package creation are internal tool execution failures.
After the package directory exists, the tool should return a JSON summary even
for `partial` or `failed` project status so the user can inspect persisted
state.

## Manifest Contract

`manifest.json` is the durable source of truth:

```json
{
  "manifestVersion": 1,
  "shortDramaId": "suspense-door-20260610-abc123",
  "status": "partial",
  "createdAtMs": 1781097600000,
  "updatedAtMs": 1781097700000,
  "brief": "string",
  "title": "string",
  "videoParameters": {
    "duration": "5"
  },
  "shots": [
    {
      "shotId": "shot-001",
      "status": "succeeded",
      "promptPath": "prompts/shot-001-video.md",
      "artifact": {
        "artifactId": "artifact-video-1",
        "jobId": "job-video-1",
        "path": ".puffer/media/videos/...",
        "mimeType": "video/mp4",
        "size": 12345
      },
      "error": null,
      "retryable": false
    }
  ]
}
```

Write `manifest.json` with a temp-file-and-rename pattern. Update it after
every shot so an interrupted run leaves the latest known state. Do not overwrite
an existing project directory with the same id.

## Runtime Architecture

Add these focused pieces:

- `resources/internal_tools/short_drama_generation.yaml`
  - Defines `ShortDramaGeneration`, aliases, schema, approval policy, network
    sandbox, and media display grouping.
- `resources/skills/short-drama-generation/SKILL.md`
  - Teaches the agent when to use `shortdrama`, how to produce the strict plan,
    how to call the helper, and how to explain partial output.
- `crates/puffer-tools/src/internal_tools.rs`
  - Adds the CLI-only descriptor and helper alias `shortdrama`.
- `crates/puffer-cli/src/media_internal_tools.rs`
  - Adds `ShortDramaGenerationArgs`, parses `--plan-file` and
    `--video-parameters-json`, and builds the JSON payload. It contains no
    business logic.
- `crates/puffer-cli/src/cli_args.rs`
  - Adds the hidden internal command and alias.
- `crates/puffer-core/runtime/internal_tool_permissions.rs`
  - Routes canonical `shortdramageneration` requests through normal internal
    permission resolution and then to the workflow executor.
- `crates/puffer-core/runtime/claude_tools/workflow/short_drama_generation.rs`
  - Owns validation, package persistence, direct `VideoGeneration` runtime
    calls, manifest updates, and summary output.
- `crates/puffer-core/media_runtime_internal_tools.rs`
  - Treats `shortdrama` as a generated video helper for timeline attachment
    extraction if the summary contains video artifacts.

Do not add a new crate, database, generic project engine, public provider
planning API, or background job system.

## Execution Flow

1. CLI boundary.
   - Read `--plan-file` from a safe workspace-relative path.
   - Parse `--plan-file` and `--video-parameters-json` as JSON.
   - Send `brief`, `plan`, `videoParameters`, and `purpose` to the parent
     runtime.

2. Permission and preflight.
   - Use the standard internal permission path.
   - Reject empty briefs, invalid plan shape, over-limit shot counts, duplicate
     shot ids, non-scalar video parameters, and missing video media config
     before creating the project directory.

3. Package creation.
   - Generate a short-drama id from a title slug, current date/time, and a short
     random suffix.
   - Create the project directory.
   - Write initial `manifest.json` with `planned` status.
   - Write `script.md`, `shots.json`, and `prompts/*.md`.

4. Video generation.
   - Run shots serially.
   - For each shot, set the shot status to `running`, update the manifest, and
     call `execute_video_generation` directly with the prompt file path,
     forwarded scalar parameters, and purpose metadata.
   - Parse the returned video JSON and record artifact metadata.
   - On error, record the error and retryability on that shot.

5. Summary.
   - Return compact JSON containing `shortDramaId`, `status`, `manifestPath`,
     `scriptPath`, `shotsTotal`, `shotsSucceeded`, `shotsFailed`, `artifacts`,
     and `retryableFailures`.

## Error Handling

Validation failures before package creation fail the internal tool call.

Once package creation starts, media failures are recorded per shot:

```json
{
  "shotId": "shot-003",
  "status": "failed",
  "error": "provider rate limit",
  "retryable": true,
  "promptPath": "prompts/shot-003-video.md"
}
```

Retry classification is conservative:

- Error messages containing rate limit, timeout, temporarily unavailable, or
  transient network wording are retryable.
- Invalid configuration, unsupported parameters, empty prompts, malformed plan
  data, and schema violations are not retryable.

The first version should not implement automatic retries, exponential backoff,
or a retry command.

## Performance Strategy

Use predictable serial generation.

Reasons:

- Video providers are the bottleneck.
- Serial execution avoids rate-limit amplification.
- Manifest updates stay simple and reliable.
- The first product value is inspectable project output, not maximum throughput.

Do not expose `maxConcurrency` or build an internal concurrency abstraction in
the first implementation.

## Companion Skill

Create a thin `short-drama-generation` skill with `skill-creator`. The skill is
usage guidance, not product logic.

It should teach the agent:

- when to use `shortdrama` instead of manual `videogen` calls
- how to turn the user's brief into the strict JSON plan
- how to keep shot counts within the v1 caps
- how to write the plan file and call the helper
- how to explain `succeeded`, `partial`, and `failed` package statuses
- that v1 does not create final MP4 compositions, keyframes, subtitles, audio,
  or image-referenced videos
- not to imply success without persisted artifacts

The skill should not include scripts, assets, or a `references/` directory in
the first version.

## Testing

Resource tests:

- `ShortDramaGeneration` loads as an internal tool.
- It is absent from normal model-facing tool definitions.
- The schema requires `brief` and `plan`.
- The skill documents `shortdrama`, `--plan-file`, and partial outputs.
- The skill does not instruct direct `puffer internal-tool` calls.

CLI tests:

- `shortdrama --brief ... --plan-file ...` serializes into the expected JSON
  input.
- `--video-parameters-json` accepts scalar values and rejects objects, arrays,
  and invalid JSON.
- Missing or malformed plan files fail at the CLI boundary.
- Missing parent internal execution endpoint fails clearly.

Core workflow tests:

- valid input creates the project directory and manifest
- empty brief fails before package creation
- invalid plans fail before package creation
- missing video media config fails before package creation
- planning artifacts are persisted before the first media call
- prompt files are generated from shot plan data
- successful shots write artifact references
- one failed shot produces project status `partial`
- all shot failures produce project status `failed` while returning a summary
- manifest updates happen after each shot
- over-limit shot counts and durations are rejected
- existing `ImageGeneration` and `VideoGeneration` behavior remains isolated

Generated media preview tests:

- `shortdrama ...` is detected as a supported generated video helper when it is
  a simple helper command.
- Raw `puffer internal-tool short-drama-generation ...` is not treated as a
  generated media helper.
- Shell control operators still cause command detection to reject the command.

Do not run real provider media generation in default CI. Use a private test
helper that injects a fake video executor into the short-drama workflow.

## Open Product Decisions

These are deliberately out of scope for the first implementation:

- final composition command and file contract
- retry command shape
- UI project viewer
- local image upload or staging
- image-referenced short-drama shots
- subtitle and audio generation

The first version should leave these as future additions around
`manifest.json`, not hooks inside the initial control flow.
