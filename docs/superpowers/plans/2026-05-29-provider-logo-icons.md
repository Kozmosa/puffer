# Provider Logo Icons Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure every known desktop provider uses a provider-specific logo icon instead of the generic `ai.svg` or `llm.svg` fallback.

**Architecture:** Keep `providerVisual(provider)` as the only visual lookup path. Add a focused static test, explicit provider id mappings, compact local monogram SVG assets, and one desktop component spec update.

**Tech Stack:** Svelte desktop app, static SVG assets in `apps/puffer-desktop/public/service-icons`, Node-based static tests, existing `npm run check`.

---

## Scope And Guardrails

Implement only the visual mapping described in
`docs/superpowers/specs/2026-05-29-provider-logo-icons-design.md`.

Do not change provider registry loading, credentials, model discovery, Settings
data shape, daemon APIs, or card layout. Do not download or vendor third-party
brand logo assets. Use existing bundled assets where available and local
monogram SVGs for everything else.

## File Map

- Modify `apps/puffer-desktop/package.json`: add a focused provider icon test script.
- Create `apps/puffer-desktop/tests/provider-visuals.test.mjs`: static regression test for known provider mappings and asset existence.
- Modify `apps/puffer-desktop/src/lib/providerVisuals.ts`: map all known provider ids and native aliases explicitly.
- Create local SVG assets in `apps/puffer-desktop/public/service-icons/` for known providers that do not already have a bundled asset.
- Create `specs/puffer-desktop/585.md`: component update spec for provider logo icon coverage.

## Task 1: Failing Coverage Test

**Files:**
- Modify: `apps/puffer-desktop/package.json`
- Create: `apps/puffer-desktop/tests/provider-visuals.test.mjs`

- [ ] **Step 1: Add the test script**

In `apps/puffer-desktop/package.json`, add this script next to the existing
static desktop tests:

```json
"test:provider-visuals": "node tests/provider-visuals.test.mjs",
```

- [ ] **Step 2: Write the failing static test**

Create `apps/puffer-desktop/tests/provider-visuals.test.mjs`:

```javascript
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const testDir = dirname(fileURLToPath(import.meta.url));
const appRoot = resolve(testDir, "..");
const repoRoot = resolve(appRoot, "../..");
const providerVisualsPath = resolve(appRoot, "src/lib/providerVisuals.ts");
const providerDir = resolve(repoRoot, "resources/providers");
const providerVisuals = readFileSync(providerVisualsPath, "utf8");

const nativeAliases = ["puffer", "codex", "claude"];
const resourceProviderIds = readdirSync(providerDir)
  .filter((fileName) => fileName.endsWith(".yaml"))
  .map((fileName) => fileName.replace(/\.yaml$/, ""))
  .sort();

if (resourceProviderIds.length === 0) {
  throw new Error(`No provider resources found in ${providerDir}`);
}

function parseRecord(name) {
  const pattern = new RegExp(`const ${name}: Record<string, string> = \\\\{([\\\\s\\\\S]*?)\\\\n\\\\};`);
  const match = providerVisuals.match(pattern);
  if (!match) throw new Error(`Missing ${name} record`);

  const entries = {};
  for (const line of match[1].split("\\n")) {
    const trimmed = line.trim().replace(/,$/, "");
    if (!trimmed) continue;
    const entry = trimmed.match(/^(?:"([^"]+)"|([a-zA-Z0-9_-]+)):\\s*"([^"]+)"$/);
    if (entry) entries[entry[1] ?? entry[2]] = entry[3];
  }
  return entries;
}

const icons = parseRecord("PROVIDER_ICONS");
const accents = parseRecord("PROVIDER_ACCENTS");
const ids = [...resourceProviderIds, ...nativeAliases];

for (const providerId of ids) {
  if (!Object.hasOwn(icons, providerId)) {
    throw new Error(`Missing explicit provider icon mapping for ${providerId}`);
  }
  if (!Object.hasOwn(accents, providerId)) {
    throw new Error(`Missing explicit provider accent mapping for ${providerId}`);
  }

  const icon = icons[providerId];
  if (icon === "ai" || icon === "llm") {
    throw new Error(`Known provider ${providerId} must not use generic ${icon}.svg`);
  }

  const assetPath = icon === "brand-logo"
    ? resolve(appRoot, "public/brand-logo.svg")
    : resolve(appRoot, `public/service-icons/${icon}.svg`);
  if (!existsSync(assetPath)) {
    throw new Error(`Mapped icon asset for ${providerId} does not exist: ${assetPath}`);
  }
}

console.log(`Verified provider visuals for ${ids.length} known provider ids.`);
```

