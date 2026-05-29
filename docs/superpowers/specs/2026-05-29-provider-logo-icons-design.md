# Provider Logo Icons Design

## Problem

Desktop provider cards currently use `providerVisuals.ts` to select a card icon
and accent. Only OpenAI and Vercel AI Gateway map to brand assets. The remaining
known providers mostly fall back to generic `ai.svg` or `llm.svg`, including the
currently visible Puffer, Codex, and Claude cards.

## Provider Coverage

The implementation must cover every provider declared in `resources/providers`:

- `anthropic`
- `cerebras`
- `groq`
- `kimi-coding`
- `kimi-openai`
- `llama-cpp`
- `lmstudio`
- `minimax-cn`
- `minimax`
- `ollama`
- `openai`
- `openrouter`
- `vercel-ai-gateway`
- `vllm`
- `worldrouter`
- `xai`
- `zhipu`

Desktop-native aliases must also be covered because they appear in Settings:

- `puffer`
- `codex`
- `claude`

## Design

Use a layered logo strategy:

1. Use existing or official brand assets when a trustworthy source is already
   available. `puffer` uses `/brand-logo.svg`; `openai` and `codex` use the
   existing OpenAI icon; `vercel-ai-gateway` uses the existing Vercel icon.
2. Use provider-specific local SVGs for known providers without reliable bundled
   assets. These should be compact, legible at 24-36 px, and visually consistent
   with the current quiet Settings UI.
3. Use monogram icons for providers whose official logo source or reuse terms
   are unclear. Monograms are acceptable for Anthropic/Claude, Cerebras, Kimi,
   MiniMax, xAI, and other providers where copying a third-party logo would add
   licensing risk.
4. Keep `ai.svg` and `llm.svg` only as unknown-provider fallbacks, not as visuals
   for known providers.

## Data Flow

`providerVisual(provider)` remains the single lookup point for Settings provider
cards. The provider id selects an explicit icon key and accent. The function
returns a URL under `/service-icons` unless the icon is the existing root
`/brand-logo.svg` asset.

## UI Rules

Provider cards keep the existing 36 px logo container and subdued card styling.
SVGs should render cleanly in light mode, inherit no unexpected surrounding text
color, and avoid high-saturation fills unless they are part of an existing
official mark already in the repository.

## Testing

Add a focused desktop test that reads `resources/providers/*.yaml` and
`providerVisuals.ts`, then asserts every known provider id has an explicit icon
mapping. The same test must include desktop-native aliases `puffer`, `codex`,
and `claude`. Known providers must not map to generic `ai` or `llm` unless they
are deliberately listed as an unknown fallback in the test.

## Compatibility

No provider registry, credential, model, or daemon behavior changes. This is a
desktop visual mapping change only.
