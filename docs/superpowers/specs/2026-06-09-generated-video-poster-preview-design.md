# Generated Video Poster Preview Design

Date: 2026-06-09

## Summary

Generated video cards should display a deterministic first-frame poster image.
The poster is extracted once when a video artifact is completed, stored beside
the video artifact, and served through the generated media preview API. Chat
cards render the poster as an image with a play affordance. Video bytes are
requested only when the user opens playback.

This intentionally replaces the previous thumbnail approach that embedded a
small `<video preload="metadata">` in the attachment strip. Browser-native
metadata frame rendering is not deterministic enough for a durable chat UI.

## Context

Milhous session `c6ad17e3-7444-48a5-ba32-0c92bc89788b` contains valid generated
MP4 artifacts:

- `9e1ce118-90bc-481b-a215-b2d904a590b1`
- `0adcf6ba-bf76-4369-a99c-e911b10935a7`

The sidecars identify them as `kind = "video"`, the MP4 files exist, and
`ffprobe` recognizes them as H.264/AAC videos. The missing piece is not video
access. The missing piece is a persisted poster image and a frontend contract
that consumes that poster instead of relying on a tiny video element to draw a
frame.

The earlier generated video playback design deliberately excluded a persisted
poster sidecar. This design supersedes that decision for long-term stability
and performance.

## Goals

- Show a stable first-frame image for generated video cards.
- Extract the poster once per artifact, not once per client render.
- Keep chat scrolling cheap: cards load small image bytes, not MP4 metadata.
- Keep overlay playback on the existing generated video access ticket path.
- Keep the implementation narrow enough to avoid a media pipeline.
- Fail gracefully when poster extraction is unavailable or fails.

## Non-Goals

- No backward-compatible DTO or RPC preservation.
- No multi-size thumbnail set.
- No animated thumbnails, sprite sheets, waveform previews, or timeline hover
  previews.
- No video transcoding.
- No media gallery or global generated media index.
- No background queue or retry service.
- No provider-specific poster APIs in the first version.
- No frontend canvas extraction fallback.
- No attempt to skip black frames or choose a "best" frame.

## Chosen Approach

Persist one poster image per generated video artifact.

When the video runtime completes an artifact, it writes the MP4 as it does
today, extracts the first decodable video frame, scales it to a single bounded
preview size, and writes `poster.jpg` under the same artifact directory:

```text
.puffer/media/artifacts/<artifact_id>/
  byteplus-video-<artifact_id>.mp4
  poster.jpg
```

The artifact sidecar records the poster metadata. If poster extraction fails,
the video artifact remains successful and the sidecar records a missing poster
state.

## Artifact Sidecar

Generated video artifact sidecars gain a `preview` object:

```json
{
  "id": "artifact-id",
  "kind": "video",
  "path": "/.../artifact/video.mp4",
  "mimeType": "video/mp4",
  "preview": {
    "kind": "poster",
    "state": "available",
    "path": "/.../artifact/poster.jpg",
    "mimeType": "image/jpeg",
    "byteCount": 32768,
    "width": 480,
    "height": 270
  }
}
```

Failure shape:

```json
{
  "preview": {
    "kind": "poster",
    "state": "missing",
    "reason": "extraction_failed"
  }
}
```

The `reason` is for diagnostics only. UI copy should not expose it.

Because backward compatibility is not a requirement, generated video previews
should require this sidecar shape after the change. Old video artifacts without
`preview` can appear as fallback file cards.

## Poster Extraction

Extraction should be a small internal helper, not a general media processing
framework.

Rules:

- Input is the just-written canonical video artifact path.
- Output is a JPEG poster in the same artifact directory.
- Use the first decodable frame at timestamp zero.
- Preserve aspect ratio.
- Scale the long edge to a bounded preview size, initially 480 px.
- Use a fixed JPEG quality suitable for UI thumbnails.
- Write atomically: create a temporary poster file, then rename it into place.
- Treat extraction failure as poster missing, not video generation failure.

The decoder implementation can use a single resolved `ffmpeg` executable. The
resolver should stay simple: bundled binary first if available, then `PATH`.
No plugin architecture, provider hook, or user-facing poster settings are
needed.

## Preview API

Replace the image-only generated preview semantics with artifact preview
semantics.

RPC:

```json
{
  "method": "read_generated_artifact_preview",
  "params": {
    "sessionId": "<session id>",
    "artifactId": "<artifact id>"
  }
}
```

