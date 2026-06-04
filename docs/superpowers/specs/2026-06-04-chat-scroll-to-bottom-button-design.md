# Chat Scroll-To-Bottom Button Design

Date: 2026-06-04

## Goal

Add a chat-thread down-arrow affordance that appears only when the transcript
is scrollable and the user is not near the latest message. Clicking it scrolls
the current chat to the bottom and hides the affordance immediately.

The design follows the `agentenv-monorepo` agent-sdk `MessageList` pattern:
the button represents transcript scroll state, floats above the composer, and
uses the same near-bottom threshold as auto-scroll behavior.

## Non-Goals

- No backend, session, or transcript model changes.
- No new generic scroll framework.
- No compatibility layer for older UI behavior.
- No unread-count, new-message badge, or multi-state notification UI.

## Reference Behavior

The agent-sdk implementation keeps the scroll affordance inside a relative
message-list container:

- It uses a 100 px near-bottom threshold.
- It hides the button when there is no meaningful overflow.
- It shows the button when the user scrolls away from the bottom.
- It hides the button immediately on click, then smooth-scrolls to the bottom.
- It keeps auto-follow behavior separate from explicit user scrolling.

Puffer already has compatible primitives in `ConversationView.svelte`:

- `THREAD_BOTTOM_THRESHOLD_PX = 100`
- `isThreadNearBottom()`
- `scrollThreadToBottom()`
- composer autosize anchoring based on near-bottom state

The Puffer implementation should reuse these primitives instead of duplicating
the agent-sdk state machine.

## UX

Placement: overlay above the composer, centered horizontally within the chat
surface.

Visibility rules:

- Hidden while the thread has no meaningful overflow.
- Hidden when the thread is within `THREAD_BOTTOM_THRESHOLD_PX` of the bottom.
- Visible when the user scrolls above that threshold.
- Hidden immediately when clicked.
- Hidden when switching sessions until the new thread state is measured.

Interaction:

- Button label: `Scroll to bottom`.
- Click calls `scrollThreadToBottom("smooth")`.
- The next scroll event recomputes visibility from the actual scroll position.

Visual styling:

- Circular icon button, approximately 30 px.
- Use the existing shared `Icon` component with a down chevron icon if
  available; otherwise add the minimal icon needed to the shared icon map.
- Match Puffer's quiet utility UI: border, background, small shadow, hover and
  focus-visible states.
- Do not overlap message content or composer controls.

## Architecture

Keep the feature inside `ConversationView.svelte` because it is tightly coupled
to the existing `threadEl`, composer anchoring, and transcript scroll behavior.

Add one state value:

- `showScrollToBottom: boolean`

Add one measurement helper:

- `refreshThreadScrollButtonState(thread = threadEl): void`

The helper should compute:

- meaningful overflow: `scrollHeight - clientHeight > THREAD_BOTTOM_THRESHOLD_PX`
- near bottom: current `isThreadNearBottom(thread)`

It should set `showScrollToBottom` to true only when there is meaningful
overflow and the thread is not near bottom.

Wire the helper to:

- `onscroll` on `.pf-chat-thread`
- post-render checks after initial load, session changes, timeline changes, and
  composer resizing
- the scroll-to-bottom button click handler

Avoid a separate component unless the scroll logic grows beyond this single
button. The goal is to keep the state close to the scroll container without
introducing an abstraction that has only one caller.

## Data Flow

1. The thread renders or updates.
2. A post-render measurement updates `showScrollToBottom`.
3. User scrolls the transcript.
4. `onscroll` recomputes whether the button should show.
5. User clicks the button.
6. The button hides immediately and the thread smooth-scrolls to the bottom.
7. The next scroll event confirms the hidden state from the actual position.

## Edge Cases

- Empty or short conversations never show the button.
- Initial session load should not flash the button before bottom scroll settles.
- Session switching should reset stale visibility from the previous thread.
- Composer autosize should preserve existing bottom anchoring and then refresh
  the button state.
- Streaming or transcript reload should keep auto-follow behavior for users
  already near the bottom, and show the button for users who intentionally
  scrolled up.

## Testing

Extend `apps/puffer-desktop/tests/chat-session-ui.spec.ts` with focused UI
coverage:

- Button appears after a scrollable conversation is scrolled away from bottom.
- Clicking the button scrolls to the bottom and hides it.
- Button stays hidden for conversations without meaningful overflow.
- Existing composer autosize bottom anchoring remains covered by the current
  test and should continue to pass.

Run:

- `git diff --check`
- `npm --prefix apps/puffer-desktop run check`
- Targeted Playwright test(s) for the new scroll button behavior

## Implementation Notes

This is intentionally smaller than a direct agent-sdk port. The agent-sdk state
machine handles broader embed scenarios, initial scroll modes, floating pills,
and reusable package boundaries. Puffer's chat view already owns the scroll
container and bottom anchoring, so a single local state value plus one
measurement helper is the long-term maintainable design.
