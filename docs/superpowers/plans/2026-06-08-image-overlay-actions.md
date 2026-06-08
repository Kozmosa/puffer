# Image Overlay Actions Execution Plan

**Goal:** Image preview overlays show one contextual icon action immediately
left of close: reveal a local image's containing folder, or download an
explicit URL image.

**Spec:** `docs/superpowers/specs/2026-06-08-image-overlay-actions-design.md`

**Scope:** Desktop attachment source typing, overlay UI, narrow Tauri commands,
and focused tests. Avoid broad preview, download, or file-manager systems.

## Scope Check

In scope:

- explicit `local_file`, `remote_url`, and `generated_media` attachment sources;
- folder action for local paths carried by source metadata;
- download action only for explicit `remote_url` image attachments;
- generated-media `localPath` populated from existing artifact paths;
- staged chat attachment DTOs using Puffer's stored attachment copy as
  `local_file`;
- focused frontend, Playwright, and Rust tests.

Out of scope:

- recovering the original file picker path for browser `File` uploads;
- scraping chat body text for image URLs;
- inferring actions from `blob:`, `data:`, or `https:` preview URLs;
- download progress, queue, history, retries, pause/resume, or custom save
  location;
- generic overlay action registries or menus;
- opening the image file itself.

## Expected File Touches

- `apps/puffer-desktop/src/lib/types.ts`
  - Replace `user_upload` with explicit source variants.
  - Extend generated-media source with optional `localPath` and
    `remoteSourceUrl`.

- `apps/puffer-desktop/src/lib/api/desktop.ts`
  - Update backend source typing and normalization.
  - Set `previewUrl` from `remote_url.url` when absent.
  - Keep generated-media preview reads routed by `artifactId`.
  - Add wrappers for the two Tauri commands.

- `apps/puffer-desktop/src/lib/screens/agent/imageOverlayAction.ts`
  - New pure action resolver with unit coverage.

- `apps/puffer-desktop/src/lib/screens/agent/AttachmentOverlay.svelte`
  - Render the icon-only contextual action left of close.
  - Keep close focus/Escape/focus-return behavior.
  - Render compact inline action status.

- `apps/puffer-desktop/src/lib/design/Icon.svelte`
  - Add only the missing `download` lucide icon.

- `apps/puffer-desktop/src/lib/screens/agent/attachments.ts`
  - Stop fabricating durable attachment sources for composer drafts.
  - Keep draft file/blob preview data separate from post-staging
    `MessageAttachment` source data.

- `apps/puffer-desktop/src/App.svelte`
  - Map generated artifact `path` into generated-media `source.localPath`.
  - Keep submit/stage plumbing type-safe after composer drafts stop pretending
    to have durable sources.

- `apps/puffer-desktop/src-tauri/src/dtos.rs`
  - Replace `UserUpload` with `LocalFile { path }`.
  - Include optional `localPath`/`remoteSourceUrl` on generated media.
  - Use `SessionStore::attachment_original_path` for staged attachments.

- `apps/puffer-desktop/src-tauri/src/chat_attachments.rs`
  - Pass enough context to serialize stored attachment paths.

- `apps/puffer-desktop/src-tauri/src/lib.rs`
  - Add `open_image_containing_folder` and `download_image_from_url`.
  - Register both commands.
  - Add small validation helpers and tests.

- `crates/puffer-cli/src/desktop_api_types.rs`
  - Mirror the attachment source DTO changes.

- `crates/puffer-cli/src/desktop_api.rs`
  - Populate `local_file` and generated-media `localPath` consistently with
    the Tauri backend.

