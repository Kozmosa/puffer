# Attachment Overlay: Generic "Open Containing Folder"

Date: 2026-06-10
Branch: feat/chat-image-generation

## Problem

When a user clicks an attachment in a chat message, `AttachmentOverlay.svelte`
opens a modal preview. The overlay's primary action button can "Open containing
folder", but today that affordance is gated to image and video attachments only.
A plain **file** attachment (PDF, text, zip, etc.) that exists on disk shows the
overlay with no way to reveal its folder — even though the path is known.

The gate lives in `imageOverlayAction.ts`:

```ts
if (!attachment || (attachment.kind !== "image" && attachment.kind !== "video")) return null;
```

Goal: any attachment that exists on disk — image, video, or file — can open the
folder it lives in from the overlay.

## Architecture context

There is exactly **one** overlay and **one** open-folder path:

```
thumbnail click
  -> AttachmentOverlay.svelte
       -> imageOverlayAction(attachment)            // decides the action
       -> openImageContainingFolder(path)           // desktop.ts bridge
       -> invoke("open_image_containing_folder")     // Tauri command
       -> image_actions.rs::open_image_containing_folder
            -> resolve_image_containing_folder       // validate + parent dir
            -> tauri_plugin_opener open_path
```

Image-generation and video-generation results are **not** a separate overlay.
They render through this same `AttachmentOverlay` as `generated_media`
attachments. Therefore "must not break image/video-gen open-folder" is satisfied
by preserving the `generated_media` branch of the shared logic — which this
design does.

## Decisions

- **No backward compatibility.** Optimize for long-term clarity over preserving
  existing names.
- **Full generic rename.** The shared logic is named for images but serves
  video, generated media, and now files. Rename to attachment-neutral names.
- **No false affordances.** Open-folder appears only when a real on-disk path
  exists. A file with no local path shows no action button (Close only). No
  generic file-download is added.

## Behavior after change

| kind | source | action |
|---|---|---|
| image / video / **file** | `local_file` (has `path`) | **Open containing folder** |
| image / video / **file** | `generated_media` (has `localPath`) | **Open containing folder** *(image-gen / video-gen, unchanged)* |
| image | `remote_url` | Download image *(unchanged)* |
| file / video | `remote_url` (no local path) | none |
| any | source missing path/localPath | none |

The sole functional change: a `file`-kind attachment with a local path now
yields `open_folder` instead of being rejected by the kind gate.

## Changes by file

### 1. `imageOverlayAction.ts` -> `attachmentOverlayAction.ts`

Rename the file, the exported function (`imageOverlayAction` ->
`attachmentOverlayAction`), and the type (`ImageOverlayAction` ->
`AttachmentOverlayAction`).

Remove the `kind`-based early return. Branch only on `source.kind`:

```ts
export type AttachmentOverlayAction =
  | { kind: "open_folder"; path: string }
  | { kind: "download"; url: string; suggestedName: string };

export function attachmentOverlayAction(
  attachment: MessageAttachment | null
): AttachmentOverlayAction | null {
  if (!attachment) return null;

  switch (attachment.source.kind) {
    case "local_file":
      return attachment.source.path
        ? { kind: "open_folder", path: attachment.source.path }
        : null;
    case "generated_media":
      return attachment.source.localPath
        ? { kind: "open_folder", path: attachment.source.localPath }
        : null;
    case "remote_url":
      return attachment.kind === "image" && attachment.source.url
        ? { kind: "download", url: attachment.source.url, suggestedName: attachment.name }
        : null;
  }
}
```

Note: `remote_url` keeps the image-only download. Remote videos and remote files
still return `null` (no local folder to open), matching today's video behavior
and the no-false-affordance decision.

### 2. `AttachmentOverlay.svelte`

- Import the renamed function/type; call the renamed `openContainingFolder`.
- Generic label:
  - `open_folder` -> `"Open containing folder"`
  - `download` -> `"Download image"`
  - (Removes the image/video-specific label branch.)
- Icon mapping, busy/error/reset state, focus/escape handling: unchanged.

### 3. `desktop.ts`

Rename `openImageContainingFolder` -> `openContainingFolder`, invoking
`"open_containing_folder"`. Error string becomes shell-generic
("Opening a folder requires the Tauri desktop shell.").

### 4. `image_actions.rs`

- `open_image_containing_folder` -> `open_containing_folder`
- `resolve_image_containing_folder` -> `resolve_containing_folder`
- Error strings drop "image": `"path must be absolute"`,
  `"path must be an existing file"`, `"path has no containing folder"`.
- **Validation logic unchanged**: absolute path + existing file -> canonicalize
  -> parent directory -> `tauri_plugin_opener`. Already content-agnostic, so it
  is safe for files without modification.
- `download_image_from_url` and its helpers remain image-specific and stay in
  this module unchanged. The module keeps the name `image_actions.rs` (no module
  split — that would be over-engineering for one generic function).

### 5. `lib.rs`

Update the command name in **both** places — they must change together:
- the `REGISTERED_TAURI_COMMANDS: &[&str]` allowlist string literal
  (currently line 56). This is a runtime gate; if the handler is renamed but
  this string is not, the command silently fails the gate.
- the `invoke_handler` registration `image_actions::open_image_containing_folder`
  (currently line 525).

Do **not** touch `open_image_dir` (lib.rs:431) — see Out of scope.

## Tests (TDD — written or updated before implementation)

### Frontend `attachmentOverlayAction.test.ts`
- Keep: local image -> open_folder; remote image -> download; generated
  image -> open_folder; generated video -> open_folder; generated media without
  local path -> null.
- **Change** the old "non-image returns null" case: a `local_file` PDF now
  returns `open_folder` with its path.
- **Add**: a `remote_url` PDF (no local path) returns `null`.

### Rust `image_actions.rs` tests
- Rename references to `resolve_containing_folder`.
- **Add**: a non-image file (e.g. `report.pdf`) resolves to its parent folder —
  proving the resolver is content-agnostic. Absolute/existing-file negative
  cases preserved.

## Out of scope (YAGNI)

- No generic file download for non-image attachments.
- No new Rust module split.
- No Tauri permission/capability changes. There are no
  `capabilities/`/`permissions/` entries for this command; the allowlist in
  `lib.rs` is the only gate.
- No change to the thumbnail-click -> overlay flow or the attachment data model.
- **Leave `open_image_dir` untouched.** It is a separate command (lib.rs:431,
  called from `MediaSettingsModal.svelte`) that opens the configured media output
  directory for a session `cwd` — unrelated to the per-attachment overlay despite
  the similar name. Renaming it is scope creep.

## Affected reference sites (rename checklist)

Verified the complete set of identifier references to update:

- `imageOverlayAction` / `ImageOverlayAction`: `imageOverlayAction.ts` (def),
  `AttachmentOverlay.svelte` (import + `overlayAction` + `overlayActionKey`
  signature), `imageOverlayAction.test.ts`.
- `openImageContainingFolder` -> `openContainingFolder`: `desktop.ts` (def),
  `AttachmentOverlay.svelte` (import + call).
- `open_image_containing_folder` -> `open_containing_folder`: `desktop.ts`
  invoke string, `lib.rs:56` (allowlist), `lib.rs:525` (handler),
  `image_actions.rs` (def).
- `resolve_image_containing_folder` -> `resolve_containing_folder`:
  `image_actions.rs` (def + 4 test references).