Successful response:

```json
{
  "state": "available",
  "mimeType": "image/jpeg",
  "bytes": [255, 216]
}
```

Failure response:

```json
{ "state": "missing" }
```

or:

```json
{ "state": "unsupported" }
```

Behavior:

- Image artifacts return their existing image preview bytes.
- Video artifacts return poster bytes.
- Video artifacts without an available poster return `missing`.
- Non-image, non-video artifacts return `unsupported`.

The preview reader must canonicalize and validate the poster path under:

```text
<session cwd>/.puffer/media/artifacts/<artifact_id>/
```

It must sniff image bytes and reject unsupported poster MIME types.

## Playback API

Keep generated video playback separate from poster preview.

`create_generated_video_access` remains the playback path. It validates the
video artifact and returns a short-lived `/media/generated-video/<ticket>` path.
The frontend requests this URL only when opening the overlay or retrying failed
playback.

The poster preview API never returns video bytes. The playback API never
returns poster bytes.

## Frontend Behavior

`MessageAttachmentPreviewStrip` should treat image and video attachments as
previewable through the same generated artifact preview API.

Video card rendering:

```html
<div class="pf-attachment-video-thumb">
  <img src="blob:poster" alt="Generated video" draggable="false" />
  <span class="pf-attachment-video-play">...</span>
</div>
```

Rules:

- The message strip must not render `<video>` for thumbnail cards.
- The card keeps the same play affordance and click behavior.
- If poster preview is missing or unsupported, render the existing file/video
  fallback card without exposing local paths.
- Opening a video still requests `createGeneratedVideoAccess`.
- Overlay playback continues to render `<video controls autoplay playsinline>`.
- Blob URL cleanup remains required for poster previews.
- Daemon playback URLs are not blob URLs and should not be revoked.

Live `/video` results and reloaded session history should use the same preview
path. There should be no live-only thumbnail behavior.

## Error Handling

- Poster extraction command unavailable: mark poster missing.
- Poster extraction exits non-zero: mark poster missing.
- Poster file missing when preview is requested: return `missing`.
- Poster path escapes the artifact directory: return `unsupported`.
- Poster bytes do not sniff as an allowed image MIME: return `unsupported`.
- Video playback ticket missing or expired: request a fresh playback ticket
  when opening the overlay.

None of these paths should leak absolute local file paths into primary chat UI.

## Performance

- Poster extraction happens once at artifact completion.
- The preview image is bounded to one size, avoiding full-resolution frame
  downloads in chat.
- Chat history loads small poster bytes through the existing preview/object URL
  flow.
- MP4 data is loaded only for overlay playback.
- No background worker, queue, cache invalidation policy, or visibility
  scheduler is needed for the first version.

## Stability And Security

The poster is a trusted derivative of a trusted generated video artifact. It
must still go through path validation before being served.

Security properties:

- artifact id syntax remains restricted;
- sidecar path canonicalization rejects symlink escapes;
- poster path must live under the artifact directory;
- preview response carries image bytes only;
- playback remains ticket-backed;
- daemon auth tokens are never embedded in media URLs.

## Testing

Rust tests:

- completed video artifact records an available poster when extraction
  succeeds;
- poster extraction failure leaves the video artifact successful and records
  missing poster state;
- video preview API returns poster JPEG bytes;
- video preview API returns `missing` when poster state or file is missing;
- video preview API rejects poster symlink escape;
- image preview behavior still returns image bytes;
- playback access still returns video tickets for video artifacts.

Frontend unit tests:

- generated video card uses poster preview bytes;
- generated video card does not render a thumbnail `<video>`;
- missing poster renders fallback without local path text;
- opening a video card requests playback access;
- blob URL cleanup revokes poster object URLs but not daemon playback URLs.

Playwright tests:

- seeded generated video attachment renders an `<img>` thumbnail with a play
  affordance;
- the thumbnail has non-empty image content before click;
- clicking opens overlay playback with `<video controls>`;
- live `/video` success and reloaded session history share the same video card
  behavior.

## Implementation Specs

After implementation, add concise component specs:

- next unused `specs/puffer-core/NN.md` for video poster extraction and sidecar
  metadata;
- next unused `specs/puffer-cli/NN.md` for generated artifact preview RPC;
- next unused `specs/puffer-desktop/NN.md` for poster-backed video cards.

Those component specs should describe final shipped behavior, not this design
discussion.
