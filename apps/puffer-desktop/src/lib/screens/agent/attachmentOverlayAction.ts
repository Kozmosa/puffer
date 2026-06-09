import type { MessageAttachment } from "../../types";

export type AttachmentOverlayAction =
  | { kind: "open_folder"; path: string }
  | { kind: "download"; url: string; suggestedName: string };

export function attachmentOverlayAction(
  attachment: MessageAttachment | null
): AttachmentOverlayAction | null {
  if (!attachment) return null;

  switch (attachment.source.kind) {
    case "local_file":
      return { kind: "open_folder", path: attachment.source.path };
    case "generated_media":
      return attachment.source.localPath
        ? { kind: "open_folder", path: attachment.source.localPath }
        : null;
    case "remote_url":
      // Only image attachments expose a download action. Remote-only videos and
      // plain files have no local folder to open and no generic remote download
      // path (the backend validates image downloads only), so they get no action.
      return attachment.kind === "image"
        ? { kind: "download", url: attachment.source.url, suggestedName: attachment.name }
        : null;
  }
}
