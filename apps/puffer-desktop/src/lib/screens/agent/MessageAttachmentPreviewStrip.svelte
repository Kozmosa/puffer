<script lang="ts">
  import { onDestroy } from "svelte";
  import AttachmentPreviewStrip from "./AttachmentPreviewStrip.svelte";
  import { readChatAttachmentPreview } from "../../api/desktop";
  import type { ChatOpenIntent } from "../../chatOpenIntent";
  import type { MessageAttachment } from "../../types";

  type AttachmentPreviewMissState = NonNullable<MessageAttachment["state"]> | "unknown";

  type Props = {
    sessionId: string | null;
    attachments?: MessageAttachment[];
    onOpenChatIntent?: (intent: ChatOpenIntent) => void;
  };

  let {
    sessionId,
    attachments = [],
    onOpenChatIntent
  }: Props = $props();

  let previewUrls = $state<Record<string, string>>({});
  const previewLoads = new Set<string>();
  const previewMisses = new Map<string, AttachmentPreviewMissState>();
  let destroyed = false;

  function previewKey(sessionId: string, attachmentId: string): string {
    return `${sessionId}\u0000${attachmentId}`;
  }

  function previewMissState(attachment: MessageAttachment): AttachmentPreviewMissState {
    return attachment.state ?? "unknown";
  }

  function needsPreview(attachment: MessageAttachment): boolean {
    return attachment.kind === "image" && !attachment.previewUrl && attachment.state !== "missing";
  }

  function previewCandidates(): MessageAttachment[] {
    if (!sessionId) return [];
    const seen = new Set<string>();
    const candidates: MessageAttachment[] = [];
    for (const attachment of attachments) {
      if (!needsPreview(attachment) || seen.has(attachment.id)) continue;
      seen.add(attachment.id);
      candidates.push(attachment);
    }
    return candidates;
  }

  function previewStillNeeded(targetSessionId: string, attachmentId: string): boolean {
    if (sessionId !== targetSessionId) return false;
    return attachments.some(
      (attachment) => attachment.id === attachmentId && needsPreview(attachment)
    );
  }

  function attachmentsForDisplay(): MessageAttachment[] {
    if (!attachments.length || !sessionId) return attachments;
    return attachments.map((attachment) => {
      if (attachment.kind !== "image" || attachment.previewUrl) return attachment;
      const previewUrl = previewUrls[previewKey(sessionId, attachment.id)];
      return previewUrl ? { ...attachment, previewUrl } : attachment;
    });
  }

  function revokePreviewUrls(keys?: Set<string>): void {
    const entries = Object.entries(previewUrls).filter(([key]) => !keys || keys.has(key));
    if (entries.length === 0) return;
    const next = { ...previewUrls };
    for (const [key, previewUrl] of entries) {
      URL.revokeObjectURL(previewUrl);
      delete next[key];
      previewMisses.delete(key);
    }
    previewUrls = next;
  }

  function pruneStalePreviewUrls(activeKeys: Set<string>): void {
    const staleKeys = new Set(Object.keys(previewUrls).filter((key) => !activeKeys.has(key)));
    revokePreviewUrls(staleKeys);
    for (const key of Array.from(previewMisses.keys())) {
      if (!activeKeys.has(key)) previewMisses.delete(key);
    }
  }

  async function loadPreview(targetSessionId: string, attachment: MessageAttachment): Promise<void> {
    const key = previewKey(targetSessionId, attachment.id);
    const missState = previewMissState(attachment);
    if (
      previewUrls[key] ||
      previewLoads.has(key) ||
      previewMisses.get(key) === missState
    ) {
      return;
    }

    previewLoads.add(key);
    try {
      const preview = await readChatAttachmentPreview(targetSessionId, attachment.id);
      if (destroyed || !previewStillNeeded(targetSessionId, attachment.id)) return;
      if (preview.state !== "available") {
        previewMisses.set(key, missState);
        return;
      }

      const bytes = new Uint8Array(preview.bytes);
      const previewUrl = URL.createObjectURL(new Blob([bytes], { type: preview.mimeType }));
      const previous = previewUrls[key];
      if (previous) URL.revokeObjectURL(previous);
      previewMisses.delete(key);
      previewUrls = {
        ...previewUrls,
        [key]: previewUrl
      };
    } catch {
      if (!destroyed && previewStillNeeded(targetSessionId, attachment.id)) {
        previewMisses.set(key, missState);
      }
    } finally {
      previewLoads.delete(key);
    }
  }

  let visibleAttachments = $derived(attachmentsForDisplay());

  $effect(() => {
    const candidates = previewCandidates();
    const activeKeys = new Set(
      sessionId ? candidates.map((attachment) => previewKey(sessionId, attachment.id)) : []
    );
    pruneStalePreviewUrls(activeKeys);
    if (!sessionId) return;
    for (const attachment of candidates) {
      void loadPreview(sessionId, attachment);
    }
  });

  onDestroy(() => {
    destroyed = true;
    revokePreviewUrls();
  });
</script>

{#if visibleAttachments.length > 0}
  <AttachmentPreviewStrip
    attachments={visibleAttachments}
    variant="message"
    {onOpenChatIntent}
  />
{/if}
