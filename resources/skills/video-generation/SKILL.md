---
name: video-generation
description: Use when the user asks to create or generate a text-to-video clip through the internal media CLI.
allowed-tools:
  - Bash
user-invocable: true
disable-model-invocation: false
---

Use foreground Bash only. allowed-tools is guidance, not the enforcement boundary; media generation is enforced by the internal tool permission path.

- Run `puffer internal-tool video-generation --prompt ...` for one logical text-to-video request.
- Set an explicit long Bash timeout within the current Bash cap before running the command.
- Treat `--prompt` as literal text unless it names a workspace-relative file; prompt file paths should be passed through `--prompt`.
- Pass `--parameters-json` only for requested scalar overrides: strings, numbers, or booleans.
- Use `--purpose` only when the request supplies a purpose that should be preserved in result metadata.
- This tool is text-to-video only. If the user asks for reference images, first frames, last frames, or image-to-video behavior, state that this tool does not support that input instead of calling it with image references.
- If video generation fails or the media runtime is unavailable, report that plainly.
- Do not imply a video was created unless the tool returns a persisted video artifact.
