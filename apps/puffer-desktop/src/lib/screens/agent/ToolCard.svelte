<script lang="ts">
  import Icon, { type IconName } from "../../design/Icon.svelte";
  import HighlightedLine from "../../components/HighlightedLine.svelte";
  import type { ToolTimelineItem } from "../../types";

  type Props = { item: ToolTimelineItem };
  let { item }: Props = $props();
  type RenderRow = { kind: "ctx" | "add" | "del" | "omit"; line: number | null; text: string };
  type FileRender = { mode: "read" | "diff"; path: string; rows: RenderRow[] };
  type BashRender = { mode: "bash"; stdout: string; stderr: string; meta: string[] };
  type ListRender = { mode: "list"; title: string; meta: string[]; rows: string[]; body?: string | null; hint?: string | null };
  type WebRender = { mode: "web"; title: string; meta: string[]; body: string };
  type ToolRender = FileRender | BashRender | ListRender | WebRender;

  function iconFor(name: string | null | undefined): IconName {
    if (!name) return "bolt";
    const t = name.toLowerCase();
    if (t.includes("edit") || t.includes("write")) return "edit";
    if (t.includes("read") || t.includes("view")) return "file";
    if (t.includes("grep") || t.includes("search")) return "search";
    if (t.includes("bash") || t.includes("shell") || t.includes("exec")) return "terminal";
    if (t.includes("fetch") || t.includes("web")) return "globe";
    if (t.includes("git") || t.includes("diff")) return "git";
    if (t.includes("list") || t.includes("ls")) return "folder";
    return "bolt";
  }

  function argLine(input: string): string {
    const first = input.split("\n").find((l) => l.trim().length > 0) ?? "";
    const trimmed = first.trim();
    if (trimmed.startsWith("{")) {
      try {
        const obj = JSON.parse(input) as Record<string, unknown>;
        const single =
          obj.path ?? obj.file_path ?? obj.pattern ?? obj.command ??
          obj.query ?? obj.url ?? obj.cwd ?? obj.regex ?? obj.prompt ?? null;
        if (typeof single === "string" && single) return single;
        const arr =
          (obj.paths as unknown) ?? (obj.files as unknown) ??
          (obj.globs as unknown) ?? (obj.urls as unknown) ?? null;
        if (Array.isArray(arr) && arr.every((x) => typeof x === "string")) {
          if (arr.length === 0) return "—";
          if (arr.length === 1) return arr[0] as string;
          return `${arr[0]}  +${arr.length - 1}`;
        }
      } catch {
        /* fall through */
      }
    }
    return trimmed;
  }

  function statusLabel(s: string): string {
    const n = s.toLowerCase();
    if (n.includes("run") || n === "pending") return "running";
    if (n.includes("err") || n.includes("fail")) return "failed";
    return "done";
  }

  function asRecord(value: unknown): Record<string, unknown> | null {
    return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : null;
  }

  function parseJsonObject(text: string): Record<string, unknown> | null {
    try {
      return asRecord(JSON.parse(text));
    } catch {
      return null;
    }
  }

  function stringField(obj: Record<string, unknown> | null, names: string[]): string | null {
    if (!obj) return null;
    for (const name of names) {
      const value = obj[name];
      if (typeof value === "string" && value.length > 0) return value;
    }
    return null;
  }

  function numberField(obj: Record<string, unknown> | null, names: string[]): number | null {
    if (!obj) return null;
    for (const name of names) {
      const value = obj[name];
      if (typeof value === "number" && Number.isFinite(value)) return value;
    }
    return null;
  }

  function boolField(obj: Record<string, unknown> | null, names: string[]): boolean | null {
    if (!obj) return null;
    for (const name of names) {
      const value = obj[name];
      if (typeof value === "boolean") return value;
    }
    return null;
  }

  function stringArrayField(obj: Record<string, unknown> | null, names: string[]): string[] {
    if (!obj) return [];
    for (const name of names) {
      const value = obj[name];
      if (Array.isArray(value)) return value.filter((item): item is string => typeof item === "string");
    }
    return [];
  }

  function recordField(obj: Record<string, unknown> | null, name: string): Record<string, unknown> | null {
    return asRecord(obj?.[name]);
  }

  function compactRows(rows: RenderRow[], limit = 140): RenderRow[] {
    if (rows.length <= limit) return rows;
    const head = Math.floor(limit * 0.65);
    const tail = limit - head;
    return [
      ...rows.slice(0, head),
      { kind: "omit", line: null, text: `${rows.length - limit} unchanged lines omitted` },
      ...rows.slice(rows.length - tail)
    ];
  }

  function readRows(content: string, startLine = 1): RenderRow[] {
    return compactRows(
      content.split("\n").map((line, index) => ({
        kind: "ctx",
        line: startLine + index,
        text: line
      })),
      120
    );
  }

  function diffRows(oldText: string | null, newText: string): RenderRow[] {
    const oldLines = oldText === null ? [] : oldText.split("\n");
    const newLines = newText.split("\n");
    if (oldText === null || oldLines.length === 0) {
      return compactRows(
        newLines.map((line, index) => ({ kind: "add", line: index + 1, text: line })),
        160
      );
    }

    let prefix = 0;
    while (
      prefix < oldLines.length &&
      prefix < newLines.length &&
      oldLines[prefix] === newLines[prefix]
    ) {
      prefix += 1;
    }
    let suffix = 0;
    while (
      suffix < oldLines.length - prefix &&
      suffix < newLines.length - prefix &&
      oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
    ) {
      suffix += 1;
    }

    const context = 3;
    const start = Math.max(0, prefix - context);
    const oldEnd = Math.min(oldLines.length, oldLines.length - suffix + context);
    const newEnd = Math.min(newLines.length, newLines.length - suffix + context);
    const rows: RenderRow[] = [];
    if (start > 0) {
      rows.push({ kind: "omit", line: null, text: `${start} unchanged lines omitted` });
    }
    for (let i = start; i < prefix; i += 1) {
      rows.push({ kind: "ctx", line: i + 1, text: oldLines[i] });
    }
    for (let i = prefix; i < oldLines.length - suffix; i += 1) {
      rows.push({ kind: "del", line: null, text: oldLines[i] });
    }
    for (let i = prefix; i < newLines.length - suffix; i += 1) {
      rows.push({ kind: "add", line: i + 1, text: newLines[i] });
    }
    for (let i = newLines.length - suffix; i < newEnd; i += 1) {
      rows.push({ kind: "ctx", line: i + 1, text: newLines[i] });
    }
    const omittedTail = newLines.length - newEnd;
    if (omittedTail > 0) {
      rows.push({ kind: "omit", line: null, text: `${omittedTail} unchanged lines omitted` });
    }
    return compactRows(rows);
  }

  function fileRenderFor(
    name: string,
    input: Record<string, unknown> | null,
    output: Record<string, unknown> | null
  ): FileRender | null {
    const normalized = name.toLowerCase();
    const file = recordField(output, "file");
    const path = stringField(input, ["file_path", "path"]) ?? stringField(output, ["filePath", "path"]) ?? stringField(file, ["filePath", "path"]) ?? "";
    if (normalized === "read" || normalized === "read_file") {
      const content = stringField(output, ["content"]) ?? stringField(file, ["content"]) ?? "";
      if (content) {
        return { mode: "read", path, rows: readRows(content, numberField(file, ["startLine"]) ?? 1) };
      }
      if (stringField(output, ["type"]) === "file_unchanged" || file) {
        const message = stringField(output, ["type"]) === "file_unchanged"
          ? "File unchanged since previous read."
          : "Binary or structured file read. Preview is not available in the transcript.";
        return { mode: "read", path, rows: [{ kind: "ctx", line: null, text: message }] };
      }
      return null;
    }
    if (normalized === "write" || normalized === "write_file") {
      const content = stringField(input, ["content", "contents"]) ?? stringField(output, ["content"]) ?? "";
      if (!content) return null;
      const original = stringField(output, ["originalFile", "original_file"]);
      return { mode: "diff", path, rows: diffRows(original, content) };
    }
    if (normalized === "edit" || normalized === "edit_file" || normalized === "replace_in_file") {
      const oldText = stringField(input, ["old", "old_string", "oldText"]);
      const newText = stringField(input, ["new", "new_string", "newText"]);
      if (oldText === null || newText === null) return null;
      return { mode: "diff", path, rows: diffRows(oldText, newText) };
    }
    return null;
  }

  function bashRenderFor(name: string, output: Record<string, unknown> | null): BashRender | null {
    const normalized = name.toLowerCase();
    if (normalized !== "bash" && normalized !== "powershell" && normalized !== "shell") return null;
    if (!output) return null;
    const stdout = stringField(output, ["stdout"]) ?? "";
    const stderr = stringField(output, ["stderr"]) ?? "";
    const meta = [
      boolField(output, ["interrupted"]) ? "interrupted" : null,
      stringField(output, ["backgroundTaskId", "background_task_id"]),
      stringField(output, ["outputFile", "output_file"]),
      boolField(output, ["noOutputExpected", "no_output_expected"]) ? "no output" : null
    ].filter((value): value is string => Boolean(value));
    if (!stdout && !stderr && meta.length === 0) return null;
    return { mode: "bash", stdout, stderr, meta };
  }

  function listRenderFor(name: string, input: Record<string, unknown> | null, output: Record<string, unknown> | null): ListRender | null {
    const normalized = name.toLowerCase();
    if (normalized === "toolsearch") {
      return { mode: "list", title: stringField(input, ["query"]) ?? "Tool search", meta: [], rows: [], body: toolOutput };
    }
    if (normalized === "skill") {
      return { mode: "list", title: stringField(input, ["skill"]) ?? "Skill", meta: [], rows: [], body: toolOutput };
    }
    if (!output) return null;
    if (normalized === "glob") {
      const rows = stringArrayField(output, ["filenames"]);
      return {
        mode: "list",
        title: stringField(input, ["pattern"]) ?? "Glob results",
        meta: [`${numberField(output, ["numFiles"]) ?? rows.length} files`, `${numberField(output, ["durationMs"]) ?? 0}ms`],
        rows,
        hint: stringField(output, ["hint"])
      };
    }
    if (normalized === "grep") {
      const mode = stringField(output, ["mode"]) ?? "results";
      const content = stringField(output, ["content"]);
      const rows = content ? content.split("\n").filter(Boolean) : stringArrayField(output, ["filenames"]);
      const meta = [
        mode,
        `${numberField(output, ["numFiles"]) ?? stringArrayField(output, ["filenames"]).length} files`,
        numberField(output, ["numLines"]) !== null ? `${numberField(output, ["numLines"])} lines` : null,
        numberField(output, ["numMatches"]) !== null ? `${numberField(output, ["numMatches"])} matches` : null
      ].filter((value): value is string => Boolean(value));
      return { mode: "list", title: stringField(input, ["pattern"]) ?? "Grep results", meta, rows };
    }
    return null;
  }

  function webRenderFor(name: string, input: Record<string, unknown> | null, output: Record<string, unknown> | null): WebRender | null {
    if (!output) return null;
    const normalized = name.toLowerCase();
    if (normalized !== "webfetch" && normalized !== "websearch") return null;
    const body = stringField(output, ["result", "content", "text"]) ?? toolOutput;
    const url = stringField(output, ["url"]) ?? stringField(input, ["url", "query"]) ?? name;
    const meta = [
      numberField(output, ["code"]) !== null ? `${numberField(output, ["code"])} ${stringField(output, ["codeText"]) ?? ""}`.trim() : null,
      numberField(output, ["bytes"]) !== null ? `${numberField(output, ["bytes"])} bytes` : null,
      numberField(output, ["durationMs"]) !== null ? `${numberField(output, ["durationMs"])}ms` : null
    ].filter((value): value is string => Boolean(value));
    return { mode: "web", title: url, meta, body };
  }

  function renderFor(
    name: string,
    input: Record<string, unknown> | null,
    output: Record<string, unknown> | null
  ): ToolRender | null {
    return fileRenderFor(name, input, output)
      ?? bashRenderFor(name, output)
      ?? listRenderFor(name, input, output)
      ?? webRenderFor(name, input, output);
  }

  // Auto-collapse threshold for long terminal-style outputs.
  const AUTO_COLLAPSE_LINE_THRESHOLD = 8;

  let toolName = $derived(
    item.toolName && item.toolName !== "undefined" ? item.toolName : "Tool"
  );
  let toolInput = $derived(item.input ?? item.body ?? "");
  let toolOutput = $derived(item.output ?? "");
  let inputJson = $derived(item.inputJson ?? parseJsonObject(toolInput));
  let outputJson = $derived(parseJsonObject(toolOutput));
  let toolRender = $derived(renderFor(toolName, inputJson, outputJson));
  let toolStatus = $derived(item.status ?? "");
  let allLines = $derived(toolOutput.split("\n"));
  let nonEmptyLines = $derived(allLines.filter((l) => l.length > 0));
  let totalLines = $derived(nonEmptyLines.length);
  let hasOutput = $derived(totalLines > 0 || toolRender !== null);
  let isPending = $derived(
    toolStatus.toLowerCase().startsWith("run") || toolStatus === "pending"
  );
  let isLarge = $derived(totalLines > AUTO_COLLAPSE_LINE_THRESHOLD);

  // Per-card collapse state — seeded from content size; user can override.
  // Pending cards render expanded so the placeholder stays visible; they
  // re-seed to the size-based default as soon as output arrives.
  let collapsed = $state(false);
  $effect(() => {
    collapsed = isPending ? false : isLarge;
  });

  let visibleLines = $derived(nonEmptyLines);
  let toggleable = $derived(hasOutput);

  let arg = $derived(argLine(toolInput));
  let status = $derived(statusLabel(toolStatus));
