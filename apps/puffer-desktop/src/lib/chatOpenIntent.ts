import type { MessageAttachment } from "./types";

export type ChatFileTarget = { path: string; line: number | null };

export type ChatOpenIntent =
  | { kind: "file"; path: string; line: number | null }
  | { kind: "attachment"; attachment: MessageAttachment };

export function chatFileTarget(href: string): ChatFileTarget | null {
  let value = href.trim();
  if (value.startsWith("file://")) {
    try {
      value = decodeURIComponent(new URL(value).pathname);
    } catch {
      value = value.slice("file://".length);
    }
  }
  if (!value.startsWith("/")) return null;
  const match = value.match(/^(.*?):(\d+)(?::\d+)?$/);
  if (match) {
    return {
      path: match[1],
      line: Number(match[2])
    };
  }
  return { path: value, line: null };
}

export function fileOpenIntent(path: string, line: number | null = null): ChatOpenIntent {
  return { kind: "file", path, line };
}

export function attachmentOpenIntent(attachment: MessageAttachment): ChatOpenIntent {
  return { kind: "attachment", attachment };
}
