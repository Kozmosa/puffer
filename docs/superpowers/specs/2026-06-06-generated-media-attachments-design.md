# Generated Media Attachments Design

- Date: 2026-06-06
- Status: Approved design, pending implementation
- Scope: Durable preview model for `ImageGeneration` results in desktop chat

## Summary

Generated images should be represented as assistant-generated attachments in
the conversation timeline. The chat surface shows compact image thumbnails and
opens the existing attachment overlay for large-image preview. The
`ImageGeneration` tool card remains available in agent activity for execution
details, but it is not the primary media display surface.

This design intentionally does not preserve older DTO behavior. The target
schema should model generated media explicitly instead of rebuilding previews
from local path text.

## Context

The current generated-image preview path is split:

- `/image` slash generation can append a transient live assistant item with an
  image-like attachment preview.
- Persisted `ImageGeneration` tool invocations load as plain tool calls, so
  historical sessions do not regain thumbnails after reload.
- Generated media preview reads are path-based and validated against the
  current daemon workspace root, which breaks sessions whose cwd differs from
  the daemon root.
- Assistant text may mention local image paths, but those paths are not a safe
  or stable preview source.

Mainstream agent chat products treat generated images as conversation outputs
or artifacts, not as raw tool log text. Puffer should follow that model while
keeping the implementation narrow.

## Goals

- Show generated images as assistant-side thumbnails in chat history.
- Reuse the existing attachment thumbnail strip and attachment overlay.
- Restore previews when loading persisted sessions.
- Keep `ImageGeneration` tool cards focused on prompt, provider, model, path,
  status, and diagnostics.
- Resolve generated media by trusted artifact identity and session context, not
  by arbitrary transcript text.
- Keep initial timeline loading lightweight by returning metadata before bytes.
- Make preview reads stable across sessions with different cwd values.

## Non-Goals

- Do not scan assistant text, markdown, inline code, or arbitrary tool output
  for local image paths.
- Do not turn the feature into a general local file previewer.
- Do not let the frontend use absolute filesystem paths directly as image URLs.
- Do not add a media gallery, export workflow, retry UI, file reveal UI, or
  provider-specific image viewer.
- Do not add video preview support in this change.
- Do not preserve legacy DTO shapes or add compatibility fallbacks for old
  frontends.
- Do not introduce a cross-session media search index.

## Recommended Architecture

Add generated media as a first-class attachment source in the desktop timeline
contract.

`AssistantMessage` should be able to carry attachments, matching the way
`UserMessage` already carries attachments. A generated media attachment should
use the existing frontend `MessageAttachment` display contract while adding
backend-only provenance fields where needed, such as artifact id and media
source.

The backend should synthesize these attachments from structured
`ImageGeneration` results and media sidecars:

1. Parse successful `TranscriptEvent::ToolInvocation` events whose `tool_id` is
   `ImageGeneration`.
2. Parse the tool output JSON for `artifactId`, `jobId`, `path`, and status.
3. Prefer the artifact sidecar for canonical path, MIME type, byte count, and
   metadata.
4. Attach the generated image to the nearest following assistant message in the
   same visible turn.
5. If there is no following assistant message, emit a small assistant timeline
   item with empty body and the generated attachment.

This keeps generated images visible in the conversation, while the tool
invocation remains in agent activity for debugging.

## Timeline Contract

Target DTO behavior:

- `UserMessage.attachments`: uploaded or staged user attachments.
- `AssistantMessage.attachments`: assistant-generated media attachments.
- `ToolCall`: no attachments by default; remains an execution-log item.

Generated image attachment fields should include:

- `id`: stable generated id, preferably `generated-image:<artifactId>`.
- `name`: display name, for example `Generated image`.
- `mimeType`: canonical MIME from sidecar or byte sniffing.
- `size`: byte count from sidecar or filesystem metadata.
- `extension`: derived from canonical MIME.
- `kind`: `image`.
- `state`: `available` or `missing`.
- backend provenance: artifact id, session id, and source type if needed for
  preview reads.

The frontend should not need to parse `ImageGeneration` JSON to display a
persisted generated image. It should receive normal timeline attachments and
render them through existing components.

## Preview Read Contract

Replace the path-only generated preview read with a session-aware generated
media preview request. The request should identify media by session id and
artifact id, not only by absolute path.

Recommended request shape:

```json
{
  "sessionId": "<session uuid>",
  "artifactId": "<media artifact uuid>"
}
```

The backend resolves this request by:

1. Loading the target session metadata.
2. Loading the media artifact sidecar by artifact id.
3. Validating that the artifact belongs to generated media and is an image.
4. Validating that the canonical file path is under the session's generated
   media root or trusted media artifact root.
