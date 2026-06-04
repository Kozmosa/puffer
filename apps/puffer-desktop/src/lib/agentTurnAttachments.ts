import type { AgentTurnAttachment } from "./types";

function promptSafeAttachmentName(name: string): string {
  return name.replace(/[\u0000-\u001f\u007f]+/g, " ").replace(/\s+/g, " ").trim() || "attachment";
}

export function formatAgentTurnAttachmentLine(attachment: AgentTurnAttachment): string {
  const label = attachment.kind === "image" ? "Image" : "File";
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