- [ ] **Step 3: Run the test and confirm it fails**

Run:

```bash
npm run test:provider-visuals
```

Expected: FAIL with missing explicit icon mapping for `puffer` or another known provider.

## Task 2: Add Local Monogram Assets

**Files:**
- Create: `apps/puffer-desktop/public/service-icons/anthropic.svg`
- Create: `apps/puffer-desktop/public/service-icons/cerebras.svg`
- Create: `apps/puffer-desktop/public/service-icons/groq.svg`
- Create: `apps/puffer-desktop/public/service-icons/kimi.svg`
- Create: `apps/puffer-desktop/public/service-icons/llama-cpp.svg`
- Create: `apps/puffer-desktop/public/service-icons/lmstudio.svg`
- Create: `apps/puffer-desktop/public/service-icons/minimax.svg`
- Create: `apps/puffer-desktop/public/service-icons/ollama.svg`
- Create: `apps/puffer-desktop/public/service-icons/openrouter.svg`
- Create: `apps/puffer-desktop/public/service-icons/vllm.svg`
- Create: `apps/puffer-desktop/public/service-icons/worldrouter.svg`
- Create: `apps/puffer-desktop/public/service-icons/xai.svg`
- Create: `apps/puffer-desktop/public/service-icons/zhipu.svg`

- [ ] **Step 1: Create monogram SVGs**

Create compact 24x24 SVG files. Use this exact template and change the
`aria-label`, text, and accent fill per provider:

```xml
<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Anthropic" viewBox="0 0 24 24">
  <rect width="24" height="24" rx="6" fill="#d97706"/>
  <text x="12" y="15.5" text-anchor="middle" font-family="Inter, Arial, sans-serif" font-size="9" font-weight="700" fill="#fff">A</text>
</svg>
```

Use these labels/text/fills:

```text
anthropic.svg: Anthropic, A, #d97706
cerebras.svg: Cerebras, C, #7c3aed
groq.svg: Groq, G, #f97316
kimi.svg: Kimi, K, #0ea5e9
llama-cpp.svg: llama.cpp, L.cpp, #dc2626, font-size 5.4
lmstudio.svg: LM Studio, LM, #1e293b
minimax.svg: MiniMax, MM, #1d4ed8
ollama.svg: Ollama, O, #0f172a
openrouter.svg: OpenRouter, OR, #06b6d4
vllm.svg: vLLM, vLLM, #16a34a, font-size 5.4
worldrouter.svg: WorldRouter, WR, #2563eb
xai.svg: xAI, xAI, #0f172a, font-size 6.2
zhipu.svg: Zhipu, Z, #2563eb
```

- [ ] **Step 2: Verify assets exist**

Run:

```bash
ls apps/puffer-desktop/public/service-icons/{anthropic,cerebras,groq,kimi,llama-cpp,lmstudio,minimax,ollama,openrouter,vllm,worldrouter,xai,zhipu}.svg
```

Expected: all 13 paths print.

## Task 3: Explicit Provider Visual Mapping

**Files:**
- Modify: `apps/puffer-desktop/src/lib/providerVisuals.ts`

- [ ] **Step 1: Update icon and accent records**

Replace `PROVIDER_ACCENTS` and `PROVIDER_ICONS` with:

