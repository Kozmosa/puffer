# Video Parameter Schema v1 Design

Date: 2026-06-09

## Status

Approved design direction. This document is a design artifact only; it does not
change implementation behavior.

## Goal

Unify video-provider parameter governance around stable Puffer semantics while
preserving provider-specific request mapping. The target scope is all executable
video providers, with BytePlus Seedance 2.0 as the reference case and
Relaydance as the second provider to prove the abstraction.

The design optimizes for long-term stability, low runtime cost, and a small
schema surface. Backward compatibility with existing saved video settings is not
a requirement.

## Current State

Video capabilities are declared statically under provider `media.video.models`.
Each model exposes select-only string parameters with:

- `name`
- `label`
- `values`
- `default`
- optional `request_field`

Desktop infers video control meaning from parameter names and request fields.
For example, it treats names like `ratio`, `aspect_ratio`, and `size` as the
aspect-ratio control, and names like `duration` or `seconds` as the duration
control. Runtime validation checks only that selected names exist and selected
values are in the declared value list.

BytePlus currently declares `duration`, `ratio`, and `resolution`. Relaydance
declares similar user-facing parameters but maps them to `seconds` and
`metadata.*` request fields. BytePlus also has a provider-specific wire-type
special case that sends `duration` as a JSON number.

## Design

Introduce a video-only parameter schema v1 with canonical Puffer semantic
parameter names:

- `duration_seconds`
- `aspect_ratio`
- `resolution`

Each video parameter descriptor should carry the following information:

- `name`: the canonical Puffer semantic name
- `label`: the display label
- `values`: the supported option list, still select-only
- `default`: one of the declared values
- `request_field`: the provider request field
- `wire_type`: either `string` or `number`, defaulting to `string`

This keeps the existing static capability model and avoids runtime provider
discovery. It also keeps provider adapters responsible for request-body shape,
while moving scalar type conversion out of provider-specific string matching.

## Provider Mapping

BytePlus should express Seedance 2.0 parameters as:

| Puffer name | Request field | Wire type | Notes |
| --- | --- | --- | --- |
| `duration_seconds` | `duration` | `number` | Supports documented second values from `4` through `15`. |
| `aspect_ratio` | `ratio` | `string` | Includes `adaptive` where supported. |
| `resolution` | `resolution` | `string` | Standard supports `480p`, `720p`, `1080p`; fast omits `1080p`. |

Relaydance should express its video parameters as:

| Puffer name | Request field | Wire type | Notes |
| --- | --- | --- | --- |
| `duration_seconds` | `seconds` | `string` | Adapter places it at the top level and keeps the current wire behavior until a provider contract requires numeric seconds. |
| `aspect_ratio` | `metadata.ratio` | `string` | Adapter nests it under `metadata`. |
| `resolution` | `metadata.resolution` | `string` | Model-specific value sets remain static. |

Adapters remain responsible for nesting and endpoint-specific request envelopes.
The shared layer only validates names, values, defaults, and scalar wire types.

## Desktop Behavior

The video settings modal should stop inferring control purpose from loose name
matches. It should render first-class controls from canonical names:

- `aspect_ratio` controls the aspect-ratio select.
- `duration_seconds` controls the duration select.
- `resolution` renders as a normal extra select unless promoted later.

This keeps the UI deterministic and removes the need for aliases such as
`ratio`, `aspect_ratio`, `size`, `duration`, and `seconds`.

Duration values should still be presented as seconds for positive integer
values. v1 should not expose provider-specific non-positive sentinel values
such as smart duration because doing that well requires option-level labels,
which are out of scope for this focused schema.

## Runtime Behavior

Runtime validation should reject:

- unknown parameter names
- values outside the declared option list
- defaults not present in the option list
- unsupported `wire_type` values
- empty `request_field` for parameters used by executable adapters

Runtime request builders should emit selected values in descriptor order and use
`wire_type` for scalar encoding. Omitted `wire_type` means `string`. `number`
values must parse before the request is submitted; parse failures should be
local validation errors, not provider HTTP errors.

## Catalog Governance

Static tests should assert video parameter contracts by semantic name for all
executable video providers:

- BytePlus standard Seedance 2.0 has `480p`, `720p`, and `1080p`.
- BytePlus fast Seedance 2.0 has only `480p` and `720p`.
- Relaydance model-specific resolution boundaries are preserved.
- Seedance 1.5 models keep their shorter duration range where declared.
- Prompt-only video models remain parameterless until reliable parameter
  metadata exists.

The tests should compare canonical semantic names, values, defaults,
`request_field`, and `wire_type`.

## Non-Goals

This design intentionally does not add:

- dynamic provider model or parameter discovery
- a general form DSL
- free-form numeric input
- nested object parameters
- parameter dependency expressions
- option-level display labels
- provider-specific smart-duration sentinels
- image-parameter migration
- automatic conversion of old saved settings

These are deferred unless a concrete provider requirement forces them.

## Risks

The main migration risk is saved settings that still use old provider-native
parameter names. Backward compatibility is out of scope, so those settings can
be treated as unavailable or normalized by selecting a fresh default capability.

The main design risk is expanding `wire_type` too early. The first version
should support only `string` and `number`, because current video providers do
not require booleans, arrays, objects, URLs, or media references for the
settings surface being optimized.

## Success Criteria

- Desktop video settings no longer rely on parameter-name heuristics.
- BytePlus and Relaydance use the same semantic parameter names.
- Provider-specific request fields remain explicit in provider resources.
- BytePlus duration is encoded as a number through descriptor metadata, not an
  adapter-specific field-name check.
- Relaydance request serialization is unchanged unless provider evidence later
  requires numeric `seconds`.
- Capability resolution remains static and does not add network calls.
- Catalog tests catch invalid model-specific option drift before runtime.
