# Image Generation Assistant Attachment Preview Design

## Summary

Generated images should appear in chat exactly like image attachments: a
thumbnail in the assistant message that opens the existing attachment preview
overlay. The UI must not render the generated image's absolute file path,
filename, provider, job id, or a separate generated-artifact card.

This is a long-term contract change. Backward compatibility with the current
path-as-text behavior is not required.

## Goals

- Reuse the existing attachment thumbnail and overlay UX for generated images.
- Persist generated image previews across refreshes, session switches, and
  daemon restarts.
- Keep the visible chat surface minimal: thumbnail only.
- Require a concrete `sessionId` for image generation requests that should
  appear in chat.
- Avoid a generated-media-specific frontend component unless the shared
  attachment component cannot support the behavior.
- Preserve graceful missing-file handling without exposing local paths.

## Non-Goals

- Do not add artifact cards.
- Do not add visible Open, Reveal, Retry, provider, model, job, filename, or
  path metadata to the chat message.
- Do not redesign media settings.
- Do not add video preview support in this change.
- Do not keep the current absolute-path text fallback.
- Do not add a thumbnail cache or image resizing pipeline in this change.

## Architecture

Use one durable message-attachment pipeline for uploaded images and generated
images.

`TranscriptEvent::AssistantMessage` becomes attachment-capable, matching the
existing `UserMessage` attachment behavior. A generated image result is stored
as a `StoredAttachment` under the session attachment store and referenced by an
assistant transcript event whose text is empty.

The desktop and daemon timeline DTOs expose assistant attachments using the
same `ChatAttachmentDto` shape already used for user attachments. The frontend
renders attachments on both user and assistant messages with
`MessageAttachmentPreviewStrip`.

The generated output path remains internal to media generation and artifact
storage. It is not included in visible message text.

Both daemon surfaces that implement `generate_media` must follow the same
contract: the CLI daemon path in `crates/puffer-cli/src/daemon.rs` and the
Tauri preview backend path in `apps/puffer-desktop/src-tauri/src/backend.rs`.
The desktop frontend should not depend on which backend served the request.

The session store should expose a small helper for staging an attachment from
an existing file path. This avoids reading generated image bytes into frontend
memory or duplicating staging logic in each backend. The helper copies the file
into the session attachment directory and records metadata, using the same
20 MiB attachment cap used by chat attachments to protect preview performance.

## Data Flow

1. The user submits `/image <prompt>`.
2. Desktop calls `generate_media` with required `sessionId`, `kind=image`, and
   `prompt`.
3. The backend runs the existing exact media runtime.
4. On success, the backend stores the generated image bytes as a session
   attachment with kind `image`.
5. The backend appends an assistant transcript event with empty text and the
   stored image attachment.
6. The backend publishes the same session-changed notification used for other
   transcript updates.
7. The frontend refreshes the selected session without showing a full loading
   reset and renders only the thumbnail.

The frontend does not read absolute paths directly. Preview bytes come through
the existing `read_chat_attachment_preview` path.

## Error Handling

Generation errors remain `generate_media` errors and do not create transcript
items.

If `sessionId` is missing, invalid, or unknown, `generate_media` returns an
error before provider execution. This avoids producing an image that cannot be
attached to a chat session.

If generation succeeds but attachment storage fails, `generate_media` returns an
error and no partial assistant message is appended.

If the generated image exceeds the attachment byte limit, `generate_media`
returns an error and does not append a partial assistant message.

If attachment metadata exists but the backing file is later missing, the UI
uses an unavailable image-thumbnail treatment:

- the message still reserves the same compact thumbnail area;
- the message does not fall back to a file card or visible filename;
- no absolute path is shown;
- clicking opens the existing attachment overlay in preview-unavailable state.

## Frontend Behavior

Assistant message rendering supports attachments the same way user message
rendering does.

For generated image messages:

- render the image thumbnail only;
- suppress empty message text;
- do not render filename or metadata next to the thumbnail;
- for missing images, render a same-size unavailable thumbnail placeholder;
- keep click behavior identical to uploaded image attachments.

This keeps generated-image UX visually indistinguishable from attachment image
UX, which is the desired behavior.

## Contract Changes

`GenerateMediaInput.sessionId` becomes required for desktop image generation.
The result no longer needs to expose `path` to the frontend. Job and artifact
metadata may remain in the response for status/debugging, but visible chat
rendering must come from the persisted assistant attachment.

`AssistantMessage` gains an `attachments` field. New transcript events should
write it explicitly. Serde defaults may still be used to keep test fixtures and
existing transcript reads simple, but compatibility fallback is not the user
visible contract.

Timeline normalization must treat user and assistant attachments uniformly. If a
message has no visible text but has attachments, the message still renders as a
valid chat row.

Any code that reconstructs model conversation state from transcripts should
ignore assistant attachments unless a provider-specific future feature requires
image outputs to be replayed into model context. The generated image is a UI
artifact, not a new user prompt.

## Testing

Coverage should verify:

- `generate_media` rejects missing or unknown `sessionId` before provider
  execution;
- assistant transcript events can serialize and deserialize attachments;
- desktop and CLI daemon timeline DTOs expose assistant attachments;
- both CLI daemon and Tauri preview backend `generate_media` paths append an
  assistant message with one image attachment and no path text;
- assistant image attachments render with `MessageAttachmentPreviewStrip`;
- missing backing files render the unavailable preview state without showing
  absolute paths;
- oversized generated images fail without appending a partial chat item;
- `/image ...` still calls `generate_media` and does not call
  `run_agent_turn`.

## Implementation Boundaries

Keep the implementation scoped to transcript attachment support, media result
storage, and shared message rendering.

Do not introduce a new artifact registry or frontend media gallery as part of
this change. If future media features need richer artifact management, they can
build on the same durable attachment metadata rather than replacing this path.
