# Chat Attachment Persistence

## Goal

Refreshing a desktop chat must preserve uploaded attachment cards. If the
stored attachment file is still present, image cards can open a preview. If the
file is missing, the card still renders from persisted metadata and opens an
unavailable state or presents a disabled preview action.

This design intentionally does not preserve compatibility with the old
placeholder-only transcript shape. Placeholder text such as `[Image: 1.jpg]`
must not be parsed to synthesize cards.

## Current Failure

The composer creates local attachment cards from transient frontend state. The
daemon transcript persists only user message text, so a refreshed timeline can
only return `[Image: name]` or `[File: name]` text. `ConversationView` renders
cards only when `TimelineItem.attachments` is present, so refreshed sessions
fall back to raw placeholder text.

The durable fix is to make attachment metadata and storage part of the session
record rather than a renderer-only presentation overlay.

## Architecture

Attachment lifecycle is owned by the local backend/session store. The frontend
may keep object URLs for immediate pre-submit previews, but those URLs are never
persisted and are not used to rebuild chat history.

The flow is:

1. The user selects or drops files in the composer.
2. The composer creates transient draft previews.
3. On send, the frontend stages each file through a dedicated attachment
   staging API before starting the agent turn.
4. The staging command writes files under the session attachment directory and
   returns stable `AttachmentRef` values.
5. `run_agent_turn` receives the text and staged attachment IDs, not raw file
   bytes or client-supplied metadata.
6. The daemon loads those staged records and persists
   `UserMessage { text, attachments, actor }`.
7. `load_session_detail` returns structured attachments with each user message.
8. The frontend renders cards from `TimelineItem.attachments` after refresh.

The folded placeholder lines remain only a provider prompt fallback while
provider-native attachment ingestion is incomplete. They are not the source of
truth for UI rendering.

## Data Model

Use separate internal and UI-facing attachment shapes. The transcript stores the
minimum data needed to find the file and render a card later:

```rust
StoredAttachment {
    id: String,
    name: String,
    mime_type: String,
    size: u64,
    extension: String,
    kind: StoredAttachmentKind,
    storage_key: String,
}
```

Timeline DTOs expose only display and interaction state:

```ts
type AttachmentRef = {
  id: string;
  name: string;
  mimeType: string;
  size: number;
  extension: string;
  kind: "image" | "file";
  state: "available" | "missing";
};
```

Field meanings:

- `id` is stable within a session and used for UI keys and preview RPCs.
- `name` is display-only and prompt-safe; it is not used for storage lookup.
- `mimeType`, `size`, `extension`, and `kind` drive card rendering and preview
  eligibility.
- `storage_key` is persisted internally so the backend can locate the file. It
  is never returned in timeline DTOs or exposed to provider prompts.
- `state` is not stored in the transcript. It is computed from metadata plus
  file existence when timelines or previews are loaded.

The session transcript event shape becomes:

```rust
UserMessage {
    text: String,
    attachments: Vec<StoredAttachment>,
    actor: Option<MessageActor>,
}
```

Do not introduce a full multipart message system in this change. `text` plus
`attachments` is enough to fix refresh behavior and keeps the implementation
focused. Provider adapters can later map this shape into provider-native text,
image, or file parts without changing the desktop timeline contract.

## Storage

Each session owns its attachments:

```text
<puffer-home>/sessions/<session-id>.session.json
<puffer-home>/sessions/<session-id>.session.jsonl
<puffer-home>/sessions/<session-id>.attachments/
  <attachment-id>/
    original
    metadata.json
```

This sidecar layout preserves the existing session metadata and transcript file
layout. It adds no session-directory migration.

The staging command writes to a temporary file, writes metadata, then atomically
renames into place. This prevents partially staged attachments from appearing
as available.

Deletion policy stays simple:

- Deleting a session deletes its attachment directory.
- No background garbage collector is added in this phase.
- No cross-session deduplication is added in this phase.

This avoids a new storage subsystem while giving attachment files the same
lifetime boundary as the transcript that references them.

## APIs

Add focused desktop APIs:

```ts
stage_chat_attachment({
  sessionId,
  name,
  mimeType,
  extension,
  kind,
  bytes
}) -> AttachmentRef

read_chat_attachment_preview({
  sessionId,
  attachmentId
}) -> AvailablePreview | MissingPreview | UnsupportedPreview
```

`stage_chat_attachment` is a Tauri command, not a daemon WebSocket JSON-RPC
method. The browser `File` bytes cross only the desktop shell IPC boundary and
are written into the session-store attachment sidecar. The daemon WebSocket and
`run_agent_turn` receive only small JSON refs.