```typescript
const PROVIDER_ACCENTS: Record<string, string> = {
  anthropic: "#d97706",
  claude: "#d97706",
  openai: "#10a37f",
  codex: "#10a37f",
  puffer: "#180524",
  "anthropic-bedrock": "#d97706",
  "anthropic-vertex": "#d97706",
  cerebras: "#7c3aed",
  groq: "#f97316",
  "kimi-coding": "#0ea5e9",
  "kimi-openai": "#0ea5e9",
  "llama-cpp": "#dc2626",
  lmstudio: "#1e293b",
  "minimax-cn": "#1d4ed8",
  minimax: "#1d4ed8",
  ollama: "#0f172a",
  openrouter: "#06b6d4",
  "vercel-ai-gateway": "#0f172a",
  vllm: "#16a34a",
  worldrouter: "#2563eb",
  xai: "#0f172a",
  zhipu: "#2563eb"
};

const PROVIDER_ICONS: Record<string, string> = {
  anthropic: "anthropic",
  claude: "anthropic",
  openai: "openai",
  codex: "openai",
  puffer: "brand-logo",
  "anthropic-bedrock": "anthropic",
  "anthropic-vertex": "anthropic",
  cerebras: "cerebras",
  groq: "groq",
  "kimi-coding": "kimi",
  "kimi-openai": "kimi",
  "llama-cpp": "llama-cpp",
  lmstudio: "lmstudio",
  "minimax-cn": "minimax",
  minimax: "minimax",
  ollama: "ollama",
  openrouter: "openrouter",
  "vercel-ai-gateway": "vercel",
  vllm: "vllm",
  worldrouter: "worldrouter",
  xai: "xai",
  zhipu: "zhipu"
};
```

- [ ] **Step 2: Update URL construction for root brand logo**

Change `providerVisual` to return `/brand-logo.svg` for the `brand-logo` icon:

```typescript
/** Returns the visual treatment for one provider card. */
export function providerVisual(provider: ProviderSummary): ProviderVisual {
  const icon = PROVIDER_ICONS[provider.id] ?? "ai";
  return {
    accent: PROVIDER_ACCENTS[provider.id] ?? "#475569",
    icon: icon === "brand-logo" ? "/brand-logo.svg" : `${SERVICE_ICON_BASE}/${icon}.svg`
  };
}
```

- [ ] **Step 3: Run the focused test**

Run:

```bash
npm run test:provider-visuals
```

Expected: PASS.

## Task 4: Desktop Spec And Verification

**Files:**
- Create: `specs/puffer-desktop/585.md`

- [ ] **Step 1: Add component update spec**

Create `specs/puffer-desktop/585.md`:

```markdown
# Provider logo icons

## Design

Desktop provider cards now map every known provider id and native alias to a
provider-specific visual. Existing bundled brand assets are reused for Puffer,
OpenAI/Codex, and Vercel AI Gateway. Other known providers use compact local
monogram SVGs so they no longer fall back to generic AI/LLM placeholders.

## Compatibility

This only changes provider card visuals. Provider registry loading,
credentials, model discovery, daemon APIs, and Settings state are unchanged.

## Tests

`npm run test:provider-visuals` verifies every provider in
`resources/providers` plus `puffer`, `codex`, and `claude` has an explicit
non-generic icon mapping and that the mapped SVG asset exists.
```

- [ ] **Step 2: Run desktop checks**

Run:

```bash
npm run test:provider-visuals
npm run check
```

Expected: provider visuals test passes. `npm run check` passes with the existing
Workflows unused CSS warnings only.

- [ ] **Step 3: Inspect in desktop app**

Use agent-browser against the running desktop app:

```bash
agent-browser open http://127.0.0.1:1420/
agent-browser snapshot -i
```

Navigate to Settings > Providers if needed. Then run:

```bash
agent-browser eval "(() => [...document.querySelectorAll('.provider-card')].map(card => ({ name: card.querySelector('.name')?.textContent?.trim(), src: card.querySelector('.logo img')?.getAttribute('src'), accent: getComputedStyle(card).getPropertyValue('--provider-accent').trim() })))()"
```

Expected for the current visible list:

```json
[
  { "name": "Puffer", "src": "/brand-logo.svg", "accent": "#180524" },
  { "name": "Codex", "src": "/service-icons/openai.svg", "accent": "#10a37f" },
  { "name": "Claude", "src": "/service-icons/anthropic.svg", "accent": "#d97706" }
]
```

- [ ] **Step 4: Commit**

Run:

```bash
git add apps/puffer-desktop/package.json apps/puffer-desktop/src/lib/providerVisuals.ts apps/puffer-desktop/tests/provider-visuals.test.mjs apps/puffer-desktop/public/service-icons/*.svg specs/puffer-desktop/585.md
git commit -m "fix(desktop): map provider logo icons"
```
