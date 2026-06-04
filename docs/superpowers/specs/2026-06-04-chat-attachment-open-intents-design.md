# Chat Attachment Open Intents

## Goal

Unify click behavior for chat-visible uploaded attachments, local file paths in
message bodies, and file references shown by tool output.

The desktop chat should treat these as one family of "openable chat objects":

- uploaded image and file attachments rendered in user messages;
- local file paths and `file://` links rendered in message text;
- file references surfaced by tool cards.

The implementation should improve long-term clarity and stability without
introducing a large attachment storage rewrite.

## Scope

This design covers the desktop frontend interaction layer in
`apps/puffer-desktop`.

Included:

- message attachment click handling;
- existing message-body local path clicks;
- existing tool-card file reference clicks;
- a lightweight image preview overlay for message attachments with a live
  `previewUrl`;
- an attachment detail state for attachments that cannot be previewed;
- one shared open-intent route owned by the agent detail screen.

Excluded:

- Rust daemon changes;
- `run_agent_turn` protocol changes;
- persistent transcript schema changes;
- writing uploaded attachment files to disk;
- download actions;
- context menus;
- persisted object URLs or frontend attachment caches;
- composer draft attachment previews beyond the existing remove behavior.

## Current State

`ConversationView.svelte` currently renders submitted user-message attachments
through `AttachmentPreviewStrip.svelte`. Image attachments can include a
browser object URL while the optimistic message is alive. Persisted/reloaded
messages keep only attachment metadata.

`MessageBody.svelte` already detects local paths and calls `onOpenFile`.
`ToolCard.svelte` also accepts an `onOpenFile` callback for file references.
These are useful behaviors, but the open routing is split by component surface.

`App.svelte` deliberately strips `previewUrl` before storing pending submitted
messages in `localStorage`. This is correct and should remain unchanged:
object URLs are process-local browser resources, not durable transcript data.

## Architecture

Introduce one small frontend type for chat-object open requests:

```ts
type ChatOpenIntent =
  | { kind: "file"; path: string; line?: number | null }
  | { kind: "attachment"; attachment: MessageAttachment }
  | { kind: "reference"; path: string; line?: number | null; source: "message" | "tool" };
```

The exact file for the type can be a small agent-screen helper module, such as
`chatOpenIntent.ts`, if implementation would otherwise widen existing modules.

Component responsibilities:

- `AttachmentPreviewStrip.svelte` renders attachment cards and thumbnails. In
  message mode, clicking an item emits an attachment intent. In composer mode,
  it keeps the current remove-focused behavior.
- `MessageBody.svelte` keeps local path detection but emits a unified file or
  reference intent instead of owning destination semantics.
- `ToolCard.svelte` emits the same file/reference intent for file targets.
- `ConversationView.svelte` passes intents upward and does not decide which
  panel opens.
- `AgentDetail.svelte` is the single routing owner. It maps file/reference
  intents to the Files tab and maps attachment intents to preview/detail UI.

This keeps chat rendering components dumb and makes future additions, such as
PDF preview or a context menu, route through one place.

## Interaction Rules

Message attachments:

- Image attachment with `previewUrl`: click opens a lightweight overlay with
  the full image, file name, and size.
- Image attachment without `previewUrl`: click opens attachment detail with a
  clear unavailable state.
- Non-image attachment: click opens attachment detail. It must not invent a
  path or switch to the Files tab unless a future model provides a real path.

Local file paths:

- Absolute paths and valid `file://` links in messages open the Files tab.
- Line numbers, when present, are preserved.

Tool file references:

- Tool-card file targets open the Files tab through the same route as message
  file paths.

Composer attachments:

- Draft attachments remain remove-oriented.
- Clicking a composer draft attachment must not open preview in this change.
  This avoids conflict with the remove button and keeps this design scoped to
  chat-visible content.

Unavailable targets:

- Failed or unsupported attachment opens show explicit text such as "Preview
  unavailable for this attachment."
- The UI must not silently ignore a valid click.

## Data Flow

Upload and send:

1. `attachments.ts` creates `ComposerAttachmentDraft` values from
   `FileList | File[]`.
2. Image drafts get a `previewUrl` from `URL.createObjectURL(file)`.
3. Submit builds:
   - `attachments`: daemon-facing metadata;
   - `displayAttachments`: optimistic frontend display data with `previewUrl`
     when available.
4. The daemon receives only metadata and the formatted message text.

Render and open:

1. `ConversationView.svelte` renders message attachments with
   `AttachmentPreviewStrip`.
2. Clicking a message attachment emits an attachment intent.
3. Clicking a local path or tool target emits a file/reference intent.
4. `AgentDetail.svelte` routes:
   - file/reference -> update `fileToOpen` and switch to `files`;
   - image attachment with `previewUrl` -> open image overlay;
   - all other attachments -> show attachment detail.

Lifecycle:

- The image overlay uses the existing object URL; it does not create a new one.
- Closing the overlay does not revoke the URL.
- Existing message cleanup remains responsible for
  `revokeTimelineAttachmentPreviews`.
- Pending submitted messages stored in `localStorage` continue stripping
  `previewUrl`.
- Reloaded transcripts may show attachment details but cannot recover image
  previews unless a later persistent attachment model is added.

## Error Handling

Attachment preview is allowed only when the attachment has a usable
`previewUrl`. Missing preview data is not an exception; it is a normal detail
state.

Files-pane open failures stay in the existing Files tab error path. The open
intent should not pre-read files, check daemon access, or duplicate FilesPane
error handling.

If an intent is malformed, the route should fail closed with an explanatory
detail state instead of throwing during render.

## Performance

The design avoids work proportional to transcript size beyond normal rendering.

Do not:

- read attachment file contents during render;
- add global attachment caches;
- persist or clone blob URLs;
- create preview object URLs on click;
- add observers for this behavior;
- generate previews for non-image attachments.

The only heavier UI is the image overlay, and it is rendered on demand.

## Accessibility

Clickable attachment cards should be real buttons or equivalent keyboard
targets with clear labels:

- image: `Open image attachment <name>`;
- file: `Open attachment details for <name>`.

The image overlay should support:

- close button;
- `Esc` close;
- focus containment or at least focus restoration to the clicked attachment;
- useful image alt text from the attachment name.

Attachment detail state should be readable by screen readers and should not
depend only on color.

## Testing

Add focused desktop UI coverage in `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
or a similarly scoped agent chat spec.

Cases:

1. Message image attachment with `previewUrl` opens an overlay with the image
   and file name; `Esc` closes it.
2. Message image attachment without `previewUrl` opens an unavailable detail
   state.
3. Message non-image attachment opens attachment detail and does not switch to
   the Files tab.
4. Message body local path still opens the Files tab and preserves line number.
5. Tool-card file reference opens through the same Files tab path.
6. Composer draft attachment remove behavior remains unchanged.

The tests should not depend on native Tauri path-drop behavior.

## Long-Term Follow-Up

A later design can introduce durable attachment resources if Puffer needs
reloaded transcripts to preview uploaded files. That should be separate because
it affects daemon storage, session persistence, security boundaries, and
possibly upload limits.

This design intentionally stops at a stable frontend intent boundary.
