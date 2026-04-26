<script lang="ts">
  import "../../design/chat.css";

  import { tick } from "svelte";
  import Puffer from "../../design/Puffer.svelte";
  import Icon from "../../design/Icon.svelte";
  import MessageBody from "../../components/MessageBody.svelte";
  import ToolCard from "./ToolCard.svelte";
  import DiffCard from "./DiffCard.svelte";
  import Approval from "./Approval.svelte";
  import type {
    PermissionTimelineItem,
    SessionListItem,
    TimelineItem,
    ToolTimelineItem,
    DiffTimelineItem,
    MessageTimelineItem
  } from "../../types";
  import type { AgentState } from "../../shell/tweaks";

  type Props = {
    session: SessionListItem | null;
    agentName?: string;
    agentState?: AgentState;
    timeline: TimelineItem[];
    pendingPermissions: PermissionTimelineItem[];
    loading: boolean;
    /** True while an agent turn is running on the current session. Flips
     *  the composer's send button into a red "Stop" so the user can
     *  interrupt a runaway loop. */
    turnRunning?: boolean;
    turnStartedAtMs?: number | null;
    turnThinking?: boolean;
    turnStatusHint?: string | null;
    onSubmitMessage: (message: string) => void;
    onResolvePermission: (permissionId: string, choice: string) => void;
    onCancelTurn?: () => void;
  };

  let {
    session,
    agentName = "Puffer",
    agentState = "idle",
    timeline,
    pendingPermissions,
    loading,
    turnRunning = false,
    turnStartedAtMs = null,
    turnThinking = false,
    turnStatusHint = null,
    onSubmitMessage,
    onResolvePermission,
    onCancelTurn
  }: Props = $props();

  let draft = $state("");
  let threadEl: HTMLDivElement | undefined;
  let lastSessionId: string | null = null;
  let nowMs = $state(Date.now());

  // Rolled-up thread: we group consecutive tool / diff items under the most
  // recent assistant message so the design's "assistant speaks, then shows its
  // tool calls" shape matches real Claude transcripts.
  type RowKind =
    | { kind: "user"; item: MessageTimelineItem }
    | { kind: "system"; item: MessageTimelineItem }
    | {
        kind: "agent";
        item: MessageTimelineItem | null;
        children: (ToolTimelineItem | DiffTimelineItem)[];
        approvals: PermissionTimelineItem[];
      };

  function buildRows(items: TimelineItem[]): RowKind[] {
    const rows: RowKind[] = [];
    let current:
      | Extract<RowKind, { kind: "agent" }>
      | null = null;
    for (const item of items) {
      if (item.kind === "user") {
        if (current) { rows.push(current); current = null; }
        rows.push({ kind: "user", item: item as MessageTimelineItem });
      } else if (item.kind === "system") {
        if (current) { rows.push(current); current = null; }
        rows.push({ kind: "system", item: item as MessageTimelineItem });
      } else if (item.kind === "assistant" || item.kind === "command") {
        if (current) rows.push(current);
        current = {
          kind: "agent",
          item: item as MessageTimelineItem,
          children: [],
          approvals: []
        };
      } else if (item.kind === "tool") {
        if (!current) current = { kind: "agent", item: null, children: [], approvals: [] };
        current.children.push(item as ToolTimelineItem);
      } else if (item.kind === "diff") {
        if (!current) current = { kind: "agent", item: null, children: [], approvals: [] };
        current.children.push(item as DiffTimelineItem);
      }
    }
    if (current) rows.push(current);
    return rows;
  }

  let rows = $derived(buildRows(timeline.filter((i) => i.kind !== "permission")));

  function formatTime(ms: number | undefined): string {
    if (!ms) return "";
    const d = new Date(ms);
    const h = d.getHours();
    const m = d.getMinutes().toString().padStart(2, "0");
    const hh = h < 10 ? `0${h}` : `${h}`;
    return `${hh}:${m}`;
  }

  function formatElapsed(startedAtMs: number | null): string {
    if (!startedAtMs) return "";
    const elapsed = Math.max(0, nowMs - startedAtMs) / 1000;
    return elapsed < 10 ? `${elapsed.toFixed(1)}s` : `${Math.floor(elapsed)}s`;
  }

  $effect(() => {
    // On session change, reset scroll to top so users see the start.
    if (session?.id !== lastSessionId) {
      lastSessionId = session?.id ?? null;
      void tick().then(() => threadEl?.scrollTo({ top: 0, behavior: "auto" }));
    }
  });

  $effect(() => {
    if (!turnRunning || !turnStartedAtMs) return;
    nowMs = Date.now();
    const interval = window.setInterval(() => {
      nowMs = Date.now();
    }, 100);
    return () => window.clearInterval(interval);
  });

  async function submit() {
    const v = draft.trim();
    if (!v) return;
    onSubmitMessage(v);
    draft = "";
    await tick();
    threadEl?.scrollTo({ top: threadEl.scrollHeight, behavior: "smooth" });
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  // Distribute any pending permissions under the latest agent row so the
  // approval prompt sits with the tool call it's asking about.
  let distributedRows = $derived.by(() => {
    const out = [...rows];
    if (!pendingPermissions.length) return out;
    // attach to the last agent row (or append a synthetic one)
    const lastAgentIdx = (() => {
      for (let i = out.length - 1; i >= 0; i--) if (out[i].kind === "agent") return i;
      return -1;
    })();
    if (lastAgentIdx >= 0 && out[lastAgentIdx].kind === "agent") {
      const prev = out[lastAgentIdx] as Extract<RowKind, { kind: "agent" }>;
      out[lastAgentIdx] = { ...prev, approvals: [...prev.approvals, ...pendingPermissions] };
    } else {
      out.push({ kind: "agent", item: null, children: [], approvals: [...pendingPermissions] });
    }
    return out;
  });

  let typingLabel = $derived.by(() => {
    const elapsed = formatElapsed(turnStartedAtMs);
    const suffix = elapsed ? ` (${elapsed})` : "";
    if (turnRunning) {
      if (turnStatusHint) return `${turnStatusHint}${suffix}`;
      if (turnThinking) return `Thinking${suffix}`;
      return `Running${suffix}`;
    }
    if (agentState === "awaiting") return `${agentName} paused - waiting for your approval`;
    return null;
  });
</script>

<div class="pf-chat">
  <div class="pf-chat-thread" bind:this={threadEl}>
    <div class="pf-chat-thread-inner">
      {#if loading}
        <div class="state">Loading conversation…</div>
      {:else if rows.length === 0 && !typingLabel}
        <div class="state">No messages in this session yet. Send a prompt to get started.</div>
      {:else}
        {#each distributedRows as row, idx (idx)}
          {#if row.kind === "user"}
            <div class="pf-msg" data-role="user">
              <div class="pf-msg-avatar">Y</div>
              <div class="pf-msg-body">
                <div class="pf-msg-meta">
                  <span class="name">you</span>
                  <span class="time">{formatTime((row.item as MessageTimelineItem & { createdAtMs?: number }).createdAtMs)}</span>
                </div>
                <div class="pf-msg-text">
                  <MessageBody body={row.item.body} />
                </div>
              </div>
            </div>
          {:else if row.kind === "system"}
            <div class="pf-msg" data-role="system" style="opacity: 0.75;">
              <div class="pf-msg-avatar" style="background: var(--muted); font-size: 10px; text-transform: uppercase; letter-spacing: 0.06em;">sys</div>
              <div class="pf-msg-body">
                <div class="pf-msg-text">
                  <MessageBody body={row.item.body} />
                </div>
              </div>
            </div>
          {:else}
            <div class="pf-msg" data-role="agent">
              <div class="pf-msg-avatar"><Puffer size={26} state="idle" /></div>
              <div class="pf-msg-body">
                <div class="pf-msg-meta">
                  <span class="name">{agentName}</span>
                </div>
                {#if row.item}
                  <div class="pf-msg-text">
                    <MessageBody body={row.item.body} />
                  </div>
                {/if}
                {#if row.children.length || row.approvals.length}
                  <div class="agent-tools">
                    {#each row.children as child (child.id)}
                      {#if child.kind === "tool"}
                        <ToolCard item={child as ToolTimelineItem} />
                      {:else if child.kind === "diff"}
                        <DiffCard item={child as DiffTimelineItem} />
                      {/if}
                    {/each}
                    {#each row.approvals as p (p.id)}
                      <Approval item={p} onResolve={onResolvePermission} />
                    {/each}
                  </div>
                {/if}
              </div>
            </div>
          {/if}
        {/each}

        {#if typingLabel}
          <div class="pf-msg" data-role="agent" style="opacity: 0.85;">
            <div class="pf-msg-avatar"><Puffer size={26} state={agentState} /></div>
            <div class="pf-msg-body">
              <div class="typing">{typingLabel}</div>
            </div>
          </div>
        {/if}
      {/if}
    </div>
  </div>

  <div class="pf-composer-wrap">
    <div class="pf-composer">
      <textarea
        bind:value={draft}
        placeholder={session ? `Reply to ${agentName}…` : "Select a session to continue"}
        onkeydown={onKeydown}
        disabled={!session}
      ></textarea>
      <div class="pf-composer-foot">
        {#if session}
          <button type="button" class="pf-chip"><Icon name="folder" size={11} />{session.folderPath ? session.folderPath.split("/").pop() : "cwd"}</button>
        {/if}
        <span class="spacer"></span>
        <span style="font-size: 11px; color: var(--muted-foreground); font-family: var(--font-mono);">
          ⏎ to send · ⇧⏎ for newline
        </span>
        {#if turnRunning}
          <button
            type="button"
            class="pf-send-btn pf-stop-btn"
            onclick={onCancelTurn}
            aria-label="Stop turn"
            title="Stop the running agent turn"
          >
            <Icon name="pause2" size={14} />
          </button>
        {:else}
          <button type="button" class="pf-send-btn" disabled={!draft.trim() || !session} onclick={submit} aria-label="Send">
            <Icon name="arrowUp" size={15} />
          </button>
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .pf-chat {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--background);
  }
  .pf-chat-thread {
    flex: 1;
    overflow-y: auto;
    padding: 24px 0 24px;
  }
  .pf-chat-thread-inner {
    max-width: 820px;
    margin: 0 auto;
    padding: 0 32px;
    display: flex;
    flex-direction: column;
    gap: var(--puffer-row-gap, 14px);
  }
  .pf-composer-wrap {
    border-top: 1px solid var(--border);
    background: color-mix(in oklab, var(--background) 96%, var(--muted));
    padding: 14px 32px 18px;
    flex-shrink: 0;
  }
  .pf-composer {
    max-width: 820px;
    margin: 0 auto;
  }
  .agent-tools {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 12px;
  }
  .typing {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 6px;
    font-size: 13px;
    color: var(--muted-foreground);
    font-family: var(--font-mono);
  }
  .state {
    text-align: center;
    color: var(--muted-foreground);
    padding: 40px 0;
    font-size: 14px;
  }

  @media (max-width: 720px) {
    .pf-chat-thread-inner { padding: 0 16px; }
    .pf-composer-wrap { padding: 12px 16px 16px; }
  }
</style>
