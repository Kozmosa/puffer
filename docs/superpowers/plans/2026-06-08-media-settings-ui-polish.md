# Media Settings UI Polish Execution Plan

**Goal:** Make image and video generation settings modals use the established
loading visual pattern and hide dropdowns that only have one meaningful option.

**Spec:** `docs/superpowers/specs/2026-06-08-media-settings-ui-polish-design.md`

**Scope:** Desktop frontend modal and focused Playwright coverage. No backend
contract, media capability DTO, config schema, generation runtime, or global
component changes.

## Scope Check

In scope:

- `Image generation settings` and `Video generation settings`;
- the existing `MediaSettingsModal.svelte` loading states;
- provider, model, image parameter, aspect ratio, and duration field rendering;
- stale saved-selection warnings and recovery behavior;
- focused Playwright tests in existing desktop test files.

Out of scope:

- global loading or form components;
- schema changes for media settings or video adapter persistence;
- capability discovery, generation execution, or daemon behavior changes;
- screenshot baselines, visual diff tooling, or new test frameworks;
- advanced settings sections, accordions, help text, or redesign of the modal.

## Expected File Touches

- `apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte`
  - Add local loading block markup and CSS.
  - Add local single-option rendering helpers/snippets.
  - Keep image identity provider + model + adapter.
  - Keep video identity provider + model for saved settings.

- `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
  - Extend existing media settings tests and add focused single-option/loading
    coverage.

- `docs/superpowers/specs/2026-06-08-media-settings-ui-polish-design.md`
  - Updated design record.

No fake daemon changes are expected unless an existing helper cannot delay or
shape `list_media_capabilities` responses for the needed tests.

## Task 1: Add Focused UI Regressions

- [ ] Add a loading-state assertion by delaying `list_media_capabilities` for
  image settings.
- [ ] Assert the modal exposes a status loading block with spinner text while
  the capability request is pending.
- [ ] Add an image capability fixture with exactly one provider, one model, and
  one value for at least one parameter.
- [ ] Assert single-choice provider/model/parameter controls are not rendered
  as comboboxes and their read-only values are visible.
- [ ] Keep or extend the existing multi-provider image settings test so it
  still verifies dropdowns and save payloads when multiple options exist.
- [ ] Add or extend video settings coverage so single-option aspect ratio and
  duration render as read-only values.
- [ ] Preserve the stale saved image selection test and assert it still shows
  the unavailable warning rather than collapsing into read-only values.

Implementation guard: do not add screenshot tests or a new component test
framework for this small polish.

## Task 2: Tighten Capability Selection Helpers

- [ ] Add small local helper functions in `MediaSettingsModal.svelte` for:
  - current capability matching;
  - unavailable provider detection;
  - unavailable model detection;
  - single-option field decisions.
- [ ] Match image capabilities by provider, model, and adapter.
- [ ] Match video saved capabilities by provider and model only because video
  settings do not persist adapter identifiers.
- [ ] Keep `capabilityKey()` for editable select option values so duplicate
  model ids with different adapters remain distinguishable while editing.
- [ ] Keep `chooseDefaultCapability()`, `selectCapability()`, and
  `normalizeParameters()` as the only places that mutate selection/default
  state.

Implementation guard: do not add adapter persistence to video settings in this
change.

## Task 3: Render Single-Option Fields As Read-Only Values

- [ ] Add one local read-only value row style/snippet that reuses the existing
  field label spacing.
- [ ] Render provider as read-only only when exactly one provider is available
  and no unavailable saved provider needs to be shown.
- [ ] Render model as read-only only when exactly one model/capability is
  available for the selected provider and no unavailable saved model needs to
  be shown.
- [ ] Render image parameters as selects only when `parameter.values.length > 1`.
- [ ] Render image parameters with one value as read-only values.
- [ ] Render image parameters with zero values as read-only defaults without
  adding validation UI.
- [ ] Render video aspect ratio and duration as read-only values while their
  option arrays contain one value.
- [ ] Keep the image output folder row unchanged.

Implementation guard: do not introduce a generic `Field` component or dynamic
form schema renderer.

## Task 4: Align Loading UI Locally

- [ ] Replace plain media loading paragraphs with a local loading block.
- [ ] Include `role="status"` and `aria-live="polite"`.
- [ ] Add a small spinner with the same visual language as the connector setup
  spinner.
- [ ] Use distinct primary and secondary text for settings snapshot loading and
  capability loading.
- [ ] Keep existing error, warning, no-capability, footer, close, and Escape
  behavior unchanged.

Implementation guard: do not move connector loading CSS or create a shared
loading component in this slice.

## Task 5: Verification

- [ ] Run Svelte diagnostics:

```bash
npm --prefix apps/puffer-desktop run check
```

- [ ] Run focused media settings UI tests:

```bash
npm --prefix apps/puffer-desktop run test:desktop -- tests/chat-session-ui.spec.ts -g "generation settings"
```

- [ ] If the test script name differs in this repo state, use the closest
  existing desktop Playwright script and the same grep filter.
- [ ] Inspect `git diff` to confirm no backend, DTO, runtime, or global style
  changes were introduced.

## Stop Conditions

Stop and revisit the spec if implementation appears to require:

- a new global loading component;
- a generic field renderer;
- media config schema changes;
- video adapter persistence;
- daemon or Tauri command changes;
- capability discovery changes;
- broad rewiring outside `MediaSettingsModal.svelte` and focused tests.
