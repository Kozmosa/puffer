# Chat Inline Image Path Files Design

## Problem

Generated image messages can include local image paths such as
`/Users/zhangxiao/.puffer/media/images/<artifact>/image.jpeg` in assistant
text. When those paths are wrapped in inline code backticks, the chat renderer
displays them as plain `<code>` nodes. They do not become local file links and
therefore cannot switch to the Files pane or reveal the file tree.

The generated image thumbnail flow already works separately: message
attachments open the image overlay. This design keeps that behavior unchanged.

## Decision

Use a single long-term rule:

- Text local file paths open in the Files pane.
- Attachment thumbnails open the attachment overlay.

This keeps path navigation and attachment viewing as separate concepts. It also
avoids image-specific branching in chat text clicks.

## Scope

In scope:

- Inline code that is exactly one local file target becomes a clickable local
  file link while keeping code styling.
- Files pane can preview common image files when opened through a file target.
- Existing attachment thumbnail behavior remains unchanged.
- Tests cover inline-code path navigation and image preview rendering.

Out of scope:

- Opening text image paths directly in the attachment overlay.
- Streaming image reads, download support, or new backend endpoints.
- Autolinking arbitrary substrings inside code snippets or fenced code blocks.
- Special cases for generated media paths beyond normal local file handling.

Backward compatibility is not a design constraint for this change. The target is
a simpler, stable rule set with low runtime cost.

## Components

### MessageBody

`MessageBody.svelte` already autolinks bare local paths in normal text via
`chatFileTarget()` and `fileOpenIntent()`. Extend only the inline-code branch:

- Parse inline code as today.
- If the entire code text is a valid `chatFileTarget()`, render it as a local
  file link with code visual styling.
- Otherwise render it as plain `<code>`.

This avoids scanning code fragments and keeps link detection deterministic.

### Chat Open Intent

No new intent type is needed. Inline-code file links reuse the existing
`{ kind: "file", path, line }` intent. `AgentDetail` continues to switch to the
Files tab for file intents.

### Files Pane Preview

`filePreview.ts` should add an `image` preview format for:

- `.jpg`
- `.jpeg`
- `.png`
- `.webp`
- `.gif`

`FilesPane.svelte` should render image previews from the existing `ReadFileResult`
base64 content using a data URL. It should not introduce a separate image read
path. The current backend hard limit and truncation behavior remain the guardrail.

## Data Flow

1. Assistant text contains `` `/absolute/path/image.jpeg` ``.
2. `MessageBody` parses inline code.
3. The code text is checked with `chatFileTarget()`.
4. If valid, the rendered link emits `fileOpenIntent(path, line)`.
5. `AgentDetail.openChatIntent()` switches to `Files`.
6. `FilesPane` expands parent directories under the current root when possible
   and opens the file.
7. `readFile()` returns base64 for image bytes.
8. `filePreview.ts` classifies the path as `image`.
9. `FilesPane` renders the image preview.

Attachment thumbnails skip this path and continue to emit
`attachmentOpenIntent()`.

## Error Handling

- Invalid inline-code path text remains plain code.
- Missing or disallowed files use the existing Files pane error display from
  `readFile()` or `listDir()`.
- Truncated image reads should not render partial images as successful previews.
  If `ReadFileResult.truncated` is true, Files should show a clear preview error
  instead of a broken image.
- Unsupported image extensions remain binary files unless another preview type
  handles them.

## Performance

The change adds one `chatFileTarget()` check per inline code segment. This is
bounded by message length and avoids broad regex scanning inside code.

Image preview uses the existing capped `readFile()` response. No new watchers,
background preloads, streaming, or caching layers are required.

## Testing

Add focused tests:

- Chat UI: an assistant message containing an inline-code absolute path renders
  that code as a local file link.
- Chat UI: clicking that inline-code link switches to Files and opens the path.
- Files UI: opening a `.jpeg` file backed by base64 content renders an image
  preview instead of the binary placeholder.
- Existing generated image attachment tests continue to pass, proving thumbnail
  overlay behavior is unchanged.

Run:

- `npm run check`
- Targeted Playwright tests for chat session and Files pane coverage.

## Non-Goals

Do not add a generalized markdown parser replacement, URL router abstraction,
media library, download manager, or generated-media-specific path resolver.
The smallest durable abstraction is the existing split between file intents and
attachment intents.
