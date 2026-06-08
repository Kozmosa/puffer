import type { AgentTurnAttachment, MessageAttachment, TimelineItem } from "./types";

function promptSafeAttachmentName(name: string): string {
  return name.replace(/[\u0000-\u001f\u007f]+/g, " ").replace(/\s+/g, " ").trim() || "attachment";
}

export function formatAgentTurnAttachmentLine(attachment: AgentTurnAttachment): string {
  const label =
    attachment.kind === "image" ? "Image" : attachment.kind === "video" ? "Video" : "File";
  return `[${label}: ${promptSafeAttachmentName(attachment.name)}]`;
}

export function formatAgentTurnMessage(
  message: string,
  attachments: AgentTurnAttachment[] = []
): string {
  const attachmentLines = attachments.map(formatAgentTurnAttachmentLine);
  if (attachmentLines.length === 0) return message;
  return [message, attachmentLines.join("\n")]
    .filter((part) => part.trim().length > 0)
    .join("\n\n");
}

export function summarizeAgentTurnAttachments(attachments: AgentTurnAttachment[]): string {
  return attachments.map((attachment) => promptSafeAttachmentName(attachment.name)).join(", ");
}

export function stripAttachmentPreviewUrls(item: TimelineItem): TimelineItem {
  if (!item.attachments?.length) return item;
  return {
    ...item,
    attachments: item.attachments.map(({ previewUrl: _previewUrl, ...attachment }) => attachment)
  };
}

export function revokeMessageAttachmentPreviews(attachments: MessageAttachment[] = []): void {
  for (const attachment of attachments) {
    if (attachment.previewUrl?.startsWith("blob:")) URL.revokeObjectURL(attachment.previewUrl);
  }
}

export function revokeTimelineAttachmentPreviews(items: TimelineItem[] = []): void {
  for (const item of items) {
    revokeMessageAttachmentPreviews(item.attachments);
  }
}
