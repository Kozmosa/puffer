# Image Generation Thumbnail Preview Design

## Summary

Generated `/image` results should render as the same compact image thumbnail UI
used by chat image attachments. The chat surface must not show the generated
file's absolute path, filename, provider, job id, or a generated-artifact card.

This is a narrow UI behavior change. It does not introduce durable assistant
attachments or a generated-media artifact system.

## User Requirement

After image generation succeeds:

- show one image thumbnail in the conversation;
- use the existing attachment image thumbnail and overlay UX;
- show no other generated-image information in the message;
- if the image cannot be read, show an unavailable thumbnail treatment instead
  of a local path.

## Goals

- Reuse the existing attachment preview strip styling and interaction.
- Keep `generate_media` response shape stable, including the existing `path`.
- Convert a successful generated image path into a transient UI preview item.
- Avoid rendering absolute local paths in visible chat content.
- Keep missing-file handling graceful and visually consistent with attachments.

## Non-Goals

- Do not change transcript event schema.
- Do not add `AssistantMessage.attachments`.
- Do not add session-store attachment staging from generated files.
- Do not require `GenerateMediaInput.sessionId` as a new backend contract.
- Do not copy generated files into chat attachment storage.
- Do not persist generated preview thumbnails across refreshes, session switches,
  or daemon restarts in this change.
- Do not add artifact cards, media galleries, file reveal actions, retry actions,
  provider/model/job metadata, or visible filenames.
- Do not add video preview support.
- Do not add thumbnail caching, image resizing, or generated-artifact indexing.

## Architecture

Keep media generation and chat attachments as separate storage concerns.

`generate_media` continues to return the generated output `path`. The frontend
uses that path only as an internal pointer for immediate preview creation. The
path is never rendered as message text.

On successful `/image`, the desktop UI appends a transient assistant timeline
item with empty body text and one image-like attachment preview. That transient
item is for the current in-memory conversation view only. It is not written to
the transcript and does not alter provider conversation replay.

The preview should reuse existing attachment UI components where possible:

- `MessageAttachmentPreviewStrip` if it can accept a direct `previewUrl` without
  requiring persisted chat attachment IDs;
- otherwise `AttachmentPreviewStrip` with a small adapter shape that matches the
  existing `MessageAttachment` display contract.

If the frontend cannot safely read local image bytes from the returned path with
existing APIs, add one small backend read endpoint dedicated to generated media
previews. That endpoint should:

- accept a local path returned by `generate_media`;
- validate that the path is a file and an image;
- return preview bytes and MIME type;
- return an unavailable state when the file is missing or unreadable;
- avoid exposing file contents for non-image paths.

Prefer the smallest endpoint that enables the UI preview. Do not build a
general artifact registry or durable attachment API for this requirement.

## Data Flow

1. The user submits `/image <prompt>`.
2. Desktop calls existing `generateMedia({ sessionId, kind: "image", prompt })`.
3. On success, if `result.path` is present, the frontend loads a preview from
   that path.
4. The frontend appends a transient assistant row with one thumbnail and no body
   text.
5. The row renders through the same image attachment thumbnail UI and opens the
   same attachment overlay.

If `result.path` is null or the file is unavailable, append the same transient
row with an unavailable image thumbnail state. Do not show a path fallback.

## Frontend Behavior

Generated image rows:

- render only the thumbnail;
- suppress empty message text;
- do not show filename, path, job id, provider, model, or status text;
- use the existing attachment click-to-preview interaction;
- reserve the same visual footprint as attachment image thumbnails;
- show the existing unavailable-image treatment when the preview cannot load.

Status messages may still communicate that generation succeeded or failed, but
they must not replace the thumbnail in the conversation and must not expose the
local file path.

## Contract Boundaries

The frontend may keep using `GenerateMediaResult.path` as an internal preview
source. It must not treat `path` as display content.

The transient generated preview is a UI artifact. It is not part of transcript
history, model context reconstruction, session attachment storage, or future
conversation replay.

If later requirements ask for generated previews to survive reloads or appear in
exported transcripts, that should be handled as a separate durable attachment
feature with its own spec.

## Testing

Coverage should verify:

- `/image` success with a readable image path shows one thumbnail in chat;
- the generated image path is not visible in the message body;
- no filename, provider, model, job id, or artifact metadata appears in the
  generated image row;
- clicking the thumbnail opens the same preview overlay used for attachments;
- missing or unreadable generated files show an unavailable thumbnail and no
  path fallback;
- `/video` behavior is unchanged unless video preview is explicitly requested;
- normal uploaded image attachments still render as before.

## Scope Guard

This change should fit in the desktop `/image` success path, preview loading,
and existing attachment preview rendering. If implementation starts requiring
transcript schema edits, session-store attachment migration, daemon event
publishing, or an artifact registry, stop and re-evaluate because that exceeds
the current requirement.