- Tests:
  - `apps/puffer-desktop/src/lib/api/desktop.attachment-types.test.ts`
  - New or nearby unit tests for `imageOverlayAction`.
  - `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
  - Existing Rust unit test modules near changed DTO/native helpers.

## Task 1: Tighten Types And Normalization

- [ ] Update `AttachmentPreviewSource` in `types.ts`.
- [ ] Update `BackendChatAttachmentSource` in `desktop.ts`.
- [ ] Update `normalizeMessageAttachment` so `remote_url` image attachments
  receive `previewUrl = url` when the backend did not provide a preview URL.
- [ ] Keep `readMessageAttachmentPreview` behavior narrow:
  - generated media reads by artifact id;
  - all other stored/local attachments read by attachment id;
  - remote URL attachments are not fetched through preview RPCs.
- [ ] Update `desktop.attachment-types.test.ts` examples.

Implementation guard: do not add a compatibility branch for old `user_upload`.
Fix producer sites instead.

## Task 2: Add The Pure Overlay Action Resolver

- [ ] Add `imageOverlayAction.ts`.
- [ ] Return `null` unless `attachment.kind === "image"`.
- [ ] Return `open_folder` for `local_file.path`.
- [ ] Return `download` for `remote_url.url`.
- [ ] Return `open_folder` for `generated_media.localPath`.
- [ ] Add unit tests covering every branch and non-image no-op.

Implementation guard: the resolver must not inspect `previewUrl`.

## Task 3: Update Attachment Producers

- [ ] In `attachments.ts`, remove durable source metadata from
  `ComposerAttachmentDraft`.
- [ ] Adjust `messageAttachmentFromDraft` and submit/stage types so optimistic
  display can still pass `File` and `previewUrl` without inventing a source.
- [ ] Ensure sent/staged messages receive backend-normalized `local_file`
  sources after staging.
- [ ] In `App.svelte`, map generated media result `artifact.path` into
  `source.localPath` for live generated image previews.
- [ ] In `dtos.rs`, serialize stored attachments as
  `local_file { path: attachment_original_path(...) }`.
- [ ] In `backend.rs`, add `localPath` to generated-media source DTOs from
  metadata or the parsed artifact path.
- [ ] Mirror the same DTO and timeline changes in
  `crates/puffer-cli/src/desktop_api_types.rs` and
  `crates/puffer-cli/src/desktop_api.rs`.

Implementation guard: do not persist original upload paths or add schema
migrations. The stored copy path is enough for this feature.

## Task 4: Add Narrow Native Commands

- [ ] Add `open_image_containing_folder(path)`:
  - require an absolute path;
  - canonicalize and require an existing file;
  - open the parent directory with `tauri_plugin_opener`.
- [ ] Add async `download_image_from_url(url, suggestedName?)`:
  - allow only `http` and `https`;
  - use a short timeout;
  - enforce a hard byte cap;
  - accept `image/*` content types;
  - when content type is absent or generic, allow only known image file
    extensions from URL/suggested name;
  - sanitize the final filename;
  - write to a temp file in Downloads and rename to final path;
  - return `{ path }`.
- [ ] Register both commands in the Tauri invoke handler.
- [ ] Add Rust tests for validation helpers without real network.

Implementation guard: no progress events, no dialog, no queue, no retry loop.

## Task 5: Render Overlay Actions

- [ ] Import the resolver and API wrappers in `AttachmentOverlay.svelte`.
- [ ] Add local state for action busy/error/saved path.
- [ ] Render one 30px icon button immediately left of close when a resolver
  action exists.
- [ ] Use `folderOpen` or `folder` for open-folder and the new `download` icon
  for downloads.
- [ ] Disable the action while running.
- [ ] Render a compact inline status below metadata on failure and optionally
  after a successful download.
- [ ] Preserve existing close button initial focus and Escape behavior.

Implementation guard: do not turn the header into a toolbar component unless
the local CSS becomes genuinely duplicated.

## Task 6: Add Focused UI Coverage

- [ ] Add a fake local image attachment with `source.kind === "local_file"` and
  a preview URL; assert the overlay action button labeled "Open image folder"
  appears left of close.
- [ ] Add a fake URL image attachment with `source.kind === "remote_url"` and
  URL preview; assert the overlay action button labeled "Download image" appears
  left of close.
- [ ] Assert Escape closes and focus returns to the thumbnail.
- [ ] Avoid invoking a real OS folder open or real network download in
  Playwright; mock the invoke layer if the test clicks the action.

Implementation guard: do not add visual screenshot tests for this small control.

## Task 7: Verification

- [ ] Run Svelte/type diagnostics:

```bash
npm --prefix apps/puffer-desktop run check
```

- [ ] Run focused attachment tests:

```bash
npm --prefix apps/puffer-desktop run test:desktop -- tests/chat-session-ui.spec.ts -g "attachment|image preview"
```

- [ ] Run focused unit tests:

```bash
npm --prefix apps/puffer-desktop test -- --run src/lib/api/desktop.attachment-types.test.ts
```

- [ ] Run Rust tests for changed desktop/CLI helpers:

```bash
cargo test -p puffer-cli desktop_api
cargo test --manifest-path apps/puffer-desktop/src-tauri/Cargo.toml
```

Adjust commands to the repo's actual test filters if names differ after
implementation.

## Final Review

- [ ] Check `git diff` for accidental download manager, menu, or broad preview
  changes.
- [ ] Confirm no source action is inferred from `previewUrl`.
- [ ] Confirm generated media still previews through artifact id.
- [ ] Confirm staged attachments still upload and render after source migration.
- [ ] Confirm remote URL attachments do not trigger preview RPC reads.
- [ ] Confirm no original file picker path is promised or stored.
- [ ] Update component specs under `specs/<component>/` only if implementation
  changes component contracts beyond this plan.

## Stop Conditions

Stop and revisit the spec if implementation appears to require:

- a general download manager;
- a custom save-location picker;
- browser URL scraping;
- persisted original upload paths;
- broad session-store schema migration;
- replacing the overlay component;
- generalized action registries or menus;
- real-network Playwright coverage.
