# Chat Composer Autosize Design

## Summary

The primary Chat composer in Corbina should grow vertically as the user types
multi-line content, then cap at a fixed maximum height and scroll internally for
very long prompts. The change is limited to the main agent chat composer in
`apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`.

This deliberately does not introduce a shared composer component or apply the
behavior to the mini window, Ask Puffer, or legacy `ConversationPane`. The goal
is to improve the main coding chat input while keeping the implementation small,
stable, and easy to reason about.

## Reference

`agentenv-monorepo/packages/agent-sdk/src/components/Composer.tsx` uses a simple
native textarea autosize pattern:

- keep the textarea as a native `<textarea>`
- set height to `auto`
- read `scrollHeight`
- apply a clamped pixel height
- cap very long input and let the textarea scroll internally

Puffer should adopt this core behavior only. It should not copy unrelated
agent-sdk features such as file mentions, highlighted overlay text, scheduling
controls, or a generalized component hierarchy.

## Current State

`ConversationView.svelte` renders the main composer textarea as:

- `value={draft}`
- `oninput={(event) => updateDraft(event.currentTarget.value)}`
- `onkeydown={onKeydown}`
- `disabled={composerDisabled}`

The existing visual styling comes from `src/lib/design/chat.css`:

- `.pf-composer textarea`
- `resize: none`
- `min-height: 26px`
- `max-height: 200px`

Because no runtime height is applied, long prompts do not expand the composer
before the max-height scroll behavior is reached.

## Goals

- Grow the main Chat composer textarea as content grows.
- Keep the compact height for empty and short prompts.
- Cap growth at a fixed maximum height and use internal vertical scrolling after
  that point.
- Preserve existing keyboard behavior: Enter sends, Shift+Enter inserts a
  newline, IME composition Enter does not submit.
- Preserve existing draft persistence, per-session draft isolation, attachment
  handling, model controls, permission controls, and submit payloads.
- Keep the change local and minimal.
- Add the required puffer-desktop component update spec for this change.

## Non-Goals

- Do not refactor `ConversationView.svelte` into a new composer component.
- Do not add a shared Svelte action/helper yet.
- Do not modify mini window, Ask Puffer, legacy `ConversationPane`, settings
  textareas, file editor textareas, or workflow textareas.
- Do not use contenteditable.
- Do not depend on browser-native `field-sizing` support.
- Do not add a new package dependency for textarea autosizing.

## Design

Add one local constant in `ConversationView.svelte`:

- `COMPOSER_MAX_HEIGHT_PX`

Set `COMPOSER_MAX_HEIGHT_PX` to `200`, matching the current CSS max-height.
Do not introduce a separate minimum-height constant; the existing CSS
`min-height: 26px` remains the compact lower bound.

Add a local textarea ref:

- `composerTextareaEl: HTMLTextAreaElement | undefined`

Add two local functions:

- `resizeComposerTextarea(textarea = composerTextareaEl)`
- `scheduleComposerResize()`

`resizeComposerTextarea` sets the textarea height to `auto`, reads
`scrollHeight`, clamps the value to `COMPOSER_MAX_HEIGHT_PX`, and writes the
final pixel height back to the element. It also sets the element's vertical
overflow to `hidden` below the cap and `auto` at the cap.

`scheduleComposerResize` waits until the current Svelte DOM update has landed
before measuring. Use `tick()`; do not add `requestAnimationFrame`,
`ResizeObserver`, debounce timers, or hidden mirror elements.

Wire the behavior into the existing textarea:

- bind the textarea with `bind:this={composerTextareaEl}`
- on input, call the existing `updateDraft` path and let that path schedule one
  resize
- when switching sessions, schedule a resize after the restored draft is applied
- when submit clears the draft, schedule a resize
- when a failed submit restores the previous draft, schedule a resize after
  restoration

Do not add a broad `$effect` that reacts to every `draft` change. Autosize
should be called from the few existing code paths that already mutate the
composer draft. This avoids duplicate measurements on normal typing and keeps
the behavior easy to audit.

The submit request shape and message formatting remain unchanged.

Add one concise component update spec at `specs/puffer-desktop/663.md`. It
should document the autosize behavior, the local-only scope, unchanged submit
contracts, and the testing coverage. This is required by repository policy for
desktop component changes.

## CSS Contract

The existing `.pf-composer textarea` rule should continue owning typography and
visual styling. It should keep:

- `resize: none`
- transparent background
- no border
- current font and padding
- current `min-height: 26px`
- current `max-height: 200px`

Add or preserve `overflow-y: hidden` as the base state. Runtime code switches the
textarea to `overflow-y: auto` only when content reaches the max height.

## Performance

The design performs one `scrollHeight` read per input event and for rare
programmatic draft changes. This is acceptable because it touches one textarea
only, not the transcript. No ResizeObserver, MutationObserver, hidden mirror
element, debounce, or shared action is needed.

The implementation should avoid measuring in loops and should not introduce a
global layout observer.

## Edge Cases

- Empty draft: height returns to the existing CSS minimum.
- Short single-line draft: height remains compact.
- Multi-line draft: height grows up to the cap.
- Very long draft: height stays capped and the textarea scrolls internally.
- Session switch: restored draft gets the correct height for that session.
- Submit accepted: cleared draft collapses the composer.
- Submit rejected or failed: restored draft gets the correct height.
- Attachments: preview strip layout stays above the textarea and is not part of
  textarea height measurement.
- Disabled composer: height still represents the current draft value.

## Testing

Add a focused Playwright test to `apps/puffer-desktop/tests/chat-session-ui.spec.ts`.

The test should:

1. Open a chat session with an enabled composer.
2. Capture the initial textarea bounding-box height.
3. Fill a prompt with several newline-separated lines.
4. Assert the textarea height is greater than the initial height.
5. Fill a very long multi-line prompt.
6. Assert the height is capped near the configured max and that
   `scrollHeight > clientHeight`.
7. Submit or clear the draft.
8. Assert the textarea height returns near the initial height.

Add a small assertion that switching away and back to a session restores both
the draft value and a matching grown height, because session restoration is one
of the explicit resize call sites.

Existing IME, draft recovery, attachment, and submit tests should continue to
pass without semantic updates.

## Audit Notes

The rechecked design intentionally removes or rejects these over-designed paths:

- no shared Svelte action until another composer needs the same behavior
- no extracted component for a single textarea
- no hidden mirror element
- no observer-based resizing
- no CSS `field-sizing` dependency
- no agent-sdk feature migration beyond the native textarea measurement pattern
