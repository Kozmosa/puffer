# Media Settings UI Polish Design

## Summary

The composer media settings modals should feel consistent with the existing
desktop loading surfaces and avoid showing inert dropdowns. This change covers
both `Image generation settings` and `Video generation settings` in
`MediaSettingsModal.svelte`.

The design intentionally keeps the work local to the modal. It does not add a
global form framework or preserve old UI behavior for single-option selects.

## Goals

- Use the established spinner-and-message loading pattern for media settings.
- Hide dropdown controls when a field has only one available choice.
- Keep capability loading lazy and preserve the current media config contract.
- Improve clarity without adding broad abstractions or new backend behavior.

## Non-Goals

- No generic dynamic form system.
- No global loading component extraction in this slice.
- No changes to media capability DTOs, saved config shape, or generation
  execution.
- No compatibility mode for the old single-option dropdown UI.

## UX Design

`settingsReady === false` and capability RPC loading render the same local media
loading block:

- `role="status"` and `aria-live="polite"`;
- a small rotating spinner;
- a primary line such as `Loading generation settings...`;
- a secondary line that explains the wait, for example `Waiting for media
  defaults from the daemon.`;
- borders, spacing, background, and typography aligned with the existing
  connector setup loading surface.

The modal keeps existing warning and empty states, including unavailable saved
model warnings and `No image capabilities available.` / `No video capabilities
available.` messages.

## Field Rendering

Each selectable field uses the same rule:

- If there are two or more valid choices, render the existing `<select>`.
- If there is exactly one valid choice, render a read-only value row instead of
  a dropdown.
- If the current saved value is unavailable, keep the existing unavailable
  select/warning behavior so the stale value is visible and actionable.

The rule applies to:

- provider selection;
- model/capability selection;
- image parameters such as size, quality, and output format;
- video aspect ratio;
- video duration.

The read-only value row is a simple local element, not an input. It uses the
same label spacing as fields and a muted bordered value container so it reads as
configuration, not editable text.

## State And Data Flow

Selection state remains owned by the existing variables:

- `providerId`;
- `modelId`;
- `adapter`;
- `parameters`;
- `aspectRatio`;
- `durationSeconds`.

The UI change does not bypass `chooseDefaultCapability()`,
`selectCapability()`, or `normalizeParameters()`. Hidden single-choice fields
still participate in saves through the current state values. Saving continues
to call `updateConfig({ media })` with the current `MediaSettings` shape.

## Stability And Performance

The implementation stays inside `MediaSettingsModal.svelte`, so there is no
cross-screen coupling and no additional RPC. Capability resolution remains lazy
and still happens only when the modal opens. Derived option lists are reused for
rendering decisions, avoiding extra scans beyond the small capability arrays
already present in the modal.

## Tests

Playwright coverage should verify:

- media loading shows the spinner/status loading block while capabilities are
  delayed;
- image settings with multiple providers/models/parameter values still render
  dropdowns and save correctly;
- image settings with one provider, one model, and one parameter value render
  read-only values instead of dropdowns;
- video settings render read-only aspect ratio and duration when each has one
  option;
- stale saved image selections still show the unavailable warning and do not
  collapse into a read-only value.
