<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { listMediaCapabilities, updateConfig } from "../../api/desktop";
  import Icon from "../../design/Icon.svelte";
  import { focusTrap } from "../../focusTrap";
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
    settingsReady?: boolean;
    onClose: () => void;
  };

  let { kind, settings, settingsReady = true, onClose }: Props = $props();
  const initialSaved = untrack(() => mediaSettingsForKind(kind, settings));
  const initialImage = untrack(() => settings.image);
  const initialVideo = untrack(() => settings.video);

  const title = $derived(mediaSettingsTitle(kind));
  const saveLabel = $derived(kind === "image" ? "Save" : `Save ${title.toLowerCase()}`);
  const closeLabel = $derived(`Close ${title.toLowerCase()}`);
  const saved = $derived(mediaSettingsForKind(kind, settings));
  const COMMON_IMAGE_SIZES = [
    "256x256",
    "512x512",
    "768x768",
    "1024x1024",
    "1024x1536",
    "1536x1024",
    "1024x1792",
    "1792x1024",
    "2048x2048"
  ];

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
  let appliedSettingsKey = $state(untrack(() => mediaSettingsKey(kind, settings)));

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
  let canSave = $derived(Boolean(settingsReady && selectedCapability && !loading && !saving));
  let sizeOptions = $derived(mergedParameterOptions("size", COMMON_IMAGE_SIZES));
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

  function mergedParameterOptions(key: string, fallback: string[]): string[] {
    const values = selectedCapability?.parameterValues?.[key];
    return Array.from(new Set([...(Array.isArray(values) ? values : []), ...fallback]));
  }

  function mediaSettingsTitle(mediaKind: MediaKind): string {
    return mediaKind === "image" ? "Image generation settings" : "Video generation settings";
  }

  function mediaSettingsForKind(mediaKind: MediaKind, mediaSettings: MediaSettings) {
    return mediaKind === "image" ? mediaSettings.image : mediaSettings.video;
  }

  function mediaSettingsKey(mediaKind: MediaKind, mediaSettings: MediaSettings): string {
    const image = mediaSettings.image;
    const video = mediaSettings.video;
    return [
      mediaKind,
      image.providerId ?? "",
      image.modelId ?? "",
      image.size,
      image.quality,
      image.outputFormat,
      video.providerId ?? "",
      video.modelId ?? "",
      video.aspectRatio,
      String(video.durationSeconds)
    ].join("\u0000");
  }

  function applySettings(mediaSettings: MediaSettings) {
    const current = mediaSettingsForKind(kind, mediaSettings);
    providerId = current.providerId ?? "";
    modelId = current.modelId ?? "";
    size = mediaSettings.image.size;
    quality = mediaSettings.image.quality;
    outputFormat = mediaSettings.image.outputFormat;
    aspectRatio = mediaSettings.video.aspectRatio;
    durationSeconds = mediaSettings.video.durationSeconds;
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

  function providerLabel(value: string): string {
    const normalized = value.trim().toLowerCase();
    if (normalized === "openai" || normalized === "codex") return "OpenAI";
    if (normalized === "anthropic" || normalized === "claude") return "Claude";
    if (normalized === "replicate") return "Replicate";
    if (normalized === "fal" || normalized === "fal-ai") return "fal.ai";
    if (normalized === "puffer") return "Puffer";
    return value
      .split(/[-_\s]+/)
      .filter(Boolean)
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ") || value;
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
  }

  onMount(() => {
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
  });

  $effect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !saving) close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  $effect(() => {
    if (!settingsReady) return;
    const nextKey = mediaSettingsKey(kind, settings);
    if (nextKey === appliedSettingsKey) return;
    appliedSettingsKey = nextKey;
    applySettings(settings);
    chooseDefaultCapability();
  });
</script>

<div
  class="pf-modal-scrim"
  role="presentation"
  onclick={() => {
    if (!saving) close();
  }}
  onkeydown={() => {}}
