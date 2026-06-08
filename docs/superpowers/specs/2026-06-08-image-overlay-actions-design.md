# Image Overlay Actions Design

## Goal

Improve the desktop image preview overlay with one contextual action next to
the close button:

- local image: open the containing folder
- URL image: download the image

The design optimizes for long-term stability, performance, and a small UI
surface. Backward compatibility with the old attachment source shape is out of
scope.

## Current Context

`AttachmentOverlay.svelte` currently renders attachment metadata, a preview
image when `attachment.kind === "image"` and `attachment.previewUrl` exists,
and a close button. The overlay receives `MessageAttachment`, whose source is
currently too coarse to distinguish durable local files from remote URL images.

Generated image previews and persisted chat attachment previews often render
through `blob:` URLs, so the overlay must not infer user actions from
`previewUrl`.

The recheck found four important boundaries:

- browser `File` objects do not reliably expose the user's original disk path,
  so user-uploaded images can only open a containing folder when the backend
  returns a durable stored-file path;
- generated image results already include a local artifact path as
  `GeneratedMediaArtifactResult.path`, so the implementation should reuse that
  field rather than adding path discovery;
- URL images should be explicit `remote_url` attachments. The UI should not
  scrape chat text or inspect preview URLs to invent remote image sources;
- the desktop Tauri DTOs and the `puffer-cli` desktop/daemon DTOs must stay
  aligned because the frontend can talk to either runtime surface.

## Chosen Approach

Use an explicit attachment source model and keep the overlay action as a
single icon-only button immediately left of the close button.

```ts
type AttachmentPreviewSource =
  | { kind: "local_file"; path: string }
  | { kind: "remote_url"; url: string; suggestedName?: string }
  | {
      kind: "generated_media";
      jobId: string;
      artifactId: string;
      index: number;
      localPath?: string;
      remoteSourceUrl?: string;
    };
```

Action resolution is a pure frontend helper:

- `local_file` image attachments return an `open_folder` action.
- `remote_url` image attachments return a `download` action.
- `generated_media` image attachments return `open_folder` only when
  `localPath` is present.
- non-image attachments return no overlay action.

This keeps behavior tied to durable metadata rather than transient preview
URLs.

`local_file.path` must be an absolute path to an existing file known by the
backend. For staged chat attachments, this may be Puffer's stored copy, not the
user's original file picker path. The UI should not promise that it can reveal
the original source folder unless that original path is explicitly stored in a
future change.

`remote_url` attachments should set `previewUrl` to the URL when no blob preview
is available. `readMessageAttachmentPreview` should not fetch remote URLs just
to build thumbnails; the browser image element can load the already-known URL.

Composer attachment drafts are not durable `MessageAttachment`s. They may carry
`File` and blob preview data for staging and optimistic display, but they should
not invent an `AttachmentPreviewSource`. Source metadata belongs to staged,
generated, or otherwise durable message attachments.

## UI/UX

The overlay header stays compact:

- Left side: filename and metadata, with truncation for long names.
- Right side: action group.
- The contextual action button sits immediately left of the close button.
- Local image uses a folder icon with `aria-label="Open image folder"` and
  title `"Open image folder"`.
- URL image uses a download icon with `aria-label="Download image"` and title
  `"Download image"`.
- Close remains the rightmost X and keeps the current focus behavior.

While an action is running, its button is disabled. Failures render as one
short inline status under the header metadata. Successful folder opens do not
leave persistent status. Successful downloads may show a short saved-path
status if the native command returns a path.

There is no overflow menu, progress panel, toast system, or custom save picker.

The existing `Icon.svelte` map already has folder and close icons. Add only the
single missing download icon needed by this control; do not introduce a new
button system.

## Native/API Surface

Add two narrow Tauri commands:

```text
open_image_containing_folder(path)
download_image_from_url(url, suggestedName?)
```

`open_image_containing_folder` validates that `path` is absolute, derives the
parent directory, and opens that directory via `tauri_plugin_opener`.

`download_image_from_url` is async and accepts only `http` and `https`. It uses
a short timeout, enforces a hard byte cap, rejects non-image content types when
available, falls back to a safe extension check when content type is absent or
generic, writes to the user's Downloads directory using a sanitized filename,
and returns the saved path. It writes to a temp file first and then atomically
renames so partial downloads are not presented as final images.

Frontend wrappers live in `desktop.ts`. `AttachmentOverlay.svelte` calls the
pure action resolver and then invokes the corresponding wrapper.

Using a narrow native download command is intentionally scoped: it avoids the
unreliable cross-origin behavior of browser `download` anchors while avoiding a
download manager, progress stream, or custom save dialog.

## Migration

Because backward compatibility is intentionally out of scope, message
attachment creation should emit the new explicit source kinds directly. The
old `user_upload` source should be removed from frontend examples and tests.

Staged chat attachment DTOs should become `local_file` sources pointing at the
stored attachment copy returned by `SessionStore::attachment_original_path`.
Any pre-stage composer helper that currently fabricates `user_upload` should be
changed so draft data is separate from durable message attachment source data.

Generated media should map the existing artifact `path` field into
`source.localPath` when creating frontend attachments and when reconstructing
generated attachments from transcript tool output. If `localPath` is absent,
generated media keeps preview behavior but shows no extra overlay action.

If a generated-media metadata record includes `remoteSourceUrl`, the DTO may
preserve it as `remoteSourceUrl`, but it should not change the primary action
while `localPath` exists. The local file reveal is more predictable for already
downloaded generated artifacts.

## Testing

Unit coverage:

- `imageOverlayAction(local_file image)` returns folder action.
- `imageOverlayAction(remote_url image)` returns download action.
- `imageOverlayAction(generated_media image with localPath)` returns folder
  action.
- `imageOverlayAction(generated_media image without localPath)` returns no
  action.
- non-image attachments return no action.
- `remote_url` normalization sets `previewUrl` to the URL when no preview URL
  is present.

Type/API coverage:

- Update message attachment source examples to the explicit source model.
- Verify `readMessageAttachmentPreview` still routes generated media previews
  by artifact id.
- Verify staged attachments serialize as `local_file` with a stored file path.
- Verify generated media serializes or maps the existing artifact path into
  `localPath` in both desktop Tauri and `puffer-cli` desktop/daemon DTO paths.

UI coverage:

- Local image overlay shows the folder icon immediately left of close.
- URL image overlay shows the download icon immediately left of close.
- Escape and close still close the overlay and restore focus.

Rust coverage:

- Absolute path validation and parent directory derivation.
- URL scheme validation.
- Download filename sanitization.
- Response validation, byte-cap handling, and target-path derivation without
  real network access.

## Out of Scope

- Download progress.
- Download queue or history.
- Custom save location picker.
- Opening the image file itself.
- Multi-action menu.
- Compatibility adapter for old attachment source shapes.
- Recovering the original file picker path for uploaded images.
- Scraping chat text for image URLs.
- Remote URL thumbnail fetching beyond assigning `previewUrl` to an explicit
  `remote_url` attachment.
