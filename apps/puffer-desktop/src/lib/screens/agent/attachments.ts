import type { AgentTurnAttachment, AgentTurnAttachmentKind } from "../../types";

export const MAX_ATTACHMENTS = 10;
export const MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024;

export const FILE_INPUT_ACCEPT = [
  "image/*",
  "application/pdf",
  "text/*",
  ".md",
  ".markdown",
  ".json",
  ".jsonl",
  ".yaml",
  ".yml",
  ".xml",
  ".csv",
  ".ts",
  ".tsx",
  ".js",
  ".jsx",
  ".mjs",
  ".cjs",
  ".py",
  ".rb",
  ".go",
  ".rs",
  ".java",
  ".kt",
  ".swift",
  ".c",
  ".cpp",
  ".h",
  ".hpp",
  ".cs",
  ".php",
  ".sh",
  ".bash",
  ".zsh",
  ".sql",
  ".toml",
  ".ini",
  ".env",
  ".zip",
  ".tar",
  ".gz",
  ".tgz",
  ".doc",
  ".docx",
  ".ppt",
  ".pptx",
  ".xls",
  ".xlsx"
].join(",");

export type ComposerAttachmentDraft = {
  id: string;
  file: File;
  name: string;
  mimeType: string;
  size: number;
  extension: string;
  kind: AgentTurnAttachmentKind;
  previewUrl?: string;
};

type AddAttachmentFilesInput = {
  current: ComposerAttachmentDraft[];
  files: FileList | File[] | null;
  nextId: () => string;
};

type AddAttachmentFilesResult = {
  attachments: ComposerAttachmentDraft[];
  error: string | null;
};

export function formatAttachmentExtension(file: File): string {
  const extension = file.name.includes(".") ? file.name.split(".").pop() || "" : "";
  if (extension) return extension.toUpperCase();
  if (file.type) return file.type.split("/").pop()?.toUpperCase() || "FILE";
  return "FILE";
}

export function dataTransferHasFiles(dataTransfer: DataTransfer | null | undefined): boolean {
  if (!dataTransfer) return false;
  return Array.from(dataTransfer.types ?? []).includes("Files");
}

export function filesFromDataTransfer(dataTransfer: DataTransfer | null | undefined): File[] {
  if (!dataTransferHasFiles(dataTransfer)) return [];
  return Array.from(dataTransfer?.files ?? []);
}

export function attachmentPayloadFromDraft(
  attachment: ComposerAttachmentDraft
): AgentTurnAttachment {
  return {
    id: attachment.id,
    name: attachment.name,
    mimeType: attachment.mimeType,
    size: attachment.size,
    extension: attachment.extension,
    kind: attachment.kind
  };
}

export function addAttachmentFiles({
  current,
  files,
  nextId
}: AddAttachmentFilesInput): AddAttachmentFilesResult {
  const selectedFiles = files ? Array.from(files) : [];
  if (selectedFiles.length === 0) return { attachments: current, error: null };

  const availableSlots = Math.max(0, MAX_ATTACHMENTS - current.length);
  const acceptedFiles: File[] = [];
  let rejectedForSize = 0;
  let rejectedForCount = 0;

  for (const file of selectedFiles) {
    if (file.size > MAX_ATTACHMENT_BYTES) {
      rejectedForSize += 1;
      continue;
    }
    if (acceptedFiles.length >= availableSlots) {
      rejectedForCount += 1;
      continue;
    }
    acceptedFiles.push(file);
  }

  const attachments =
    acceptedFiles.length > 0
      ? [...current, ...acceptedFiles.map((file) => createAttachmentDraft(file, nextId()))]
      : current;

  const messages: string[] = [];
  if (rejectedForCount > 0) messages.push(`Only ${MAX_ATTACHMENTS} attachments can be added.`);
  if (rejectedForSize > 0) messages.push("Files must be 20 MiB or smaller.");

  return {
    attachments,
    error: messages.length > 0 ? messages.join(" ") : null
  };
}

export function revokeAttachmentPreview(attachment: ComposerAttachmentDraft): void {
  if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl);
}

export function revokeAttachmentPreviews(attachments: ComposerAttachmentDraft[]): void {
  attachments.forEach(revokeAttachmentPreview);
}

function createAttachmentDraft(file: File, id: string): ComposerAttachmentDraft {
  const kind: AgentTurnAttachmentKind = file.type.startsWith("image/") ? "image" : "file";
  return {
    id,
    file,
    name: file.name,
    mimeType: file.type || "application/octet-stream",
    size: file.size,
    extension: formatAttachmentExtension(file),
    kind,
    ...(kind === "image" ? { previewUrl: URL.createObjectURL(file) } : {})
  };
}