>
  <div
    class="pf-modal pf-media-modal"
    onclick={(event) => event.stopPropagation()}
    role="dialog"
    aria-modal="true"
    aria-labelledby="pf-media-modal-title"
    tabindex="-1"
    use:focusTrap
    onkeydown={() => {}}
  >
    <header class="pf-modal-head">
      <div class="pf-modal-title-group">
        <div id="pf-media-modal-title" class="pf-modal-title">{title}</div>
      </div>
      <button
        type="button"
        class="pf-modal-close"
        aria-label={closeLabel}
        disabled={saving}
        onclick={close}
      >
        <Icon name="x" size={14} />
      </button>
    </header>

    <div class="pf-modal-body pf-media-modal-body">
      {#if !settingsReady}
        <p class="pf-media-state">Loading generation settings...</p>
      {:else if loading}
        <p class="pf-media-state">Loading {kind} capabilities...</p>
      {:else if error}
        <p class="pf-media-state" data-warning="true" role="alert">{error}</p>
      {:else if !hasAvailableCapabilities}
        <p class="pf-media-state">No {kind} capabilities available.</p>
      {:else}
        {#if savedSelectionMissing}
          <p class="pf-media-state" data-warning="true" role="alert">Saved model is no longer available.</p>
        {/if}

        <div class="pf-media-form-grid">
          <label class="pf-media-field">
            <span class="pf-field-label">Provider</span>
            <select
              class="sc-input"
              value={providerId}
              onchange={(event) => handleProviderChange(event.currentTarget.value)}
            >
              {#if providerId && !providerOptions.includes(providerId)}
                <option value={providerId} disabled>{providerId} unavailable</option>
              {/if}
              {#each providerOptions as provider}
                <option value={provider}>{providerLabel(provider)}</option>
              {/each}
            </select>
          </label>

          <label class="pf-media-field">
            <span class="pf-field-label">Model</span>
            <select class="sc-input" value={modelId} onchange={(event) => (modelId = event.currentTarget.value)}>
              {#if modelId && !modelOptions.some((capability) => capability.modelId === modelId)}
                <option value={modelId} disabled>{modelId} unavailable</option>
              {/if}
              {#each modelOptions as capability}
                <option value={capability.modelId}>{capability.modelId}</option>
              {/each}
            </select>
          </label>

          {#if kind === "image"}
            <label class="pf-media-field">
              <span class="pf-field-label">Size</span>
              <select class="sc-input" value={size} onchange={(event) => (size = event.currentTarget.value)}>
                {#each sizeOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>
            <label class="pf-media-field">
              <span class="pf-field-label">Quality</span>
              <select class="sc-input" value={quality} onchange={(event) => (quality = event.currentTarget.value)}>
                {#each qualityOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>
            <label class="pf-media-field">
              <span class="pf-field-label">Output format</span>
              <select class="sc-input" value={outputFormat} onchange={(event) => (outputFormat = event.currentTarget.value)}>
                {#each outputFormatOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>
          {:else}
            <label class="pf-media-field">
              <span class="pf-field-label">Aspect ratio</span>
              <select class="sc-input" value={aspectRatio} onchange={(event) => (aspectRatio = event.currentTarget.value)}>
                {#each aspectRatioOptions as option}
                  <option value={option}>{option}</option>
                {/each}
              </select>
            </label>
            <label class="pf-media-field">
              <span class="pf-field-label">Duration</span>
              <select
                class="sc-input"
                value={String(durationSeconds)}
                onchange={(event) => (durationSeconds = Number(event.currentTarget.value))}
              >
                {#each durationOptions as option}
                  <option value={String(option)}>{option}s</option>
                {/each}
              </select>
            </label>
          {/if}
        </div>
      {/if}
    </div>

    <footer class="pf-modal-foot">
      <div class="pf-modal-foot-btns">
        <button type="button" class="sc-btn" data-variant="ghost" data-size="sm" onclick={close} disabled={saving}>
          Cancel
        </button>
        <button type="button" class="sc-btn" data-variant="default" data-size="sm" disabled={!canSave} onclick={save}>
          <Icon name="check" size={13} />{saving ? "Saving..." : saveLabel}
        </button>
      </div>
    </footer>
  </div>
</div>

<style>
  .pf-modal-scrim {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 48px 24px;
    background: color-mix(in oklch, var(--background) 30%, transparent 70%);
    animation: pf-modal-scrim-in 140ms ease-out;
  }

  @keyframes pf-modal-scrim-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .pf-modal {
    max-height: calc(100vh - 96px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 14px;
    background: var(--card);
    color: var(--card-foreground);
    box-shadow: 0 24px 64px -12px oklch(0 0 0 / 0.35), 0 4px 16px -4px oklch(0 0 0 / 0.2);
    animation: pf-modal-in 160ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }

  @keyframes pf-modal-in {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .pf-media-modal {
    width: min(480px, calc(100vw - 28px));
  }

  .pf-modal-head {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 12px 20px 6px;
    flex-shrink: 0;
  }

  .pf-modal-title-group {
    display: flex;
    flex: 1 1 0;
    min-width: 0;
    flex-direction: column;
    gap: 2px;
  }

  .pf-modal-title {
    color: var(--foreground);
    font-size: 17px;
    line-height: 22px;
    font-weight: 600;
    letter-spacing: 0;
  }

  .pf-modal-close {
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border: 1px solid transparent;
    border-radius: 7px;
    background: transparent;
    color: var(--foreground);
    cursor: pointer;
  }

  .pf-modal-close:hover:not(:disabled) {
    background: var(--muted);
    color: var(--foreground);
  }

  .pf-modal-close:disabled {
    cursor: default;
    opacity: 0.5;
  }

  .pf-modal-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px 20px 4px;
  }

  .pf-media-modal-body {
    font-size: 12px;
  }

  .pf-media-form-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 12px;
  }

  .pf-media-field {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 6px;
  }

  .pf-field-label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--foreground);
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: -0.005em;
  }

  .pf-media-field select {
    width: 100%;
    min-width: 0;
  }

  .pf-media-state {
    margin: 0;
    min-height: 36px;
    display: flex;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0 10px;
    color: var(--muted-foreground);
    background: color-mix(in oklab, var(--background) 94%, var(--muted));
    font-size: 12px;
    line-height: 1.4;
  }

  .pf-media-state[data-warning="true"] {
    border-color: color-mix(in oklab, var(--destructive) 30%, var(--border));
    background: color-mix(in oklab, var(--destructive) 8%, var(--background));
    color: var(--foreground);
  }

  .pf-modal-foot {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
    flex-shrink: 0;
    padding: 12px 20px;
  }

  .pf-modal-foot-btns {
    display: flex;
    flex-shrink: 0;
    gap: 8px;
    margin-left: auto;
  }

  @media (max-width: 560px) {
    .pf-modal-scrim {
      padding: 24px 14px;
    }

    .pf-modal-foot-btns,
    .pf-modal-foot-btns .sc-btn {
      width: 100%;
    }

    .pf-modal-foot-btns {
      display: grid;
      grid-template-columns: auto minmax(0, 1fr);
    }

    .pf-modal-foot-btns .sc-btn {
      min-width: 0;
    }
  }
</style>
