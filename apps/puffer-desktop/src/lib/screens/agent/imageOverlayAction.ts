import type { MessageAttachment } from "../../types";

export type ImageOverlayAction =
  | { kind: "open_folder"; path: string }
  | { kind: "download"; url: string; suggestedName: string };

export function imageOverlayAction(attachment: MessageAttachment | null): ImageOverlayAction | null {
  if (!attachment || attachment.kind !== "image") return null;

  switch (attachment.source.kind) {
    case "local_file":
      return attachment.source.path ? { kind: "open_folder", path: attachment.source.path } : null;
    case "remote_url":
      return attachment.source.url
        ? { kind: "download", url: attachment.source.url, suggestedName: attachment.name }
        : null;
    case "generated_media":
      return attachment.source.localPath
        ? { kind: "open_folder", path: attachment.source.localPath }
        : null;
  }
}
