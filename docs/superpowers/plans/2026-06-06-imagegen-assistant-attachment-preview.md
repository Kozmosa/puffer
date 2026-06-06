# Imagegen Thumbnail Preview Implementation Plan

**Goal:** When `/image` succeeds, show only an image thumbnail in chat using the
same UI/UX as attachment image previews. Do not show the generated file path or
other metadata.

**Architecture:** Keep generated media as an existing media result. Use
`GenerateMediaResult.path` only as an internal preview source, then append a
transient assistant UI row with an image-like attachment preview. Do not change
transcript schema, session-store attachments, or daemon persistence contracts.

**Spec:** `docs/superpowers/specs/2026-06-06-imagegen-assistant-attachment-preview-design.md`

---

## Scope Check

This is a small desktop UI feature, not a durable storage feature.

In scope:

- successful `/image` result preview;
- no visible local path;
- existing attachment thumbnail and overlay UX;
- missing-file thumbnail fallback;
- focused UI/API tests.

Out of scope:

- assistant transcript attachments;
- session-store staging for generated files;
- generated artifact registry;
- persistence across reload/session switch/daemon restart;
- video preview;
- thumbnail cache or image resizing pipeline;
- visible open/reveal/retry/provider/model/job controls.

---

## Expected File Touches

- `apps/puffer-desktop/src/App.svelte`
  - Convert successful `/image` results into transient assistant preview rows.
  - Do not render `result.path` as text.

- `apps/puffer-desktop/src/lib/screens/agent/ConversationView.svelte`
  - Ensure assistant rows can render image preview strips when provided by the
    transient UI item.
  - Suppress empty assistant body text.

- `apps/puffer-desktop/src/lib/screens/agent/MessageAttachmentPreviewStrip.svelte`
  - Reuse as-is if direct `previewUrl` attachments already work.
  - Only adjust if the component unnecessarily requires persisted attachment
    preview loading.

- `apps/puffer-desktop/src/lib/screens/agent/AttachmentPreviewStrip.svelte`
  - Reuse existing unavailable image treatment.
  - Only adjust if generated missing-image previews currently fall back to a
    file card or visible filename.

- `apps/puffer-desktop/src/lib/api/desktop.ts`
  - Keep `GenerateMediaResult.path`.
  - Add a minimal generated-preview read helper only if no existing local file
    preview path is available.

- Optional backend read endpoint:
  - `crates/puffer-cli/src/daemon.rs`
  - `apps/puffer-desktop/src-tauri/src/backend.rs`
  - Add only if the frontend cannot safely load the generated image path.
  - The endpoint should return `{ state, mimeType, bytes }` for image preview
    use and should not create persistent attachments.

- Tests:
  - `apps/puffer-desktop/tests/support/fakeDaemon.ts`
  - `apps/puffer-desktop/tests/chat-session-ui.spec.ts`
  - Add or update focused cases for generated image preview.

---

## Task 1: Inspect Current Preview Capabilities

- [ ] Check whether `MessageAttachmentPreviewStrip` can render a
  `MessageAttachment` that already has `previewUrl`.
- [ ] Check whether `AttachmentOverlay` can open that same attachment without a
  persisted chat attachment id.
- [ ] Check whether desktop code already has a safe file-to-preview helper for
  local image paths.

Decision:

- If direct `previewUrl` works, keep implementation frontend-only except tests.
- If preview bytes must come from the backend, add the smallest read endpoint
  described in Task 2.

---

## Task 2: Add Minimal Preview Loading If Needed

Skip this task if an existing helper can create `previewUrl` from `result.path`.

- [ ] Add `read_generated_media_preview` to the daemon/Tauri desktop API surface.
- [ ] Input: `{ path: string }`.
- [ ] Output: `{ state: "available" | "missing" | "unsupported", mimeType?: string, bytes?: number[] }`.
- [ ] Validate the target is a regular file.
- [ ] Allow only image MIME/extensions needed by generated images, such as PNG,
  JPEG, and WebP.
- [ ] Return `missing` or `unsupported` instead of throwing for normal preview
  failures.
- [ ] Do not write transcript events.
- [ ] Do not copy files into attachment storage.
- [ ] Do not expose filename/path in the response.

Focused tests:

- [ ] readable PNG returns available bytes and `image/png`;
- [ ] missing file returns `missing`;
- [ ] non-image file returns `unsupported`.

---

## Task 3: Build Generated Image Transient Row

- [ ] In `submitMediaSlash`, keep `/video` behavior unchanged.
- [ ] For `/image` success, if `result.path` is present, load the preview.
- [ ] Create a transient assistant timeline item with:
  - `kind: "assistant"`;
  - empty `body`;
  - one image attachment-shaped object;
  - `previewUrl` when bytes are available;
  - `state: "missing"` or equivalent unavailable state when preview load fails.
- [ ] Append this transient item to the current session view.
- [ ] Ensure the message body does not include `result.path`.
- [ ] Keep status text concise and path-free.

Implementation guard:

- Do not alter `GenerateMediaInput.sessionId`.
- Do not alter `GenerateMediaResult.path`.
- Do not append transcript events for the generated image.

---

## Task 4: Reuse Attachment Thumbnail Rendering

- [ ] Render assistant-row attachments through the same preview strip used by
  user message image attachments.
- [ ] Suppress the assistant message text block when body is empty.
- [ ] Confirm the thumbnail size, border, hover behavior, and overlay behavior
  match attachment image previews.
- [ ] Confirm unavailable previews do not show filename or path.

If the existing component cannot support this without broad changes, add a
small adapter near the generated-row code rather than creating a new generated
media card.

---

## Task 5: Add Focused UI Tests

- [ ] Fake daemon returns a successful `/image` result with an image path.
- [ ] Test verifies a thumbnail appears in the chat.
- [ ] Test verifies the absolute path is not visible.
- [ ] Test verifies no job id/provider/model metadata is visible in the row.
- [ ] Test clicks the thumbnail and verifies the attachment preview overlay
  opens.
- [ ] Fake missing-file result verifies an unavailable thumbnail appears and no
  path fallback appears.
- [ ] Regression test normal uploaded image attachments still render.

---

## Task 6: Manual Verification

- [ ] Run the focused desktop UI tests.
- [ ] Run the smallest relevant TypeScript/Svelte checks available for
  `apps/puffer-desktop`.
- [ ] Launch the desktop/web preview if practical and verify:
  - `/image` shows a thumbnail;
  - no local path appears;
  - clicking thumbnail opens preview;
  - missing file shows unavailable thumbnail;
  - `/video` is unchanged.

---

## Stop Conditions

Stop and revisit the spec if implementation appears to require any of these:

- changing `TranscriptEvent`;
- changing session-store attachment storage;
- requiring `sessionId` for media generation as a backend contract;
- writing assistant messages from the daemon;
- adding durable generated artifact records;
- adding a generated-media gallery/card system.

Those are valid future features, but they exceed the current thumbnail-only
requirement.
