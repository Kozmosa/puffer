<script lang="ts">
  import type { DiffSnapshot } from "../types";
  import HighlightedLine from "./HighlightedLine.svelte";

  type PatchLine = {
    kind: "meta" | "context" | "added" | "removed";
    text: string;
    oldNumber: number | null;
    newNumber: number | null;
  };

  export let diff: DiffSnapshot;
  export let compact = false;

  const compactLineLimit = 40;

  function diffStats(text: string) {
    const lines = text.split("\n");
    let additions = 0;
    let removals = 0;

    for (const line of lines) {
      if (line.startsWith("+") && !line.startsWith("+++")) {
        additions += 1;
      } else if (line.startsWith("-") && !line.startsWith("---")) {
        removals += 1;
      }
    }

    return { additions, removals };
  }

  function parsePatch(text: string): PatchLine[] {
    const result: PatchLine[] = [];
    const lines = text.split("\n");
    let oldNumber = 0;
    let newNumber = 0;

    for (const line of lines) {
      if (line.startsWith("@@")) {
        const match = /@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
        oldNumber = match ? Number(match[1]) : oldNumber;
        newNumber = match ? Number(match[2]) : newNumber;
        result.push({ kind: "meta", text: line, oldNumber: null, newNumber: null });
        continue;
      }

      if (line.startsWith("+") && !line.startsWith("+++")) {
        result.push({ kind: "added", text: line, oldNumber: null, newNumber });
        newNumber += 1;
        continue;
      }

      if (line.startsWith("-") && !line.startsWith("---")) {
        result.push({ kind: "removed", text: line, oldNumber, newNumber: null });
        oldNumber += 1;
        continue;
      }

      result.push({ kind: "context", text: line || " ", oldNumber, newNumber });
      oldNumber += 1;
      newNumber += 1;
    }

    return result;
  }

  $: stats = diffStats(diff.patch);
  $: patchLines = parsePatch(diff.patch);
  $: displayedLines = compact ? patchLines.slice(0, compactLineLimit) : patchLines;
  $: isTruncated = compact && patchLines.length > compactLineLimit;
</script>

<article class:compact class="diff-view">
  <header class="diff-header">
    <div>
      <h3>{diff.title}</h3>
      <p class="diff-status">{diff.status}</p>
    </div>
    <div class="diff-summary">
      <span class="summary-pill">
        <strong>+{stats.additions}</strong>
        <span>added</span>
      </span>
      <span class="summary-pill removed">
        <strong>-{stats.removals}</strong>
        <span>removed</span>
      </span>
    </div>
  </header>

  <div class="file-header">
    <span class="file-path">{diff.command}</span>
    <span class="file-meta">{diff.unstagedDiffstat || diff.stagedDiffstat || "session diff"}</span>
  </div>

  <div class="patch-shell">
    <div class="patch-lines">
      {#each displayedLines as line}
        <div class={"patch-line " + line.kind}>
          <span class="gutter">{line.oldNumber ?? ""}</span>
          <span class="gutter">{line.newNumber ?? ""}</span>
          <code><HighlightedLine text={line.text} path={diff.title || diff.command} /></code>
        </div>
      {/each}
    </div>
  </div>

  {#if isTruncated}
    <p class="truncation-note">Showing first {compactLineLimit} diff lines.</p>
  {/if}
</article>

<style>
  .diff-view {
    display: grid;
    gap: 0.85rem;
    padding: 1rem 1.1rem 1.2rem;
    min-height: 100%;
    align-content: start;
    background:
      linear-gradient(180deg, rgba(250, 246, 240, 0.98), rgba(242, 235, 225, 0.94)),
      var(--canvas-muted);
  }

  .diff-view.compact {
    padding: 0.8rem 0.9rem;
    min-height: auto;
  }

  .diff-header {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    align-items: start;
    position: sticky;
    top: 0;
    z-index: 2;
    padding-bottom: 0.45rem;
    background:
      linear-gradient(180deg, rgba(250, 246, 240, 0.98), rgba(250, 246, 240, 0.92));
  }

  h3 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 1.26rem;
    line-height: 1.06;
    letter-spacing: -0.03em;
  }

  .diff-status {
    margin: 0.24rem 0 0;
    color: var(--text-soft);
    font-size: 0.8rem;
  }

  .diff-summary {
    display: flex;
    gap: 0.55rem;
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .summary-pill {
    display: grid;
    gap: 0.04rem;
    padding: 0.36rem 0.56rem;
    border-radius: 0;
    background: rgba(46, 160, 67, 0.1);
    color: #1a7f37;
    min-width: 4.5rem;
    text-align: center;
  }

  .summary-pill.removed {
    background: rgba(207, 34, 46, 0.1);
    color: #cf222e;
  }

  .summary-pill strong {
    font-size: 0.84rem;
    line-height: 1;
  }

  .summary-pill span {
    font-size: 0.62rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .file-header {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    align-items: center;
    padding: 0.58rem 0.8rem;
    border-radius: 0;
    background: #f6f8fa;
    box-shadow: 0 0 0 1px #d0d7de inset;
    color: #57606a;
    font-size: 0.76rem;
    position: sticky;
    top: 2.9rem;
    z-index: 2;
  }

  .file-path {
    font-weight: 600;
    color: #24292f;
  }

  .patch-shell {
    overflow: hidden;
    border-radius: 0;
    box-shadow: 0 0 0 1px #d0d7de inset;
    background: #ffffff;
  }

  .patch-lines {
    display: grid;
  }

  .patch-line {
    display: grid;
    grid-template-columns: 3rem 3rem minmax(0, 1fr);
    font-family: var(--font-mono);
    font-size: 0.8rem;
    line-height: 1.5;
  }

  .patch-line + .patch-line {
    box-shadow: 0 -1px 0 rgba(208, 215, 222, 0.7) inset;
  }

  .patch-line.meta {
    background: #f6f8fa;
    color: #57606a;
  }

  .patch-line.added {
    background: #dafbe1;
    color: #1a7f37;
  }

  .patch-line.removed {
    background: #ffebe9;
    color: #cf222e;
  }

  .patch-line.context {
    background: #ffffff;
    color: #24292f;
  }

  .gutter {
    display: grid;
    place-items: center end;
    padding: 0.12rem 0.48rem 0.12rem 0.2rem;
    color: #8c959f;
    user-select: none;
    box-shadow: 1px 0 0 rgba(208, 215, 222, 0.7) inset;
  }

  code {
    display: block;
    padding: 0.12rem 0.75rem;
    overflow-x: auto;
    white-space: pre;
  }

  .truncation-note {
    margin: 0;
    color: var(--text-soft);
    font-size: 0.76rem;
  }
</style>