5. Returning bytes and canonical MIME type, or `missing` / `unsupported`.

Absolute path reads may remain as an internal helper, but the public desktop
RPC should not depend on the daemon workspace root.

## MIME and Path Rules

MIME should not be inferred from filename extension alone. Use this precedence:

1. Artifact sidecar MIME when present and valid.
2. Byte sniffing for PNG, JPEG, and WebP.
3. Extension only as a final display fallback.

Path validation should be strict:

- Accept only files associated with a media artifact sidecar.
- Reject paths outside the session media root or trusted artifact root.
- Reject symlink escapes after canonicalization.
- Return `missing` for known artifact files that no longer exist.
- Return `unsupported` for non-image artifacts or invalid provenance.

This solves `.png` paths containing JPEG bytes without expanding the feature
into arbitrary local file access.

## Frontend Behavior

The chat UI should render assistant-generated attachments exactly like message
attachments:

- show one thumbnail per generated image;
- open the existing `AttachmentOverlay` on click;
- hide local paths from the primary message body;
- show the existing unavailable-image treatment when preview bytes cannot be
  read;
- keep object URL creation and revocation within the existing attachment
  preview lifecycle.

The `ImageGeneration` tool card should remain a standard activity item. It may
display structured execution details, but it should not be the only place where
the generated image is visible.

## Data Flow

```text
ImageGeneration tool succeeds
  -> tool output includes artifactId/jobId/path/status
  -> media artifact sidecar stores canonical image metadata
  -> session timeline loader synthesizes assistant attachment metadata
  -> frontend normalizes attachment through existing MessageAttachment shape
  -> thumbnail component lazily requests preview bytes by sessionId + artifactId
  -> attachment overlay displays the same preview object URL
```

The transcript remains the source of event ordering. The media sidecar remains
the source of file identity and image metadata.

## Performance

- Do not include image bytes in timeline responses.
- Return only attachment metadata during session load.
- Lazy-load thumbnails when the message attachment strip mounts.
- Cache preview bytes or object URLs by `sessionId + artifactId` in the
  frontend for the lifetime of the loaded session view.
- Revoke object URLs on session switches, timeline replacement, and overlay
  close, following the existing attachment cleanup pattern.
- Avoid scanning the filesystem during timeline load. Read only sidecars
  referenced by `ImageGeneration` tool outputs.

This keeps history loading proportional to transcript size and referenced
artifacts, not to media directory size.

## Error Handling

- Successful tool output with readable image artifact: show thumbnail.
- Successful tool output with missing file: show unavailable image placeholder.
- Missing or malformed sidecar but existing structured path: return missing or
  unsupported instead of exposing the path.
- MIME mismatch: use sidecar or byte-sniffed MIME for display and blob
  creation.
- Unsupported artifact: omit preview bytes and keep the tool log available.
- Backend preview read failure: frontend keeps the attachment row with the
  unavailable state and does not render path fallback text.

## Testing

Backend coverage:

- timeline loader attaches generated image metadata to assistant messages;
- multiple `ImageGeneration` calls before one assistant message attach multiple
  images in order;
- generated images without a following assistant message produce a minimal
  assistant attachment item;
- malformed tool output does not panic and leaves the tool call visible;
- generated preview reads resolve by `sessionId + artifactId`;
- preview reads work when session cwd differs from daemon workspace root;
- symlink escape and non-image artifact reads return `unsupported`;
- missing artifact files return `missing`;
- MIME mismatch uses sidecar or byte sniffing instead of extension alone.

Frontend coverage:

- persisted assistant-generated attachments show thumbnails after session load;
- clicking a generated thumbnail opens the existing attachment overlay;
- local image paths are not visible in primary chat content;
- missing previews show the unavailable thumbnail treatment;
- normal user image attachments still render and open unchanged;
- `ImageGeneration` remains available in agent activity as a tool card.

## Scope Guard

The implementation should stay within:

- desktop timeline DTOs and normalization;
- session timeline synthesis from `ImageGeneration` tool output;
- generated media preview RPC;
- existing attachment thumbnail and overlay components;
- targeted tests for those paths.

Do not add a media gallery, full artifact browser, arbitrary path previewing,
or cross-session index. If implementation starts requiring any of those, split
that into a separate design.

## Long-Term Benefit

This design gives generated media the same user-facing behavior as attachments
without coupling the chat UI to tool output JSON or local absolute paths. It
keeps the trusted boundary small, supports persisted sessions, and avoids
loading image bytes until the UI actually needs them.
