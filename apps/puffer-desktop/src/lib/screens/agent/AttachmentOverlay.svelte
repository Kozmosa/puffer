<script lang="ts">
  import { tick } from "svelte";
  import { downloadImageFromUrl, openImageContainingFolder } from "../../api/desktop";
  import Icon, { type IconName } from "../../design/Icon.svelte";
  import type { MessageAttachment } from "../../types";
  import { imageOverlayAction, type ImageOverlayAction } from "./imageOverlayAction";

  type Props = {
    attachment: MessageAttachment | null;
    onClose: () => void;
  };

  let { attachment, onClose }: Props = $props();
  let closeButtonEl = $state<HTMLButtonElement | undefined>(undefined);
  let actionBusy = $state(false);
  let actionError = $state<string | null>(null);
  let actionSavedPath = $state<string | null>(null);
  let titleId = $derived(attachment ? `attachment-overlay-title-${attachment.id}` : "attachment-overlay-title");
  let canPreviewImage = $derived(Boolean(attachment?.kind === "image" && attachment.previewUrl));
  let canPreviewVideo = $derived(Boolean(attachment?.kind === "video" && attachment.previewUrl));
  let overlayAction = $derived(imageOverlayAction(attachment));
  let overlayActionLabel = $derived(
    overlayAction?.kind === "download"
      ? "Download image"
      : attachment?.kind === "video"
        ? "Open video folder"
        : "Open image folder"
  );
  let overlayActionIcon = $derived<IconName>(
    overlayAction?.kind === "download" ? "download" : "folderOpen"
  );
  let actionResetKey = $derived(
    attachment ? `${attachment.id}:${overlayActionKey(overlayAction)}` : "none"
  );

  function formatBytes(size: number): string {
    if (!Number.isFinite(size) || size < 0) return "Unknown size";
    if (size < 1024) return `${size} B`;
    const kib = size / 1024;
    if (kib < 1024) return `${kib.toFixed(kib >= 10 ? 0 : 1)} KiB`;
    const mib = kib / 1024;
    return `${mib.toFixed(mib >= 10 ? 0 : 1)} MiB`;
  }

  function close() {
    onClose();
  }

  function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  function overlayActionKey(action: ImageOverlayAction | null): string {
    if (!action) return "none";
    return action.kind === "download"
      ? `download:${action.url}:${action.suggestedName}`
      : `open_folder:${action.path}`;
  }

  async function runOverlayAction() {
    const action = overlayAction;
    if (!action || actionBusy) return;

    actionBusy = true;
    actionError = null;
    actionSavedPath = null;
    try {
      if (action.kind === "open_folder") {
        await openImageContainingFolder(action.path);
      } else {
        const result = await downloadImageFromUrl(action.url, action.suggestedName);
        actionSavedPath = result.path;
      }
    } catch (error) {
      actionError = errorMessage(error);
    } finally {
      actionBusy = false;
    }
  }

  $effect(() => {
    const resetKey = actionResetKey;
    if (!resetKey) return;
    actionBusy = false;
    actionError = null;
    actionSavedPath = null;
  });

  $effect(() => {
    if (!attachment || typeof window === "undefined") return;
    const previouslyFocusedEl =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    void tick().then(() => closeButtonEl?.focus());
    const handleKeydown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      close();
    };
    window.addEventListener("keydown", handleKeydown);
    return () => {
      window.removeEventListener("keydown", handleKeydown);
      if (previouslyFocusedEl?.isConnected) void tick().then(() => previouslyFocusedEl.focus());
    };
  });
</script>

