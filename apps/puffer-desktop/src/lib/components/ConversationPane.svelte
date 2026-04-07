<script lang="ts">
  import { tick } from "svelte";
  import type {
    DiffTimelineItem,
    PermissionTimelineItem,
    SessionListItem,
    TimelineItem,
    ToolTimelineItem
  } from "../types";
  import MessageBody from "./MessageBody.svelte";

  export let session: SessionListItem | null = null;
  export let timeline: TimelineItem[] = [];
  export let loading = false;
  export let pendingPermissions: PermissionTimelineItem[] = [];
  export let onSubmitMessage: (message: string) => void = () => {};
  export let onResolvePermission: (permissionId: string, choice: string) => void = () => {};

  let draft = "";
  let collapsedIds = new Set<string>();
  let threadElement: HTMLDivElement | null = null;
  let previousSessionId: string | null = null;

  function transcriptText(item: TimelineItem): string {
    switch (item.kind) {
      case "user":
      case "assistant":
      case "system":
        return item.body;
      case "command":
        return ["```sh", item.body, "```"].join("\n");
      case "tool":
        return [
          item.summary,
          "",
          item.input ? "Input" : "",
          item.input ? "```json" : "",
          item.input ?? "",
          item.input ? "```" : "",
          item.output ? "" : "",
          item.output ? "Output" : "",
          item.output ? "```text" : "",
          item.output ?? "",
          item.output ? "```" : ""
        ]
          .filter(Boolean)
          .join("\n");
      default:
        return item.body;
    }
  }

  function rawText(item: TimelineItem): string {
    return transcriptText(item).replace(/\r\n?/g, "\n");
  }

  function lineCount(item: TimelineItem): number {
    return rawText(item).split("\n").length;
  }

  function shouldCollapse(item: TimelineItem): boolean {
    return item.kind !== "user" && lineCount(item) > 12;
  }

  function previewText(item: TimelineItem): string {
    const lines = rawText(item).split("\n");
    if (lines.length <= 4) {
      return lines.join("\n");
    }
    return [...lines.slice(0, 2), "...", ...lines.slice(-2)].join("\n");
  }

  function isCollapsed(item: TimelineItem): boolean {
    return collapsedIds.has(item.id);
  }

  function toggleCollapsed(item: TimelineItem) {
    const next = new Set(collapsedIds);
    if (next.has(item.id)) {
      next.delete(item.id);
    } else {
      next.add(item.id);
    }
    collapsedIds = next;
  }

  async function submitDraft() {
    const trimmed = draft.trim();
    if (!trimmed) {
      return;
    }
    onSubmitMessage(trimmed);
    draft = "";
    await tick();
    threadElement?.scrollTo({ top: threadElement.scrollHeight, behavior: "smooth" });
  }

  function handleComposerKeydown(event: KeyboardEvent) {
    if (event.key !== "Enter") {
      return;
    }
    if (!(event.metaKey || event.ctrlKey)) {
      return;
    }
    event.preventDefault();
    submitDraft();
  }

  function permissionIcon(choice: string): "allow_once" | "allow_session" | "deny" {
    const normalized = choice.toLowerCase();
    if (normalized.includes("session")) {
      return "allow_session";
    }
    if (normalized.includes("deny")) {
      return "deny";
    }
    return "allow_once";
  }

  function isToolItem(item: TimelineItem): item is ToolTimelineItem {
    return item.kind === "tool";
  }

  function isDiffItem(item: TimelineItem): item is DiffTimelineItem {
    return item.kind === "diff";
  }

  function commandLabel(text: string): string {
    return text.startsWith("/") ? text.slice(1) : text;
  }

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

  function diffPreview(text: string, maxLines = 8): string {
    return text.split("\n").slice(0, maxLines).join("\n");
  }

  $: transcriptItems = timeline.filter((item) => item.kind !== "permission");
  $: {
    const next = new Set(collapsedIds);
    for (const id of Array.from(next)) {
      if (!transcriptItems.some((item) => item.id === id && shouldCollapse(item))) {
        next.delete(id);
      }
    }
    collapsedIds = next;
  }
  $: if (session?.id !== previousSessionId) {
    previousSessionId = session?.id ?? null;
    void tick().then(() => {
      threadElement?.scrollTo({ top: 0, behavior: "auto" });
    });
  }
</script>