Vite/Playwright tests may install a dev-only frontend staging hook that returns
fixture `AttachmentRef` values without writing files. Production builds must
not use that hook and must fail staging when the Tauri shell is unavailable.

Stage files sequentially. The existing 20 MiB per-file and 10-file limits bound
peak memory enough that a chunk protocol is unnecessary in the first
implementation. Do not add base64 file payloads to daemon JSON-RPC or
transcripts.

`run_agent_turn` accepts:

```ts
{
  sessionId: string;
  message: string;
  attachmentIds: string[];
}
```

The daemon validates each ID belongs to the session, loads the corresponding
`StoredAttachment` from the sidecar metadata, and persists those records before
provider execution starts. A crash after turn start still reloads the user's
message and cards.

`load_session_detail` returns user timeline items with `attachments`.
`normalizeTimelineItem` maps those refs into `TimelineItem.attachments`.

Remote-daemon attachment upload is outside this first implementation. If a
session is running against a remote target, the composer should reject local
attachments with a clear status message rather than silently creating
non-durable cards.

## Preview Behavior

Image card preview:

- If `state=available` and the file still exists, the preview API returns a
  byte payload plus MIME type suitable for the existing overlay.
- If the file is missing, the API returns `missing`; the UI renders the card and
  shows the unavailable detail state.
- If the file exists but is not previewable, the API returns `unsupported`; the
  UI renders metadata only.

The timeline `state` is advisory. The preview API re-checks storage on click so
the UI handles files deleted after the transcript was loaded.

The frontend creates an object URL only for the currently open preview overlay
and revokes it when the overlay closes. Timeline items never store preview URLs.

Non-image file cards show metadata and the unavailable or details overlay. This
design does not add download, open-with-system-app, or rich PDF preview.

## Frontend Changes

Composer drafts keep their current local preview behavior before submit.
After submit:

- draft files are staged first;
- the optimistic user row uses returned refs;
- pending localStorage no longer needs to be the durable source of attachment
  presentation;
- refreshed timelines render cards from backend attachments;
- `visibleMessageBody` hides folded placeholder lines only when structured
  attachments are present.

Do not parse `[Image: name]` or `[File: name]` from message text to create
cards. That would hide data loss and cannot recover IDs, MIME type, size,
storage state, or preview capability.

## Backend Changes

Update the session store event model and desktop DTOs to carry stored
attachments. Update daemon `run_agent_turn` and the desktop fallback path to
accept staged attachment IDs.

The storage code should live behind a small session-store attachment module
with these responsibilities:

- allocate attachment IDs;
- validate file count and size limits;
- write staged files atomically;
- persist and load metadata;
- compute availability state;
- read preview bytes for supported image attachments.

This keeps attachment storage independent from provider execution and prevents
desktop UI code from knowing storage paths.

## Security And Performance

Security constraints:

- Never expose `storage_key` or absolute storage paths to the model.
- Sanitize display names before storing and before folding prompt fallback
  lines.
- Enforce attachment limits in the backend even if the frontend already
  checked them.
- Treat MIME type from the browser as a hint; infer or validate when possible.
- Scope preview reads by `sessionId` and `attachmentId`.

Performance constraints:

- Do not base64 encode files into JSON transcripts or turn RPCs.
- Do not load staged file bytes while listing sessions.
- Compute attachment availability from metadata plus existence checks when
  loading a session detail.
- Read preview bytes only on demand.
- Keep object URL creation in the frontend limited to composer drafts and
  currently open previews.

## Testing

Add tests before implementation:

1. Session-store tests stage an attachment, persist a user message with the
   staged record, reload the session, and observe the stored attachment.
2. Daemon/API tests return user timeline attachments with `state=available`.
3. Removing the stored attachment file changes the returned timeline state to
   `missing` and preview reads return the missing response.
4. Playwright tests load a persisted timeline with structured attachments,
   force a full browser reload, and still render attachment cards.
5. Playwright tests verify a structured attachment hides folded `[Image: name]`
   or `[File: name]` fallback lines.
6. Playwright tests verify missing attachments do not open an image preview and
   show the existing unavailable attachment detail.

## Explicit Non-Goals

- No compatibility parser for historical placeholder-only transcripts.
- No cloud sync or cross-device attachment availability.
- No download manager.
- No system-app open action.
- No rich PDF, document, audio, or video preview.
- No cross-session deduplication.
- No background garbage collector beyond deleting attachments with their
  session.
