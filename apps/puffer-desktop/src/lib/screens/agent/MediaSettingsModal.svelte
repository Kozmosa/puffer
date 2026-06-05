<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { listMediaCapabilities, updateConfig } from "../../api/desktop";
  import Icon from "../../design/Icon.svelte";
  import type {
    ImageMediaSettings,
    MediaCapabilityInfo,
    MediaKind,
    MediaSettings,
    VideoMediaSettings
  } from "../../types";

  type Props = {
    kind: MediaKind;
    settings: MediaSettings;
    onClose: () => void;
  };

  let { kind, settings, onClose }: Props = $props();
  const initialSaved = untrack(() => (kind === "image" ? settings.image : settings.video));
  const initialImage = untrack(() => settings.image);
  const initialVideo = untrack(() => settings.video);

  const title = $derived(kind === "image" ? "Image settings" : "Video settings");
  const saveLabel = $derived(kind === "image" ? "Save image settings" : "Save video settings");
  const saved = $derived(kind === "image" ? settings.image : settings.video);

  let capabilities = $state<MediaCapabilityInfo[]>([]);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let providerId = $state(initialSaved.providerId ?? "");
  let modelId = $state(initialSaved.modelId ?? "");
  let size = $state(initialImage.size);
  let quality = $state(initialImage.quality);
  let outputFormat = $state(initialImage.outputFormat);
  let aspectRatio = $state(initialVideo.aspectRatio);
  let durationSeconds = $state(initialVideo.durationSeconds);
  let dialogEl: HTMLDivElement | undefined;
  let closeButtonEl: HTMLButtonElement | undefined;
  let previouslyFocusedEl: HTMLElement | null = null;

  let availableCapabilities = $derived(
    capabilities.filter((capability) => capability.kind === kind && capability.status === "available")
  );
  let providerOptions = $derived.by(() => {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const capability of availableCapabilities) {
      if (seen.has(capability.providerId)) continue;
      seen.add(capability.providerId);
      out.push(capability.providerId);
    }
    return out;
  });
  let modelOptions = $derived(
    availableCapabilities.filter((capability) => capability.providerId === providerId)
  );
  let selectedCapability = $derived(
    availableCapabilities.find(
      (capability) => capability.providerId === providerId && capability.modelId === modelId
    ) ?? null
  );
  let hasAvailableCapabilities = $derived(availableCapabilities.length > 0);
  let savedSelectionMissing = $derived(
    !loading &&
      Boolean(saved.providerId && saved.modelId) &&
      !availableCapabilities.some(
        (capability) =>
          capability.providerId === saved.providerId && capability.modelId === saved.modelId
      )
  );
  let canSave = $derived(Boolean(selectedCapability && !loading && !saving));
  let sizeOptions = $derived(parameterOptions("size", ["1024x1024"]));
  let qualityOptions = $derived(parameterOptions("quality", ["auto"]));
  let outputFormatOptions = $derived(parameterOptions("outputFormat", ["png"]));
  let aspectRatioOptions = $derived(parameterOptions("aspectRatio", ["16:9"]));
  let durationOptions = $derived(
    parameterOptions("durationSeconds", ["8"]).map((value) => Number(value)).filter(Boolean)
  );

  function parameterOptions(key: string, fallback: string[]): string[] {
    const values = selectedCapability?.parameterValues?.[key];
    if (!Array.isArray(values) || values.length === 0) return fallback;
    return values;
  }

  function chooseDefaultCapability() {
    if (availableCapabilities.length === 0) return;
    const savedCapability = availableCapabilities.find(
      (capability) => capability.providerId === providerId && capability.modelId === modelId
    );
    if (savedCapability) return;
    if (saved.providerId && saved.modelId) return;
    const first = availableCapabilities[0];
    providerId = first.providerId;
    modelId = first.modelId;
  }

  function handleProviderChange(value: string) {
    providerId = value;
    modelId = availableCapabilities.find((capability) => capability.providerId === value)?.modelId ?? "";
  }

  function withCurrentImage(): ImageMediaSettings {
    return {
      providerId: providerId || null,
      modelId: modelId || null,
      size,
      quality,
      outputFormat
    };
  }

  function withCurrentVideo(): VideoMediaSettings {
    return {
      providerId: providerId || null,
      modelId: modelId || null,
      aspectRatio,
      durationSeconds
    };
  }

  async function save() {
    if (!canSave) return;
    saving = true;
    error = null;
    try {
      const media: MediaSettings =
        kind === "image"
          ? { image: withCurrentImage(), video: { ...settings.video } }
          : { image: { ...settings.image }, video: withCurrentVideo() };
      await updateConfig({ media });
      close();
    } catch (saveError) {
      error = (saveError as Error).message ?? String(saveError);
    } finally {
      saving = false;
    }
  }

  function close() {
    onClose();
    if (previouslyFocusedEl?.isConnected) void tick().then(() => previouslyFocusedEl?.focus());
  }

  function focusableElements(): HTMLElement[] {
    if (!dialogEl) return [];
    return Array.from(
      dialogEl.querySelectorAll<HTMLElement>(
        "button:not(:disabled), select:not(:disabled), input:not(:disabled)"
      )
    ).filter((element) => element.offsetParent !== null);
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = focusableElements();
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  onMount(() => {
    previouslyFocusedEl = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    void tick().then(() => closeButtonEl?.focus());
    const load = async () => {
      loading = true;
      error = null;
      try {
        capabilities = await listMediaCapabilities(kind);
        chooseDefaultCapability();
      } catch (loadError) {
        error = (loadError as Error).message ?? String(loadError);
        capabilities = [];
      } finally {
        loading = false;
      }
    };
    void load();
    document.addEventListener("keydown", handleKeydown);
    return () => {
      document.removeEventListener("keydown", handleKeydown);
    };
  });
</script>

<div class="pf-media-modal-backdrop" role="presentation">
  <div
    bind:this={dialogEl}
    class="pf-media-modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="pf-media-modal-title"
  >
    <header class="pf-media-modal-head">
      <h2 id="pf-media-modal-title">{title}</h2>
      <button
        bind:this={closeButtonEl}
        type="button"
        class="pf-media-icon-btn"
        aria-label="Close media settings"
        onclick={close}
      >
        <Icon name="x" size={15} />
      </button>
    </header>

    <div class="pf-media-modal-body">
      {#if loading}
        <p class="pf-media-state">Loading {kind} capabilities...</p>
      {:else if error}
        <p class="pf-media-state warn" role="alert">{error}</p>
      {:else if !hasAvailableCapabilities}
        <p class="pf-media-state">No {kind} capabilities available.</p>
      {:else}
        {#if savedSelectionMissing}
          <p class="pf-media-state warn" role="alert">Saved model is no longer available.</p>
        {/if}

        <label>
          <span>Provider</span>
          <select value={providerId} onchange={(event) => handleProviderChange(event.currentTarget.value)}>
            {#if providerId && !providerOptions.includes(providerId)}
              <option value={providerId} disabled>{providerId} unavailable</option>
            {/if}
            {#each providerOptions as provider}
              <option value={provider}>{provider}</option>
            {/each}
          </select>
        </label>

        <label>
          <span>Model</span>
          <select value={modelId} onchange={(event) => (modelId = event.currentTarget.value)}>
            {#if modelId && !modelOptions.some((capability) => capability.modelId === modelId)}
              <option value={modelId} disabled>{modelId} unavailable</option>
            {/if}
            {#each modelOptions as capability}
              <option value={capability.modelId}>{capability.modelId}</option>
            {/each}
          </select>
        </label>

        {#if kind === "image"}
          <div class="pf-media-grid">
            <label>
              <span>Size</span>
              <select value={size} onchange={(event) => (size = event.currentTarget.value)}>
                {#each sizeOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>
            <label>
              <span>Quality</span>
              <select value={quality} onchange={(event) => (quality = event.currentTarget.value)}>
                {#each qualityOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>
            <label>
              <span>Output format</span>
              <select value={outputFormat} onchange={(event) => (outputFormat = event.currentTarget.value)}>
                {#each outputFormatOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>
          </div>
        {:else}
          <div class="pf-media-grid">
            <label>
              <span>Aspect ratio</span>
              <select value={aspectRatio} onchange={(event) => (aspectRatio = event.currentTarget.value)}>
                {#each aspectRatioOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>
            <label>
              <span>Duration</span>
              <select value={String(durationSeconds)} onchange={(event) => (durationSeconds = Number(event.currentTarget.value))}>
                {#each durationOptions as option}
                  <option value={String(option)}>{option}s</option>
                {/each}
              </select>
            </label>
          </div>
        {/if}
      {/if}
    </div>

    <footer class="pf-media-modal-actions">
      <button type="button" class="pf-media-secondary-btn" onclick={close}>Cancel</button>
      <button type="button" class="pf-media-primary-btn" disabled={!canSave} onclick={save}>
        {saving ? "Saving..." : saveLabel}
      </button>
    </footer>
  </div>
</div>

<style>
  .pf-media-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    padding: 20px;
    background: color-mix(in oklab, var(--background) 68%, transparent);
  }

  .pf-media-modal {
    width: min(460px, 100%);
    max-height: min(620px, calc(100vh - 40px));
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--background);
    color: var(--foreground);
    box-shadow: var(--shadow-lg);
  }

  .pf-media-modal-head,
  .pf-media-modal-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
  }

  .pf-media-modal-actions {
    justify-content: flex-end;
    border-top: 1px solid var(--border);
    border-bottom: 0;
  }

  .pf-media-modal-head h2 {
    margin: 0;
    font-size: 14px;
    line-height: 1.2;
  }

  .pf-media-icon-btn {
    width: 28px;
    height: 28px;
    display: inline-grid;
    place-items: center;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--muted-foreground);
    cursor: pointer;
  }

  .pf-media-icon-btn:hover {
    background: var(--accent);
    color: var(--foreground);
  }

  .pf-media-modal-body {
    display: grid;
    gap: 12px;
    padding: 14px;
    overflow: auto;
  }

  .pf-media-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }

  .pf-media-modal label {
    display: grid;
    gap: 6px;
    min-width: 0;
    font-size: 12px;
    font-weight: 650;
    color: var(--muted-foreground);
  }

  .pf-media-modal select {
    min-width: 0;
    height: 34px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--background);
    color: var(--foreground);
    font: inherit;
    font-weight: 600;
    padding: 0 8px;
  }

  .pf-media-state {
    margin: 0;
    min-height: 34px;
    display: flex;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0 10px;
    color: var(--muted-foreground);
    font-size: 12.5px;
  }

  .pf-media-state.warn {
    border-color: color-mix(in oklab, var(--pf-run-failed) 35%, var(--border));
    color: var(--foreground);
  }

  .pf-media-primary-btn,
  .pf-media-secondary-btn {
    height: 32px;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0 12px;
    font: inherit;
    font-size: 12.5px;
    font-weight: 700;
    cursor: pointer;
  }

  .pf-media-primary-btn {
    border-color: color-mix(in oklab, var(--puffer-accent) 42%, var(--border));
    background: var(--puffer-accent);
    color: var(--puffer-accent-foreground);
  }

  .pf-media-primary-btn:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .pf-media-secondary-btn {
    background: transparent;
    color: var(--foreground);
  }

  .pf-media-secondary-btn:hover {
    background: var(--accent);
  }

  @media (max-width: 560px) {
    .pf-media-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