<section class="conversation">
  <header class="conversation-header">
    <h2>{session?.displayName ?? session?.title ?? "Select a session"}</h2>
    <p class="session-meta">
      {#if session}
        {session.cwd}
      {:else}
        Choose a conversation from the workspace tree.
      {/if}
    </p>
  </header>

  <div bind:this={threadElement} class="thread">
    {#if loading}
      <p class="state">Loading conversation...</p>
    {:else if !transcriptItems.length}
      <p class="state">No messages in this session yet.</p>
    {:else}
      {#each transcriptItems as item}
        <article class:user={item.kind === "user"} class={"entry " + item.kind}>
          {#if item.kind === "user"}
            <div class="bubble">
              <MessageBody body={item.body} />
            </div>
          {:else if isToolItem(item)}
            <div class="entry-meta">
              <span>{item.toolName} · {item.status}</span>

              {#if shouldCollapse(item)}
                <button class="collapse-toggle" on:click={() => toggleCollapsed(item)}>
                  <svg viewBox="0 0 16 16" aria-hidden="true">
                    <path
                      d={isCollapsed(item) ? "M6 4l4 4-4 4" : "M4 6l4 4 4-4"}
                      fill="none"
                      stroke="currentColor"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="1.4"
                    />
                  </svg>
                  <span>{isCollapsed(item) ? "Expand" : "Collapse"}</span>
                </button>
              {/if}
            </div>

            {#if isCollapsed(item)}
              <pre class="collapsed-preview">{previewText(item)}</pre>
            {:else}
              <div class="tool-log">
                <p class="tool-summary">{item.summary}</p>
                <div class="tool-grid">
                  <div class="tool-section">
                    <span class="tool-label">Input</span>
                    <pre>{item.input}</pre>
                  </div>

                  {#if item.output}
                    <div class="tool-section">
                      <span class="tool-label">Output</span>
                      <pre>{item.output}</pre>
                    </div>
                  {/if}
                </div>
              </div>
            {/if}
          {:else if item.kind === "command"}
            <div class="command-log">
              <code>/{commandLabel(item.body)}</code>
            </div>
          {:else if isDiffItem(item)}
            <div class="entry-meta">
              <span>Diff snapshot</span>
            </div>

            <div class="diff-log">
              <p class="diff-title">{item.diff.title}</p>
              <p class="diff-status">{item.diff.status}</p>
              <div class="diff-stats">
                <span>+{diffStats(item.diff.patch).additions}</span>
                <span>-{diffStats(item.diff.patch).removals}</span>
                <span>{item.diff.command}</span>
              </div>
              <pre>{diffPreview(item.diff.patch)}</pre>
            </div>
          {:else}
            <div class="entry-meta">
              {#if item.kind === "assistant"}
                <span>Assistant</span>
              {:else if item.kind === "system"}
                <span>System</span>
              {/if}

              {#if shouldCollapse(item)}
                <button class="collapse-toggle" on:click={() => toggleCollapsed(item)}>
                  <svg viewBox="0 0 16 16" aria-hidden="true">
                    <path
                      d={isCollapsed(item) ? "M6 4l4 4-4 4" : "M4 6l4 4 4-4"}
                      fill="none"
                      stroke="currentColor"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="1.4"
                    />
                  </svg>
                  <span>{isCollapsed(item) ? "Expand" : "Collapse"}</span>
                </button>
              {/if}
            </div>

            <div class="markdown">
              {#if isCollapsed(item)}
                <pre class="collapsed-preview">{previewText(item)}</pre>
              {:else}
                <MessageBody body={transcriptText(item)} />
              {/if}
            </div>
          {/if}
        </article>
      {/each}
    {/if}
  </div>

  <form class="composer" on:submit|preventDefault={submitDraft}>
    <textarea
      bind:value={draft}
      rows={3}
      placeholder="Send a message to continue this session"
      spellcheck={false}
      on:keydown={handleComposerKeydown}
    ></textarea>
    <button disabled={!draft.trim()} type="submit">
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path
          d="M2 8h9M8 3l5 5-5 5"
          fill="none"
          stroke="currentColor"
          stroke-linecap="round"
          stroke-linejoin="round"
          stroke-width="1.4"
        />
      </svg>
      <span>Send</span>
    </button>
  </form>

  {#if pendingPermissions.length}
    <div class="permission-bar">
      {#each pendingPermissions as permission}
        <div class="permission-copy">
          <p class="permission-label">Permission required</p>
          <p>{permission.permissionDialog.reason}</p>
        </div>
        <div class="permission-actions">
          {#each permission.choices as choice}
            <button type="button" on:click={() => onResolvePermission(permission.id, choice)}>
              {#if permissionIcon(choice) === "allow_once"}
                <svg viewBox="0 0 16 16" aria-hidden="true">
                  <path
                    d="M3.5 8.5 6.5 11.5 12.5 4.5"
                    fill="none"
                    stroke="currentColor"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="1.4"
                  />
                </svg>
              {:else if permissionIcon(choice) === "allow_session"}
                <svg viewBox="0 0 16 16" aria-hidden="true">
                  <circle
                    cx="8"
                    cy="8"
                    r="5.5"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.2"
                  />
                  <path
                    d="M8 5v3.2l2 1.3"
                    fill="none"
                    stroke="currentColor"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="1.4"
                  />
                </svg>
              {:else}
                <svg viewBox="0 0 16 16" aria-hidden="true">
                  <path
                    d="M4.5 4.5 11.5 11.5M11.5 4.5 4.5 11.5"
                    fill="none"
                    stroke="currentColor"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="1.4"
                  />
                </svg>
              {/if}
              <span>{choice}</span>
            </button>
          {/each}
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .conversation {
    min-width: 0;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    background:
      linear-gradient(180deg, rgba(252, 248, 242, 0.94), rgba(247, 240, 231, 0.8)),
      var(--canvas);
  }

  .conversation-header {
    padding: 1.1rem 1.7rem 0.35rem;
  }

  h2 {
    margin: 0 0 0.22rem;
    font-family: var(--font-display);
    font-size: 1.28rem;
    line-height: 1.08;
    letter-spacing: -0.02em;
    font-weight: 700;
  }

  .session-meta {
    margin: 0;
    color: var(--text-soft);
    font-size: 0.76rem;
    letter-spacing: 0.01em;
  }

  .thread {
    min-height: 0;
    overflow: auto;
    padding: 0.25rem 1.7rem 1.3rem;
    display: grid;
    gap: 1rem;
    align-content: start;
  }

  .entry {
    display: grid;
    gap: 0.32rem;
    max-width: 50rem;
    padding-left: 0.95rem;
    border-left: 2px solid transparent;
  }

  .entry.user {
    justify-items: end;
    justify-self: end;
    width: 100%;
    padding-left: 0;
    border-left: 0;
  }

  .entry.assistant {
    border-left-color: rgba(36, 105, 81, 0.2);
  }

  .entry.tool {
    border-left-color: rgba(118, 97, 72, 0.2);
  }

  .entry.diff {
    border-left-color: rgba(36, 105, 81, 0.12);
  }

  .entry.command,
  .entry.system {
    border-left-color: rgba(118, 97, 72, 0.12);
  }

  .entry-meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    color: var(--text-soft);
    font-size: 0.7rem;
    letter-spacing: 0;
    font-weight: 600;
  }

  .bubble {
    max-width: min(42rem, 80%);
    padding: 1rem 1.15rem;
    border-radius: 0;
    background: rgba(255, 255, 255, 0.94);
    box-shadow:
      0 16px 30px rgba(16, 21, 28, 0.06),
      0 1px 0 rgba(255, 255, 255, 0.55) inset;
  }

  .markdown {
    display: grid;
    gap: 0.8rem;
    color: var(--text);
    font-size: 1rem;
  }

  .tool-log {
    display: grid;
    gap: 0.7rem;
  }

  .diff-log {
    display: grid;
    gap: 0.55rem;
    padding: 0.72rem 0.85rem;
    background: rgba(255, 255, 255, 0.46);
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.45) inset,
      0 0 0 1px rgba(36, 105, 81, 0.08);
  }

  .diff-title,
  .diff-status {
    margin: 0;
  }

  .diff-title {
    font-weight: 700;
  }

  .diff-status {
    color: var(--text-soft);
    font-size: 0.8rem;
  }

  .diff-stats {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    color: var(--text-soft);
    font-size: 0.72rem;
  }

  .diff-log pre {
    margin: 0;
    padding: 0.72rem 0.85rem;
    background: rgba(255, 255, 255, 0.72);
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.55) inset,
      0 0 0 1px rgba(118, 97, 72, 0.1);
    white-space: pre-wrap;
    font-family: var(--font-mono);
    font-size: 0.82rem;
    line-height: 1.58;
    overflow: auto;
  }

  .tool-summary {
    margin: 0;
    line-height: 1.72;
  }

  .tool-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 0.65rem;
  }

  .tool-section {
    display: grid;
    gap: 0.28rem;
  }

  .tool-label {
    color: var(--text-soft);
    font-size: 0.7rem;
    letter-spacing: 0;
    font-weight: 600;
  }

  .tool-section pre {
    margin: 0;
    padding: 0.8rem 0.9rem;
    background: rgba(255, 255, 255, 0.78);
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.55) inset,
      0 0 0 1px rgba(118, 97, 72, 0.1);
    white-space: pre-wrap;
    font-family: var(--font-mono);
    font-size: 0.84rem;
    line-height: 1.65;
    overflow: auto;
  }

  .command-log {
    display: grid;
    padding: 0.72rem 0.85rem;
    background: rgba(255, 255, 255, 0.78);
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.55) inset,
      0 0 0 1px rgba(118, 97, 72, 0.1);
  }

  .command-log code {
    font-family: var(--font-mono);
    font-size: 0.86rem;
    white-space: pre;
    overflow-x: auto;
  }

  .collapsed-preview {
    margin: 0;
    white-space: pre-wrap;
    font-family: var(--font-mono);
    font-size: 0.9rem;
    line-height: 1.72;
    color: var(--text-muted);
  }

  .composer {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.9rem;
    padding: 1.05rem 1.7rem 1.15rem;
    background: rgba(246, 239, 229, 0.92);
    box-shadow: 0 -1px 0 rgba(120, 99, 72, 0.08) inset;
  }

  textarea {
    resize: vertical;
    min-height: 4rem;
    max-height: 12rem;
    padding: 1rem 1.05rem;
    border: 0;
    border-radius: 0;
    background: rgba(255, 255, 255, 0.82);
    color: var(--text);
    font: inherit;
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.55) inset;
    outline: none;
  }

  textarea:focus {
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.55) inset,
      0 0 0 3px rgba(36, 105, 81, 0.1);
  }

  .composer button,
  .collapse-toggle,
  .permission-actions button {
    border: 0;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.76);
    color: var(--text);
    padding: 0.58rem 0.82rem;
    cursor: pointer;
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.55) inset,
      0 0 0 1px rgba(118, 97, 72, 0.14);
    font: inherit;
    display: inline-flex;
    align-items: center;
    gap: 0.42rem;
  }

  .composer button {
    align-self: end;
    background: rgba(255, 255, 255, 0.92);
    color: var(--text);
    padding-inline: 1.05rem;
  }

  .composer::after {
    content: "Ctrl/Command + Enter to send";
    grid-column: 1 / -1;
    color: var(--text-soft);
    font-size: 0.68rem;
    line-height: 1;
  }

  .composer button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .collapse-toggle {
    padding: 0.24rem 0.5rem;
    color: var(--text-soft);
    font-size: 0.72rem;
  }

  .collapse-toggle svg,
  .composer button svg,
  .permission-actions button svg {
    width: 0.8rem;
    height: 0.8rem;
    flex: 0 0 auto;
  }

  .permission-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    padding: 1rem 1.7rem 1.2rem;
    background: rgba(244, 230, 208, 0.82);
    box-shadow: 0 -1px 0 rgba(141, 97, 48, 0.08) inset;
  }

  .permission-copy {
    display: grid;
    gap: 0.2rem;
    max-width: 42rem;
  }

  .permission-label {
    margin: 0;
    font-size: 0.66rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--warning);
    font-weight: 600;
  }

  .permission-copy p:last-child {
    margin: 0;
    color: var(--text);
  }

  .permission-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.55rem;
    justify-content: flex-end;
  }

  .state {
    margin: 0;
    color: var(--text-soft);
  }

  @media (max-width: 980px) {
    .tool-grid {
      grid-template-columns: 1fr;
    }

    .composer {
      grid-template-columns: 1fr;
    }

    .permission-bar {
      display: grid;
    }

    .bubble {
      max-width: 100%;
    }

    .conversation-header,
    .thread,
    .composer,
    .permission-bar {
      padding-left: 1.1rem;
      padding-right: 1.1rem;
    }
  }
</style>
