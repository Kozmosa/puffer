<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, untrack } from "svelte";
  import { listMediaCapabilities, updateConfig } from "../../api/desktop";
  import Icon from "../../design/Icon.svelte";
  import { focusTrap } from "../../focusTrap";
  import type {
    ImageMediaSettings,
    MediaCapabilityInfo,
    MediaCapabilityParameterInfo,
    MediaKind,
    MediaSettings,
    VideoMediaSettings
  } from "../../types";

  type Props = {
    kind: MediaKind;
    sessionCwd: string;
    settings: MediaSettings;
    settingsReady?: boolean;
    onClose: () => void;
  };

  let { kind, sessionCwd, settings, settingsReady = true, onClose }: Props = $props();
  const IMAGE_OUTPUT_DIR_RELATIVE = ".puffer/media/images";
  const initialSaved = untrack(() => mediaSettingsForKind(kind, settings));
  const initialImage = untrack(() => settings.image);
  const initialVideo = untrack(() => settings.video);

  const title = $derived(mediaSettingsTitle(kind));
  const saveLabel = $derived(kind === "image" ? "Save" : `Save ${title.toLowerCase()}`);
  const closeLabel = $derived(`Close ${title.toLowerCase()}`);
  const saved = $derived(mediaSettingsForKind(kind, settings));
  const imageDir = $derived(imageDirForSessionCwd(sessionCwd));
  const aspectRatioOptions = ["16:9"];
  const durationOptions = [8];

  let capabilities = $state<MediaCapabilityInfo[]>([]);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let openError = $state<string | null>(null);
  let providerId = $state(initialSaved.providerId ?? "");
  let modelId = $state(initialSaved.modelId ?? "");
  let adapter = $state(initialImage.adapter ?? "");
  let parameters = $state<Record<string, string>>({ ...initialImage.parameters });
  let aspectRatio = $state(initialVideo.aspectRatio);
  let durationSeconds = $state(initialVideo.durationSeconds);
  let appliedSettingsKey = $state(untrack(() => mediaSettingsKey(kind, settings)));

  let availableCapabilities = $derived(
    capabilities.filter((capability) => capability.kind === kind && capability.status === "available")
  );
  let providerOptions = $derived.by(() => {
    const seen = new Set<string>();
    const out: { id: string; label: string }[] = [];
    for (const capability of availableCapabilities) {
      if (seen.has(capability.providerId)) continue;
      seen.add(capability.providerId);
      out.push({
        id: capability.providerId,
        label: capability.providerDisplayName || capability.providerId
      });
    }
    return out;
  });
  let modelOptions = $derived(
    availableCapabilities.filter((capability) => capability.providerId === providerId)
  );
  let selectedCapability = $derived(
    availableCapabilities.find(
      (capability) =>
        capability.providerId === providerId &&
        capability.modelId === modelId &&
        capability.adapter === adapter
    ) ?? null
  );
  let hasAvailableCapabilities = $derived(availableCapabilities.length > 0);
  let savedSelectionMissing = $derived(
    !loading &&
      savedSelectionIsConfigured(kind, settings) &&
      !savedSelectionIsAvailable(kind, settings, availableCapabilities)
  );
  let canSave = $derived(Boolean(settingsReady && selectedCapability && !loading && !saving));

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
      image.adapter ?? "",
      serializeParameters(image.parameters),
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
    adapter = mediaSettings.image.adapter ?? "";
    parameters = { ...mediaSettings.image.parameters };
    aspectRatio = mediaSettings.video.aspectRatio;
    durationSeconds = mediaSettings.video.durationSeconds;
  }

  function chooseDefaultCapability() {
    if (availableCapabilities.length === 0) return;
    const savedCapability = availableCapabilities.find(
      (capability) =>
        capability.providerId === providerId &&
        capability.modelId === modelId &&
        capability.adapter === adapter
    );
    if (savedCapability) return;
    if (savedSelectionIsConfigured(kind, settings)) return;
    const first = availableCapabilities[0];
    selectCapability(first);
  }

  function handleProviderChange(value: string) {
    const first = availableCapabilities.find((capability) => capability.providerId === value);
    if (first) {
      selectCapability(first);
    } else {
      providerId = value;
      modelId = "";
      adapter = "";
      parameters = {};
    }
  }

  function handleCapabilityChange(value: string) {
    const next = modelOptions.find((capability) => capabilityKey(capability) === value);
    if (next) selectCapability(next);
  }

  function selectCapability(capability: MediaCapabilityInfo) {
    providerId = capability.providerId;
    modelId = capability.modelId;
    adapter = capability.adapter;
    parameters = { ...capability.defaults };
  }

  function capabilityKey(capability: MediaCapabilityInfo): string {
    return [capability.providerId, capability.modelId, capability.adapter].join("\u0000");
  }

  function modelLabel(capability: MediaCapabilityInfo): string {
    const label = capability.modelDisplayName || capability.modelId;
    return modelOptions.filter((candidate) => candidate.modelId === capability.modelId).length > 1
      ? `${label} (${capability.adapter})`
      : label;
  }

  function parameterValue(parameter: MediaCapabilityParameterInfo): string {
    return parameters[parameter.name] ?? parameter.default;
  }

  function setParameterValue(name: string, value: string) {
    parameters = { ...parameters, [name]: value };
  }

  function normalizeParameters(
    capability: MediaCapabilityInfo,
    current: Record<string, string>
  ): Record<string, string> {
    const next: Record<string, string> = {};
    for (const parameter of capability.parameters) {
      const currentValue = current[parameter.name];
      next[parameter.name] = parameter.values.includes(currentValue)
        ? currentValue
        : parameter.default;
    }
    return next;
  }

  function serializeParameters(value: Record<string, string>): string {
    return Object.entries(value)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, val]) => `${key}=${val}`)
      .join("\u0000");
  }

  function imageDirForSessionCwd(cwd: string): string {
    if (!cwd) return "";
    const base = cwd.replace(/[\\/]+$/, "");
    return base ? `${base}/${IMAGE_OUTPUT_DIR_RELATIVE}` : `/${IMAGE_OUTPUT_DIR_RELATIVE}`;
  }

  function savedSelectionIsConfigured(mediaKind: MediaKind, mediaSettings: MediaSettings): boolean {
    if (mediaKind === "image") {
      const image = mediaSettings.image;
      return Boolean(image.providerId || image.modelId || image.adapter);
    }
    const video = mediaSettings.video;
    return Boolean(video.providerId || video.modelId);
  }

  function savedSelectionIsAvailable(
    mediaKind: MediaKind,
    mediaSettings: MediaSettings,
    available: MediaCapabilityInfo[]
  ): boolean {
    if (mediaKind === "image") {
      const image = mediaSettings.image;
      return available.some(
        (capability) =>
          capability.providerId === image.providerId &&
          capability.modelId === image.modelId &&
          capability.adapter === image.adapter
      );
    }
    const video = mediaSettings.video;
    return available.some(
      (capability) =>
        capability.providerId === video.providerId && capability.modelId === video.modelId
    );
  }

  function withCurrentImage(): ImageMediaSettings {
    const normalizedParameters = selectedCapability
      ? normalizeParameters(selectedCapability, parameters)
      : { ...parameters };
    return {
      providerId: providerId || null,
      modelId: modelId || null,
      adapter: adapter || null,
      parameters: normalizedParameters
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

  async function openImageDir() {
    openError = null;
    try {
      await invoke("open_image_dir", { cwd: sessionCwd });
    } catch (openDirError) {
      openError = String(openDirError);
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

  $effect(() => {
    if (kind !== "image" || !selectedCapability) return;
    const next = normalizeParameters(selectedCapability, parameters);
    if (serializeParameters(next) !== serializeParameters(parameters)) {
      parameters = next;
    }
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
              {#if providerId && !providerOptions.some((provider) => provider.id === providerId)}
                <option value={providerId} disabled>{providerId} unavailable</option>
              {/if}
              {#each providerOptions as provider}
                <option value={provider.id}>{provider.label}</option>
              {/each}
            </select>
          </label>

          <label class="pf-media-field">
            <span class="pf-field-label">Model</span>
            <select
              class="sc-input"
              value={selectedCapability ? capabilityKey(selectedCapability) : ""}
              onchange={(event) => handleCapabilityChange(event.currentTarget.value)}
            >
              {#if modelId && !modelOptions.some((capability) => capability.modelId === modelId && capability.adapter === adapter)}
                <option value={modelId} disabled>{modelId} unavailable</option>
              {/if}
              {#each modelOptions as capability}
                <option value={capabilityKey(capability)}>{modelLabel(capability)}</option>
              {/each}
            </select>
          </label>

          {#if kind === "image"}
            {#if selectedCapability}
              {#each selectedCapability.parameters as parameter (parameter.name)}
                <label class="pf-media-field">
                  <span class="pf-field-label">{parameter.label}</span>
                  <select
                    class="sc-input"
                    value={parameterValue(parameter)}
                    onchange={(event) => setParameterValue(parameter.name, event.currentTarget.value)}
                  >
                    {#each parameter.values as option}
                      <option value={option}>{option}</option>
                    {/each}
                  </select>
                </label>
              {/each}
            {/if}
            {#if imageDir}
              <div class="pf-media-field">
                <span id="pf-image-folder-label" class="pf-field-label">Image folder</span>
                <div class="pf-media-path-row">
                  <input
                    class="sc-input"
                    type="text"
                    aria-labelledby="pf-image-folder-label"
                    readonly
                    value={imageDir}
                  />
                  <button
                    type="button"
                    class="sc-btn"
                    data-variant="outline"
                    onclick={openImageDir}
                  >
                    Open folder
                  </button>
                </div>
              </div>
              {#if openError}
                <p class="pf-media-open-error" role="alert">{openError}</p>
              {/if}
            {/if}
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
          {saving ? "Saving..." : saveLabel}
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

  .pf-media-path-row {
    display: flex;
    min-width: 0;
    gap: 8px;
  }

  .pf-media-path-row input {
    flex: 1 1 auto;
    min-width: 0;
  }

  .pf-media-path-row button {
    flex: 0 0 auto;
    white-space: nowrap;
  }

  .pf-media-open-error {
    margin: -2px 0 0;
    border: 1px solid color-mix(in oklab, var(--destructive) 30%, var(--border));
    border-radius: 8px;
    padding: 8px 10px;
    color: var(--foreground);
    background: color-mix(in oklab, var(--destructive) 8%, var(--background));
    font-size: 12px;
    line-height: 1.4;
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