{#if attachment}
  <div
    class="pf-attachment-overlay"
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    data-testid="attachment-overlay"
  >
    <button
      type="button"
      class="pf-attachment-overlay-backdrop"
      aria-label="Close attachment preview"
      onclick={close}
    ></button>
    <section class="pf-attachment-dialog">
      <header class="pf-attachment-dialog-head">
        <div class="pf-attachment-dialog-meta">
          <h2 id={titleId}>{attachment.name}</h2>
          <p>{attachment.extension} · {attachment.mimeType} · {formatBytes(attachment.size)}</p>
          {#if actionError}
            <span class="pf-attachment-action-status pf-attachment-action-status-error">
              {actionError}
            </span>
          {:else if actionSavedPath}
            <span class="pf-attachment-action-status">
              Downloaded to {actionSavedPath}
            </span>
          {/if}
        </div>
        <div class="pf-attachment-dialog-actions">
          {#if overlayAction}
            <button
              type="button"
              class="pf-attachment-dialog-action"
              aria-label={overlayActionLabel}
              title={overlayActionLabel}
              disabled={actionBusy}
              onclick={runOverlayAction}
            >
              <Icon name={overlayActionIcon} size={15} />
            </button>
          {/if}
          <button
            bind:this={closeButtonEl}
            type="button"
            class="pf-attachment-dialog-action pf-attachment-dialog-close"
            aria-label="Close attachment preview"
            onclick={close}
          >
            <Icon name="x" size={15} />
          </button>
        </div>
      </header>

      {#if canPreviewImage && attachment.previewUrl}
        <div class="pf-attachment-image-frame">
          <img src={attachment.previewUrl} alt={attachment.name} draggable="false" />
        </div>
      {:else if canPreviewVideo && attachment.previewUrl}
        <div class="pf-attachment-video-frame">
          <video src={attachment.previewUrl} controls autoplay playsinline></video>
        </div>
      {:else}
        <div class="pf-attachment-unavailable">
          <span class="pf-attachment-unavailable-icon">
            <Icon name="file" size={24} />
          </span>
          <strong>Preview unavailable for this attachment.</strong>
          <span>This chat item has attachment metadata, but no durable preview content.</span>
        </div>
      {/if}
    </section>
  </div>
{/if}

<style>
  .pf-attachment-overlay {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    padding: 32px;
  }
  .pf-attachment-overlay-backdrop {
    position: absolute;
    inset: 0;
    border: 0;
    background: color-mix(in oklab, black 48%, transparent);
  }
  .pf-attachment-dialog {
    position: relative;
    width: min(860px, 100%);
    max-height: min(760px, 90vh);
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--background);
    box-shadow: var(--shadow-lg);
  }
  .pf-attachment-dialog-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
  }
  .pf-attachment-dialog-meta {
    min-width: 0;
  }
  .pf-attachment-dialog-head h2 {
    margin: 0;
    font-size: 14px;
    line-height: 20px;
    font-weight: 700;
  }
  .pf-attachment-dialog-head p {
    margin: 2px 0 0;
    color: var(--muted-foreground);
    font-size: 12px;
    line-height: 16px;
  }
  .pf-attachment-action-status {
    display: block;
    margin-top: 4px;
    color: var(--muted-foreground);
    font-size: 12px;
    line-height: 16px;
    overflow-wrap: anywhere;
  }
  .pf-attachment-action-status-error {
    color: var(--destructive, #b42318);
  }
  .pf-attachment-dialog-actions {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    flex: 0 0 auto;
  }
  .pf-attachment-dialog-action {
    width: 30px;
    height: 30px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--background);
    color: var(--muted-foreground);
    cursor: pointer;
  }
  .pf-attachment-dialog-action:hover:not(:disabled) {
    color: var(--foreground);
    background: var(--accent);
  }
  .pf-attachment-dialog-action:disabled {
    cursor: default;
    opacity: 0.55;
  }
  .pf-attachment-image-frame {
    min-height: 240px;
    display: grid;
    place-items: center;
    overflow: auto;
    background: color-mix(in oklab, var(--muted) 45%, black);
  }
  .pf-attachment-image-frame img {
    max-width: 100%;
    max-height: 72vh;
    display: block;
    object-fit: contain;
  }
  .pf-attachment-video-frame {
    min-height: 0;
    display: grid;
    place-items: center;
    background: black;
  }
  .pf-attachment-video-frame video {
    width: 100%;
    max-height: calc(90vh - 82px);
    display: block;
    background: black;
  }
  .pf-attachment-unavailable {
    min-height: 240px;
    display: grid;
    place-items: center;
    align-content: center;
    gap: 8px;
    padding: 32px;
    color: var(--muted-foreground);
    text-align: center;
  }
  .pf-attachment-unavailable strong {
    color: var(--foreground);
    font-size: 14px;
  }
  .pf-attachment-unavailable-icon {
    width: 48px;
    height: 48px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--muted);
  }
</style>
