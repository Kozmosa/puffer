import { describe, it, expect } from "vitest";
import { videoMimeType, isPlayableMediaPath, previewFormat } from "./filePreview";

describe("videoMimeType", () => {
  it("maps supported video extensions case-insensitively", () => {
    expect(videoMimeType("/a/clip.mp4")).toBe("video/mp4");
    expect(videoMimeType("/a/clip.m4v")).toBe("video/mp4");
    expect(videoMimeType("/a/clip.webm")).toBe("video/webm");
    expect(videoMimeType("/a/clip.ogv")).toBe("video/ogg");
    expect(videoMimeType("/a/clip.MOV")).toBe("video/quicktime");
  });

  it("returns null for audio/other files", () => {
    expect(videoMimeType("/a/song.ogg")).toBeNull();
    expect(videoMimeType("/a/photo.png")).toBeNull();
    expect(videoMimeType("/a/readme.md")).toBeNull();
  });
});

describe("isPlayableMediaPath", () => {
  it("is true only for playable video paths", () => {
    expect(isPlayableMediaPath("/a/clip.mp4")).toBe(true);
    expect(isPlayableMediaPath("/a/notes.txt")).toBe(false);
  });
});

describe("previewFormat", () => {
  it("classifies video before falling through to text", () => {
    expect(previewFormat("/a/clip.mp4")).toBe("video");
    expect(previewFormat("/a/pic.png")).toBe("image");
    expect(previewFormat("/a/notes.txt")).toBe("text");
  });
});