</script>

<div class="pf-tool" data-collapsed={collapsed} data-pending={isPending}>
  <button
    type="button"
    class="pf-tool-head"
    onclick={() => (toggleable ? (collapsed = !collapsed) : undefined)}
    aria-expanded={toggleable ? !collapsed : undefined}
    aria-label={toggleable ? (collapsed ? "Expand tool output" : "Collapse tool output") : undefined}
    disabled={!toggleable}
  >
    <span class="pf-tool-icon"><Icon name={iconFor(toolName)} size={13} /></span>
    <span class="pf-tool-name">{toolName}</span>
    <span class="pf-tool-arg" title={arg}>{arg}</span>
    <span class="pf-tool-status" data-state={status}>
      <span class="dot"></span>{status}
    </span>
    {#if toggleable}
      <span class="pf-tool-chevron" aria-hidden="true">
        <Icon name={collapsed ? "chevR" : "chevD"} size={11} />
      </span>
    {/if}
  </button>
  {#if hasOutput && !collapsed}
    <div class="pf-tool-body">
      {#if toolRender?.mode === "read" || toolRender?.mode === "diff"}
        <div class="pf-file-render" data-mode={toolRender.mode}>
          {#if toolRender.path}
            <div class="pf-file-path" title={toolRender.path}>{toolRender.path}</div>
          {/if}
          <div class="pf-file-lines">
            {#each toolRender.rows as row, i (i)}
              <div class="pf-file-row {row.kind}">
                <span class="gutter">{row.line ?? ""}</span>
                <span class="mark">{row.kind === "add" ? "+" : row.kind === "del" ? "-" : row.kind === "omit" ? "…" : ""}</span>
                <span class="code"><HighlightedLine text={row.text || " "} path={toolRender.path} /></span>
              </div>
            {/each}
          </div>
        </div>
      {:else if toolRender?.mode === "bash"}
        <div class="pf-structured-render">
          {#if toolRender.meta.length}
            <div class="pf-render-meta">{toolRender.meta.join(" · ")}</div>
          {/if}
          {#if toolRender.stdout}
            <div class="pf-render-section">
              <div class="pf-render-label">stdout</div>
              <pre>{toolRender.stdout}</pre>
            </div>
          {/if}
          {#if toolRender.stderr}
            <div class="pf-render-section danger">
              <div class="pf-render-label">stderr</div>
              <pre>{toolRender.stderr}</pre>
            </div>
          {/if}
        </div>
      {:else if toolRender?.mode === "list"}
        <div class="pf-structured-render">
          <div class="pf-render-title">{toolRender.title}</div>
          {#if toolRender.meta.length}
            <div class="pf-render-meta">{toolRender.meta.join(" · ")}</div>
          {/if}
          {#if toolRender.body}
            <pre>{toolRender.body}</pre>
          {:else if toolRender.rows.length}
            <div class="pf-result-list">
              {#each toolRender.rows as row, i (i)}
                <div class="pf-result-row">{row}</div>
              {/each}
            </div>
          {:else}
            <div class="pf-render-empty">No results</div>
          {/if}
          {#if toolRender.hint}
            <div class="pf-render-hint">{toolRender.hint}</div>
          {/if}
        </div>
      {:else if toolRender?.mode === "web"}
        <div class="pf-structured-render">
          <div class="pf-render-title">{toolRender.title}</div>
          {#if toolRender.meta.length}
            <div class="pf-render-meta">{toolRender.meta.join(" · ")}</div>
          {/if}
          <pre>{toolRender.body}</pre>
        </div>
      {:else}
        <div class="terminal">
          {#each visibleLines as line, i (i)}
            <div class:dim={line.trim().startsWith("//") || line.trim().startsWith("#")}>{line}</div>
          {/each}
        </div>
      {/if}
    </div>
  {:else if isPending}
    <div class="pf-tool-body pf-tool-pending-body">
      <div class="pf-tool-pending">
        <div class="pf-tool-pending-bar"></div>
        <div class="pf-tool-pending-text">awaiting result…</div>
      </div>
    </div>
  {/if}
</div>

<style>
  .pf-tool-head {
    width: 100%;
    text-align: left;
    background: color-mix(in oklab, var(--muted) 50%, var(--background));
    border: 0;
    font: inherit;
    cursor: pointer;
  }
  .pf-tool-head:disabled { cursor: default; }
  .pf-tool-chevron {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    color: var(--muted-foreground);
    flex-shrink: 0;
    margin-left: 4px;
    transition: transform 120ms;
  }
  .pf-tool-head:hover .pf-tool-chevron {
    color: var(--foreground);
  }
  .pf-tool-more {
    all: unset;
    display: inline-flex;
    margin-top: 4px;
    padding: 2px 8px;
    font-family: var(--font-mono);
    font-size: 11px;
    color: oklch(0.7 0.1 145);
    background: transparent;
    cursor: pointer;
    border-radius: 4px;
  }
  .pf-tool-more:hover {
    background: color-mix(in oklab, oklch(0.7 0.1 145) 12%, transparent);
  }
  .pf-file-render {
    background: var(--background);
    border-top: 1px solid var(--border);
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .pf-file-path {
    padding: 7px 12px;
    border-bottom: 1px solid var(--border);
    color: var(--muted-foreground);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pf-file-lines {
    padding: 6px 0;
    overflow: auto;
  }
  .pf-file-row {
    display: grid;
    grid-template-columns: 48px 18px minmax(0, 1fr);
    min-height: 20px;
    line-height: 20px;
  }
  .pf-file-row .gutter {
    padding-right: 8px;
    text-align: right;
    color: var(--muted-foreground);
    user-select: none;
  }
  .pf-file-row .mark {
    color: var(--muted-foreground);
    user-select: none;
  }
  .pf-file-row .code {
    white-space: pre;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pf-file-row.add {
    background: color-mix(in oklab, oklch(0.72 0.18 145) 12%, transparent);
  }
  .pf-file-row.add .mark {
    color: oklch(0.48 0.16 145);
  }
  .pf-file-row.del {
    background: color-mix(in oklab, var(--destructive) 10%, transparent);
  }
  .pf-file-row.del .mark {
    color: var(--destructive);
  }
  .pf-file-row.omit {
    color: var(--muted-foreground);
    background: color-mix(in oklab, var(--muted) 45%, transparent);
    font-style: italic;
  }
  .pf-structured-render {
    background: var(--background);
    border-top: 1px solid var(--border);
    padding: 10px 12px;
    font-family: var(--font-mono);
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pf-render-title {
    font-family: var(--font-sans);
    font-size: 12px;
    font-weight: 600;
    color: var(--foreground);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pf-render-meta,
  .pf-render-label,
  .pf-render-empty,
  .pf-render-hint {
    font-family: var(--font-sans);
    font-size: 11px;
    color: var(--muted-foreground);
  }
  .pf-render-section {
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .pf-render-section.danger {
    border-color: color-mix(in oklab, var(--destructive) 28%, var(--border));
  }
  .pf-render-label {
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in oklab, var(--muted) 40%, transparent);
  }
  .pf-structured-render pre {
    margin: 0;
    padding: 8px;
    white-space: pre-wrap;
    overflow: auto;
    line-height: 1.45;
  }
  .pf-result-list {
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .pf-result-row {
    padding: 5px 8px;
    white-space: pre;
    overflow: hidden;
    text-overflow: ellipsis;
    border-bottom: 1px solid color-mix(in oklab, var(--border) 70%, transparent);
  }
  .pf-result-row:last-child {
    border-bottom: 0;
  }
  .pf-tool-pending-body {
    background: oklch(0.16 0 0);
    padding: 0;
  }
  .pf-tool-pending {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 14px;
    font-family: var(--font-mono);
  }
  .pf-tool-pending-bar {
    height: 10px;
    border-radius: 3px;
    background: linear-gradient(
      90deg,
      oklch(0.3 0 0) 0%,
      oklch(0.45 0 0) 50%,
      oklch(0.3 0 0) 100%
    );
    background-size: 200% 100%;
    animation: pf-shimmer 1.4s linear infinite;
    width: 62%;
  }
  .pf-tool-pending-text {
    color: oklch(0.7 0 0);
    font-size: 11.5px;
    font-style: italic;
    opacity: 0.85;
  }
  @keyframes pf-shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }
</style>
