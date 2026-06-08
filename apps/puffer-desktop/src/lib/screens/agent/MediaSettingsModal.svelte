<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, untrack } from "svelte";
  import { listMediaCapabilities, updateConfig } from "../../api/desktop";
  import Icon from "../../design/Icon.svelte";
  import { focusTrap } from "../../focusTrap";
  import type {
    MediaCapabilityInfo,
    MediaCapabilityParameterInfo,
    MediaGenerationSettings,
    MediaKind,
    MediaSettings,
    SettingsSnapshot
  } from "../../types";

  type Props = {
    kind: MediaKind;
    sessionCwd: string;
    settings: MediaSettings;
    settingsReady?: boolean;
    onSaved: (snapshot: SettingsSnapshot) => void;
    onClose: () => void;
  };

  let { kind, sessionCwd, settings, settingsReady = true, onSaved, onClose }: Props = $props();
  const IMAGE_OUTPUT_DIR_RELATIVE = ".puffer/media/images";
  const VIDEO_ASPECT_RATIO_PARAMETER_NAMES = ["aspect_ratio", "aspectRatio"];
  const VIDEO_DURATION_PARAMETER_NAMES = ["duration", "duration_seconds", "durationSeconds"];
  const initialSaved = untrack(() => mediaSettingsForKind(kind, settings));
  const initialParameters = untrack(() => initialSaved?.parameters ?? {});

  const title = $derived(mediaSettingsTitle(kind));
  const saveLabel = "Save";
  const closeLabel = $derived(`Close ${title.toLowerCase()}`);
  const saved = $derived(mediaSettingsForKind(kind, settings));
  const imageDir = $derived(imageDirForSessionCwd(sessionCwd));

  let capabilities = $state<MediaCapabilityInfo[]>([]);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let openError = $state<string | null>(null);
  let providerId = $state(initialSaved?.providerId ?? "");
  let modelId = $state(initialSaved?.modelId ?? "");
  let adapter = $state(initialSaved?.adapter ?? "");
  let parameters = $state<Record<string, string>>({ ...initialParameters });
  let aspectRatio = $state(videoAspectRatioFromParameters(initialParameters));
  let durationSeconds = $state(videoDurationFromParameters(initialParameters));
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
  let selectedCapability = $derived(currentMatchingCapability());
  let aspectRatioParameter = $derived(
    findCapabilityParameter(selectedCapability, VIDEO_ASPECT_RATIO_PARAMETER_NAMES)
  );
  let durationParameter = $derived(
    findCapabilityParameter(selectedCapability, VIDEO_DURATION_PARAMETER_NAMES)
  );
  let aspectRatioOptions = $derived(videoStringOptions(aspectRatioParameter, aspectRatio));
  let durationOptions = $derived(videoDurationOptions(durationParameter, durationSeconds));
  let aspectRatioLabel = $derived(aspectRatioParameter?.label ?? "Aspect ratio");
  let durationFieldLabel = $derived(durationParameter?.label ?? "Duration");
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
      image?.providerId ?? "",
      image?.modelId ?? "",
      image?.adapter ?? "",
      serializeParameters(image?.parameters ?? {}),
      video?.providerId ?? "",
      video?.modelId ?? "",
      video?.adapter ?? "",
      serializeParameters(video?.parameters ?? {})
    ].join("\u0000");
  }

  function applySettings(mediaSettings: MediaSettings) {
    const current = mediaSettingsForKind(kind, mediaSettings);
    providerId = current?.providerId ?? "";
    modelId = current?.modelId ?? "";
    adapter = current?.adapter ?? "";
    parameters = { ...(current?.parameters ?? {}) };
    aspectRatio = videoAspectRatioFromParameters(current?.parameters ?? {});
    durationSeconds = videoDurationFromParameters(current?.parameters ?? {});
  }

  function chooseDefaultCapability() {
    if (availableCapabilities.length === 0) return;
    const savedCapability = currentMatchingCapability();
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
    if (capability.kind === "image") {
      parameters = { ...capability.defaults };
    } else {
      parameters = { ...capability.defaults };
      aspectRatio = defaultVideoAspectRatio(capability, aspectRatio);
      durationSeconds = defaultVideoDurationSeconds(capability, durationSeconds);
    }
  }

  function capabilityKey(capability: MediaCapabilityInfo): string {
    return [capability.providerId, capability.modelId, capability.adapter].join("\u0000");
  }

  function currentCapabilityKey(): string {
    return modelId ? [providerId, modelId, adapter].join("\u0000") : "";
  }

  function currentMatchingCapability(): MediaCapabilityInfo | null {
    return (
      availableCapabilities.find((capability) =>
        capabilityMatchesIdentity(capability, kind, providerId, modelId, adapter)
      ) ?? null
    );
  }

  function capabilityMatchesIdentity(
    capability: MediaCapabilityInfo,
    mediaKind: MediaKind,
    selectedProviderId: string | null | undefined,
    selectedModelId: string | null | undefined,
    selectedAdapter: string | null | undefined
  ): boolean {
    if (!selectedProviderId || !selectedModelId) return false;
    if (capability.providerId !== selectedProviderId || capability.modelId !== selectedModelId) {
      return false;
    }
    return capability.adapter === selectedAdapter;
  }

  function providerSelectionIsUnavailable(): boolean {
    return Boolean(providerId && !providerOptions.some((provider) => provider.id === providerId));
  }

  function modelSelectionIsUnavailable(): boolean {
    return Boolean(
      modelId &&
        !modelOptions.some((capability) =>
          capabilityMatchesIdentity(capability, kind, providerId, modelId, adapter)
        )
    );
  }

  function shouldRenderProviderSelect(): boolean {
    return providerOptions.length !== 1 || providerSelectionIsUnavailable();
  }

  function shouldRenderModelSelect(): boolean {
    return modelOptions.length !== 1 || modelSelectionIsUnavailable();
  }

  function readOnlyProviderLabel(): string {
    return (
      providerOptions.find((provider) => provider.id === providerId)?.label ??
      providerOptions[0]?.label ??
      providerId
    );
  }

  function readOnlyModelLabel(): string {
    const capability = selectedCapability ?? modelOptions[0] ?? null;
    return capability ? modelLabel(capability) : modelId;
  }

  function shouldRenderParameterSelect(parameter: MediaCapabilityParameterInfo): boolean {
    return parameter.values.length > 1;
  }

  function shouldRenderAspectRatioSelect(): boolean {
    return aspectRatioOptions.length !== 1;
  }

  function shouldRenderDurationSelect(): boolean {
    return durationOptions.length !== 1;
  }

  function formatDurationLabel(value: number): string {
    return `${value}s`;
  }

  function mediaCapabilitiesLoadingPrimary(mediaKind: MediaKind): string {
    return `Loading ${mediaKind} capabilities...`;
  }

  function mediaCapabilitiesLoadingSecondary(mediaKind: MediaKind): string {
    return mediaKind === "image"
      ? "Checking available image generation models."
      : "Checking available video generation models.";
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

  function findCapabilityParameter(
    capability: MediaCapabilityInfo | null,
    names: string[]
  ): MediaCapabilityParameterInfo | null {
    if (!capability || capability.kind !== "video") return null;
    const normalizedNames = new Set(names.map(normalizeParameterName));
    return (
      capability.parameters.find((parameter) => {
        const name = normalizeParameterName(parameter.name);
        const requestField = parameter.requestField
          ? normalizeParameterName(parameter.requestField)
          : "";
        return normalizedNames.has(name) || normalizedNames.has(requestField);
      }) ?? null
    );
  }

  function normalizeParameterName(value: string): string {
    return value.replace(/[-_\s]/g, "").toLowerCase();
  }

  function videoStringOptions(
    parameter: MediaCapabilityParameterInfo | null,
    currentValue: string
  ): string[] {
    const values =
      parameter?.values.filter((value) => value.trim().length > 0) ??
      [];
    if (values.length > 0) return uniqueStrings(values);
    if (parameter?.default) return [parameter.default];
    return currentValue ? [currentValue] : [];
  }

  function videoDurationOptions(
    parameter: MediaCapabilityParameterInfo | null,
    currentValue: number
  ): number[] {
    const rawValues =
      parameter && parameter.values.length > 0
        ? parameter.values
        : parameter?.default
          ? [parameter.default]
          : [];
    const values = rawValues
      .map(parseDurationSeconds)
      .filter((value): value is number => value !== null);
    if (values.length > 0) return uniqueNumbers(values);
    return Number.isFinite(currentValue) ? [currentValue] : [];
  }

  function uniqueStrings(values: string[]): string[] {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const value of values) {
      if (seen.has(value)) continue;
      seen.add(value);
      out.push(value);
    }
    return out;
  }

  function uniqueNumbers(values: number[]): number[] {
    const seen = new Set<number>();
    const out: number[] = [];
    for (const value of values) {
      if (seen.has(value)) continue;
      seen.add(value);
      out.push(value);
    }
    return out;
  }

  function parseDurationSeconds(value: string): number | null {
    const numeric = Number(value.trim().replace(/s$/i, ""));
    return Number.isFinite(numeric) && numeric > 0 ? numeric : null;
  }

  function defaultVideoAspectRatio(
    capability: MediaCapabilityInfo,
    currentValue: string
  ): string {
    const parameter = findCapabilityParameter(capability, VIDEO_ASPECT_RATIO_PARAMETER_NAMES);
    const options = videoStringOptions(parameter, currentValue);
    const defaultValue = parameter?.default ?? "";
    if (defaultValue && options.includes(defaultValue)) return defaultValue;
    return options[0] ?? currentValue;
  }

  function defaultVideoDurationSeconds(
    capability: MediaCapabilityInfo,
    currentValue: number
  ): number {
    const parameter = findCapabilityParameter(capability, VIDEO_DURATION_PARAMETER_NAMES);
    const options = videoDurationOptions(parameter, currentValue);
    const defaultValue = parameter?.default ? parseDurationSeconds(parameter.default) : null;
    if (defaultValue !== null && options.includes(defaultValue)) return defaultValue;
    return options[0] ?? currentValue;
  }

  function normalizeVideoAspectRatio(value: string): string {
    return aspectRatioOptions.includes(value) ? value : aspectRatioOptions[0] ?? value;
  }

  function normalizeVideoDurationSeconds(value: number): number {
    return durationOptions.includes(value) ? value : durationOptions[0] ?? value;
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
      return Boolean(image);
    }
    const video = mediaSettings.video;
    return Boolean(video);
  }

  function savedSelectionIsAvailable(
    mediaKind: MediaKind,
    mediaSettings: MediaSettings,
    available: MediaCapabilityInfo[]
  ): boolean {
    if (mediaKind === "image") {
      const image = mediaSettings.image;
      if (!image) return false;
      return available.some((capability) =>
        capabilityMatchesIdentity(
          capability,
          mediaKind,
          image.providerId,
          image.modelId,
          image.adapter
        )
      );
    }
    const video = mediaSettings.video;
    if (!video) return false;
    return available.some((capability) =>
      capabilityMatchesIdentity(
        capability,
        mediaKind,
        video.providerId,
        video.modelId,
        video.adapter
      )
    );
  }

  function withCurrentSelection(): MediaGenerationSettings {
    const currentParameters =
      kind === "video" ? videoParametersFromCurrent() : { ...parameters };
    const normalizedParameters = selectedCapability
      ? normalizeParameters(selectedCapability, currentParameters)
      : currentParameters;
    return {
      providerId,
      modelId,
      operation: "generate",
      adapter,
      parameters: normalizedParameters
    };
  }

  function videoParametersFromCurrent(): Record<string, string> {
    const next = { ...parameters };
    const aspectName = aspectRatioParameter?.name ?? "aspect_ratio";
    const durationName = durationParameter?.name ?? "duration";
    next[aspectName] = normalizeVideoAspectRatio(aspectRatio);
    next[durationName] = String(normalizeVideoDurationSeconds(durationSeconds));
    return next;
  }

  function videoAspectRatioFromParameters(value: Record<string, string>): string {
    return value.aspect_ratio ?? value.aspectRatio ?? "16:9";
  }

  function videoDurationFromParameters(value: Record<string, string>): number {
    const duration = value.duration ?? value.duration_seconds ?? value.durationSeconds ?? "8";
    return parseDurationSeconds(duration) ?? 8;
  }

  async function save() {
    if (!canSave) return;
    saving = true;
    error = null;
    try {
      const media: MediaSettings =
        kind === "image"
          ? { image: withCurrentSelection(), video: settings.video }
          : { image: settings.image, video: withCurrentSelection() };
      const snapshot = await updateConfig({ media });
      onSaved(snapshot);
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

  $effect(() => {
    if (kind !== "video" || !selectedCapability) return;
    const nextAspectRatio = normalizeVideoAspectRatio(aspectRatio);
    const nextDurationSeconds = normalizeVideoDurationSeconds(durationSeconds);
    if (nextAspectRatio !== aspectRatio) {
      aspectRatio = nextAspectRatio;
    }
    if (nextDurationSeconds !== durationSeconds) {
      durationSeconds = nextDurationSeconds;
    }
  });
</script>

{#snippet loadingBlock(primary: string, secondary: string)}
  <div class="pf-media-loading" role="status" aria-live="polite">
    <span class="pf-media-loading-spinner" aria-hidden="true"></span>
    <div>
      <strong>{primary}</strong>
      <span>{secondary}</span>
    </div>
  </div>
{/snippet}

{#snippet readOnlyField(label: string, value: string)}
  <div class="pf-media-field">
    <span class="pf-field-label">{label}</span>
    <div class="pf-media-readonly-value">{value}</div>
  </div>
{/snippet}

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
        {@render loadingBlock(mediaCapabilitiesLoadingPrimary(kind), mediaCapabilitiesLoadingSecondary(kind))}
      {:else if loading}
        {@render loadingBlock(mediaCapabilitiesLoadingPrimary(kind), mediaCapabilitiesLoadingSecondary(kind))}
      {:else if error}
        <p class="pf-media-state" data-warning="true" role="alert">{error}</p>
      {:else if !hasAvailableCapabilities}
        <p class="pf-media-state">No {kind} capabilities available.</p>
      {:else}
        {#if savedSelectionMissing}
          <p class="pf-media-state" data-warning="true" role="alert">Saved model is no longer available.</p>
        {/if}

        <div class="pf-media-form-grid">
          {#if shouldRenderProviderSelect()}
            <label class="pf-media-field">
              <span class="pf-field-label">Provider</span>
              <select
                class="sc-input"
                value={providerId}
                onchange={(event) => handleProviderChange(event.currentTarget.value)}
              >
                {#if providerSelectionIsUnavailable()}
                  <option value={providerId} disabled>{providerId} unavailable</option>
                {/if}
                {#each providerOptions as provider}
                  <option value={provider.id}>{provider.label}</option>
                {/each}
              </select>
            </label>
          {:else}
            {@render readOnlyField("Provider", readOnlyProviderLabel())}
          {/if}

          {#if shouldRenderModelSelect()}
            <label class="pf-media-field">
              <span class="pf-field-label">Model</span>
              <select
                class="sc-input"
                value={selectedCapability
                  ? capabilityKey(selectedCapability)
                  : currentCapabilityKey()}
                onchange={(event) => handleCapabilityChange(event.currentTarget.value)}
              >
                {#if modelSelectionIsUnavailable()}
                  <option value={currentCapabilityKey()} disabled>{modelId} unavailable</option>
                {/if}
                {#each modelOptions as capability}
                  <option value={capabilityKey(capability)}>{modelLabel(capability)}</option>
                {/each}
              </select>
            </label>
          {:else}
            {@render readOnlyField("Model", readOnlyModelLabel())}
          {/if}

          {#if kind === "image"}
            {#if selectedCapability}
              {#each selectedCapability.parameters as parameter (parameter.name)}
                {#if shouldRenderParameterSelect(parameter)}
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
                {:else}
                  {@render readOnlyField(parameter.label, parameterValue(parameter))}
                {/if}
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
            {#if shouldRenderAspectRatioSelect()}
              <label class="pf-media-field">
                <span class="pf-field-label">{aspectRatioLabel}</span>
                <select
                  class="sc-input"
                  value={aspectRatio}
                  onchange={(event) => (aspectRatio = event.currentTarget.value)}
                >
                  {#each aspectRatioOptions as option}
                    <option value={option}>{option}</option>
                  {/each}
                </select>
              </label>
            {:else}
              {@render readOnlyField(aspectRatioLabel, aspectRatio)}
            {/if}
            {#if shouldRenderDurationSelect()}
              <label class="pf-media-field">
                <span class="pf-field-label">{durationFieldLabel}</span>
                <select
                  class="sc-input"
                  value={String(durationSeconds)}
                  onchange={(event) => {
                    const next = parseDurationSeconds(event.currentTarget.value);
                    if (next !== null) durationSeconds = next;
                  }}
                >
                  {#each durationOptions as option}
                    <option value={String(option)}>{formatDurationLabel(option)}</option>
                  {/each}
                </select>
              </label>
            {:else}
              {@render readOnlyField(durationFieldLabel, formatDurationLabel(durationSeconds))}
            {/if}
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

  .pf-media-readonly-value {
    min-height: 36px;
    display: flex;
    align-items: center;
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0 10px;
    color: var(--foreground);
    background: color-mix(in oklab, var(--muted) 18%, var(--background));
    font-size: 12px;
    line-height: 1.4;
    overflow-wrap: anywhere;
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

  .pf-media-loading {
    min-height: 180px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 24px;
    color: var(--muted-foreground);
    background: color-mix(in oklab, var(--muted) 20%, var(--background));
  }

  .pf-media-loading strong,
  .pf-media-loading span {
    display: block;
  }

  .pf-media-loading strong {
    color: var(--foreground);
    font-size: 13px;
    font-weight: 600;
  }

  .pf-media-loading span {
    margin-top: 2px;
    font-size: 12px;
    line-height: 1.4;
  }

  .pf-media-loading-spinner {
    width: 18px;
    height: 18px;
    flex: 0 0 auto;
    border: 2px solid color-mix(in oklab, var(--muted-foreground) 22%, transparent);
    border-top-color: var(--muted-foreground);
    border-radius: 50%;
    animation: pf-media-spin 0.8s linear infinite;
  }

  @keyframes pf-media-spin {
    to {
      transform: rotate(360deg);
    }
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
