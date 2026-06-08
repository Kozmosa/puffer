import { beforeEach, expect, test, vi } from "vitest";
import type { AgentTurnOptions } from "./desktop";
import type { AttachmentPreviewResult, MessageAttachment } from "../types";

const invoke = vi.fn();
const request = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke
}));

vi.mock("./daemonClient", () => ({
  canInvokeTauri: () => true,
  canReachDaemon: () => true,
  configuredBrowserRemoteDaemonHandshake: () => null,
  ensureLocalDaemonClient: async () => ({ request }),
  switchDaemonClient: vi.fn()
}));

const attachment: MessageAttachment = {
  id: "attachment-1",
  name: "pixel.png",
  mimeType: "image/png",
  size: 5,
  extension: "PNG",
  kind: "image",
  state: "available",
  source: { kind: "local_file", path: "/tmp/puffer/attachments/pixel.png" },
  previewUrl: "blob:preview"
};

const remoteAttachment: MessageAttachment = {
  id: "remote-image",
  name: "remote.png",
  mimeType: "image/png",
  size: 0,
  extension: "PNG",
  kind: "image",
  state: "available",
  source: { kind: "remote_url", url: "https://example.test/remote.png" },
  previewUrl: "https://example.test/remote.png"
};

const turnOptions: AgentTurnOptions = {
  attachmentIds: [attachment.id]
};

const generatedAttachment: MessageAttachment = {
  id: "generated-image:artifact-1",
  name: "Generated image",
  mimeType: "image/png",
  size: 8,
  extension: "PNG",
  kind: "image",
  state: "available",
  source: {
    kind: "generated_media",
    jobId: "job-1",
    artifactId: "artifact-1",
    index: 0,
    localPath: "/tmp/puffer/.puffer/media/images/artifact-1/image.png",
    remoteSourceUrl: "https://example.test/source.png"
  }
};

const legacyTurnOptions: AgentTurnOptions = {
  // @ts-expect-error daemon-facing turn options no longer accept attachment metadata.
  attachments: [attachment]
};

const preview: AttachmentPreviewResult = {
  state: "available",
  mimeType: "image/png",
  bytes: [1, 2, 3]
};

void turnOptions;
void legacyTurnOptions;
void preview;

beforeEach(() => {
  invoke.mockReset();
  request.mockReset();
});

test("message attachments support explicit local file preview sources", () => {
  expect(attachment.source.kind).toBe("local_file");
  if (attachment.source.kind === "local_file") {
    expect(attachment.source.path).toBe("/tmp/puffer/attachments/pixel.png");
  }
});

test("message attachments support explicit remote URL preview sources", () => {
  expect(remoteAttachment.source.kind).toBe("remote_url");
  if (remoteAttachment.source.kind === "remote_url") {
    expect(remoteAttachment.source.url).toBe("https://example.test/remote.png");
  }
});

test("message attachments support generated media preview sources", () => {
  expect(generatedAttachment.source.kind).toBe("generated_media");
});

test("keeps generated media grouping fields", () => {
  expect(generatedAttachment.source.kind).toBe("generated_media");
  if (generatedAttachment.source.kind === "generated_media") {
    expect(generatedAttachment.source.jobId).toBe("job-1");
    expect(generatedAttachment.source.index).toBe(0);
    expect(generatedAttachment.source.localPath).toContain("artifact-1");
    expect(generatedAttachment.source.remoteSourceUrl).toBe("https://example.test/source.png");
  }
});

test("normalizes remote URL image attachments with URL previews", async () => {
  const { loadSessionDetailFromDaemon } = await import("./desktop");
  request.mockResolvedValueOnce({
    sessionId: "session-1",
    displayName: null,
    generatedTitle: null,
    title: "Remote image",
    cwd: "/tmp/puffer",
    folderPath: "/tmp/puffer",
    updatedAtMs: 1,
    createdAtMs: 1,
    eventCount: 1,
    slug: null,
    tags: [],
    note: null,
    parentSessionId: null,
    providerId: "codex",
    modelId: null,
    activityStatus: "idle",
    timeline: [
      {
        kind: "user_message",
        id: "message-1",
        text: "remote",
        createdAtMs: 1,
        attachments: [
          {
            id: "remote-image",
            name: "remote.png",
            mimeType: "image/png",
            size: 0,
            extension: "PNG",
            kind: "image",
            state: "available",
            source: { kind: "remote_url", url: "https://example.test/remote.png" }
          }
        ]
      }
    ],
    latestDiff: null,
    diffHistory: [],
    repoStatus: {
      sessionId: "session-1",
      cwd: "/tmp/puffer",
      repoRoot: null,
      branch: null,
      headSha: null,
      isClean: true,
      statusLines: [],
      hasGh: false,
      ghAuthenticated: false,
      canCreatePullRequest: false,
      canMergePullRequest: false,
      createPullRequestReason: null,
      mergePullRequestReason: null,
      openPullRequest: null,
      warnings: []
    },
    agentDiff: { files: [], entries: [] },
    divergence: { agentOnly: [], gitOnly: [], agentTotal: 0, gitTotal: 0 }
  });

  const detail = await loadSessionDetailFromDaemon("session-1");
  const normalized = detail.timeline[0].attachments?.[0];

  expect(normalized?.previewUrl).toBe("https://example.test/remote.png");
});

test("reads generated media previews by artifact id", async () => {
  const { readMessageAttachmentPreview } = await import("./desktop");
  request.mockResolvedValueOnce(preview);

  await expect(readMessageAttachmentPreview("session-1", generatedAttachment)).resolves.toBe(preview);

  expect(request).toHaveBeenCalledWith("read_generated_media_preview", {
    sessionId: "session-1",
    artifactId: "artifact-1"
  });
  expect(invoke).not.toHaveBeenCalled();
});

test("reads local file previews by stored attachment id", async () => {
  const { readMessageAttachmentPreview } = await import("./desktop");
  invoke.mockResolvedValueOnce(preview);

  await expect(readMessageAttachmentPreview("session-1", attachment)).resolves.toBe(preview);

  expect(invoke).toHaveBeenCalledWith("read_chat_attachment_preview", {
    sessionId: "session-1",
    attachmentId: "attachment-1"
  });
  expect(request).not.toHaveBeenCalled();
});

test("does not fetch remote URL previews through preview RPCs", async () => {
  const { readMessageAttachmentPreview } = await import("./desktop");

  await expect(readMessageAttachmentPreview("session-1", remoteAttachment)).resolves.toEqual({
    state: "unsupported"
  });

  expect(invoke).not.toHaveBeenCalled();
  expect(request).not.toHaveBeenCalled();
});
