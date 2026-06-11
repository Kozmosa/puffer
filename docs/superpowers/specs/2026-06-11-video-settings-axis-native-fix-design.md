# Video Settings Axis-Native Fix Design

Date: 2026-06-11

## Problem

Opening video generation settings in the `Milhous` session can crash with:

```text
undefined is not an object (evaluating 'capability.parameters')
```

The session data is not the cause. The active daemon returns media capabilities
with the current axis-native contract: each video capability carries `axes` and
does not carry the old `parameters` or `defaults` fields. The desktop frontend
still models capabilities as `parameters/defaults`, auto-selects the first
available video capability when no saved video setting exists, and then reads
`capability.parameters`.

This is a protocol split between the desktop UI and the daemon.

## Goals

- Make desktop media settings consume the current daemon capability contract
  directly.
- Remove the old concrete media selection shape from the desktop settings path.
- Persist only logical selections: provider, logical model, and axis selections.
- Keep the fix small enough to land safely without introducing a new framework
  or provider-specific UI.
- Add contract tests that prevent the daemon and desktop UI from drifting again.

## Non-Goals

- Do not support the old `parameters/defaults` capability payload.
- Do not support the old persisted `providerId/modelId/adapter/parameters`
  desktop media selection shape.
- Do not add a shared API crate in this fix.
- Do not redesign the full media-generation runtime.
- Do not introduce provider-specific form components.

## Selected Approach

Use an axis-native desktop UI and logical media settings.

The daemon already exposes typed axes and resolves logical selections into
concrete provider requests server-side. The desktop app should use that model as
the single source of truth instead of translating it back into the old
`parameters/defaults` shape.

This keeps the long-term boundary clean:

- UI owns selection and display of declared axes.
- Config owns stable logical choices.
- Runtime owns concrete provider model, adapter, and request construction.

## Rejected Approaches

### Frontend Compatibility Adapter

Mapping `axes` back into `parameters/defaults` inside `desktop.ts` would fix the
crash quickly, but it would keep the old model alive and still leave save
payloads mismatched with daemon config.

### Shared Desktop API Crate Now

A shared DTO crate would reduce drift, but it is larger than this targeted fix.
This change should first align the protocol and add contract tests. A shared
crate is a separate API-contract cleanup and should be considered only if more
desktop/daemon API drift appears.

## Data Contracts

### Capability

Desktop code should treat this as the only media capability shape:

```ts
type MediaCapabilityInfo = {
  providerId: string;
  providerDisplayName: string;
  modelId: string;
  modelDisplayName: string;
  kind: "image" | "video";
  operation: string;
  axes: MediaCapabilityAxisInfo[];
  status: string;
  source: string;
  reason: string | null;
  checkedAtMs: number;
};
```

`modelId` is the logical model id in this contract. It is acceptable to keep the
field name `modelId` for the daemon payload if that is the existing serialized
name, but desktop implementation should treat it as logical model identity. It
must not treat it as a concrete upstream model id.

```ts
type MediaCapabilityAxisInfo = {
  id: string;
  label: string;
  role: "param" | "selector" | string;
  control: unknown;
  requestField: string | null;
  wireType: "string" | "number" | string;
};
```

### Settings

Desktop code should treat this as the only persisted media setting shape:

```ts
type MediaGenerationSettings = {
  providerId: string;
  logicalModelId: string;
  selections: Record<string, string>;
};
```

The UI may display `modelId` labels from capabilities, but config writes must use
`logicalModelId`. Runtime-only fields such as `adapter`, `operation`, concrete
model ids, and upstream request parameters do not belong in persisted desktop
settings.

## UI Flow

When `MediaSettingsModal` opens:

1. Load `listMediaCapabilities(kind)`.
2. Filter to capabilities for the requested kind with `status === "available"`.
3. Match saved settings by `providerId + logicalModelId`.
4. If no saved setting exists, select the first available capability.
5. Build local selections from axis defaults and valid saved selections.
6. Render controls from axes.
7. Save `{ providerId, logicalModelId, selections }`.

If a saved selection cannot be matched, show the existing "saved model is no
longer available" state and keep Save disabled until the user selects an
available model.

## Axis Rendering

Keep the renderer generic and minimal:

- Enum controls render as a select. A single option renders read-only.
- Bool controls render as a checkbox.
- Range controls render as a bounded number input using min, max, and step.
- Unknown control shapes render read-only with the selected/default value, and
  the modal reports the malformed capability in an error state if a save would
  be unsafe.

Do not expand large ranges into option arrays. This avoids UI stalls if a
provider declares a wide numeric range.

## Selection Normalization

For each axis:

1. Prefer a saved selection if it is valid for the axis control.
2. Otherwise use the axis default.
3. Otherwise use the first enum value, `false` for bool, or the range minimum.

Drop selections for axes that no longer exist. Do not preserve unknown saved
selection keys.

## Error Handling

- A missing or malformed `axes` array is a capability contract error. The modal
  should show a clear error instead of rendering controls.
- Empty available capabilities should continue to show the existing empty or
  connect-provider state.
- Save should be disabled when no valid capability is selected.
- The old `capability.parameters` access path should be removed so this class of
  crash cannot recur.

## Backend Alignment

The CLI daemon and Tauri backend should both expose axis-native capabilities.
There should be no desktop-specific `parameters/defaults` capability DTO.

The daemon's `update_config` path should accept only logical media selections.
If the desktop sends the old concrete shape, it should fail as an invalid API
request rather than silently accepting or translating it.

## Testing

Add focused tests:

- CLI daemon unit test: `list_media_capabilities` returns `axes` and does not
  return `parameters` or `defaults`.
- Tauri backend unit test: `list_media_capabilities` returns the same
  axis-native contract.
- TypeScript unit tests for axis default extraction and saved selection
  normalization.
- Playwright test: fake daemon returns only `axes`; opening video generation
  settings does not crash and can save logical media settings.
- Playwright test: stale logical selections are reported as unavailable and do
  not save until the user chooses an available model.

## Implementation Notes

Keep the edit scope narrow:

- `apps/puffer-desktop/src/lib/types.ts`
- `apps/puffer-desktop/src/lib/screens/agent/MediaSettingsModal.svelte`
- `apps/puffer-desktop/src/lib/api/desktop.ts` only if type aliases or minimal
  validation need adjustment
- `apps/puffer-desktop/tests/support/fakeDaemon.ts`
- focused tests in existing desktop and daemon test files
- backend DTO/tests only where they currently emit or assert old fields

Avoid introducing a large form abstraction. A small helper for axis defaults and
validation is enough if the logic would otherwise clutter the modal.

## Success Criteria

- Opening video generation settings in the `Milhous` session no longer crashes.
- The desktop app no longer references `capability.parameters` or
  `capability.defaults`.
- Saving video settings writes logical selection data only.
- Tests fail if daemon media capabilities regress to the old
  `parameters/defaults` contract.
- No unrelated files are modified.
