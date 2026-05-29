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

Use official provider brand marks, sourced from the MIT-licensed
[lobehub/lobe-icons](https://github.com/lobehub/lobe-icons) collection
(`packages/static-svg/icons`) and vendored as static assets under
`apps/puffer-desktop/public/service-icons/`:

1. Reuse the existing bundled `puffer` brand asset (`/brand-logo.svg`) for the
   `puffer` native alias, and keep the existing in-repo `openai.svg` and
   `vercel.svg` assets.
2. For every other known provider, vendor the corresponding lobe-icons SVG
   (`-color.svg` variant when available, otherwise the mono `.svg`). `codex`
   gets its own dedicated brand mark distinct from `openai`.
3. Keep `ai.svg` and `llm.svg` only as unknown-provider fallbacks, not as
   visuals for known providers.

The vendored asset set covers: `anthropic` (Claude mark), `cerebras`, `codex`,
`groq`, `kimi`, `lmstudio`, `minimax`, `ollama`, `openrouter`, `vllm`,
`worldrouter`, `xai` (Grok mark), and `zhipu`. The `llama-cpp` provider has no
official lobe-icons asset and keeps a small local monogram (`L.cpp`).

## Non-Goals

Do not add provider registry behavior, config behavior, credential behavior, or
model discovery behavior. Do not redesign the provider card layout. Do not
generate or trace new brand marks beyond the lobe-icons set; if a provider lacks
a lobe-icons asset, keep a neutral local monogram rather than fabricating one.

## Data Flow

`providerVisual(provider)` remains the single lookup point for Settings provider
cards. The provider id selects an explicit icon key and accent. The function can
return either a `/service-icons/*.svg` URL or the existing root
`/brand-logo.svg` URL.

## UI Rules

Provider cards keep the existing 36 px logo container and subdued card styling.
Vendored lobe-icons SVGs use a 24x24 viewBox with `width="1em"`/`height="1em"`
and either explicit brand colors or `currentColor`, all of which remain legible
in the existing 36 px logo container.

## Testing

Add a focused desktop test that reads `resources/providers/*.yaml` and
`providerVisuals.ts`, then asserts every known provider id has an explicit icon
mapping. The same test must include desktop-native aliases `puffer`, `codex`,
and `claude`. Known providers must not map to generic `ai` or `llm` unless they
are deliberately listed as unknown fallbacks in the test. The test should also
assert each mapped local SVG exists.

## Compatibility

No provider registry, credential, model, or daemon behavior changes. This is a
desktop visual mapping change only.
