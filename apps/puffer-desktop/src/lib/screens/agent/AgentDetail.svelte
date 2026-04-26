<script lang="ts">
  import Puffer from "../../design/Puffer.svelte";
  import Icon from "../../design/Icon.svelte";
  import ConversationView from "./ConversationView.svelte";
  import DiffView from "../../components/DiffView.svelte";
  import FilesPane from "./FilesPane.svelte";
  import ModelPicker from "./ModelPicker.svelte";
  import TerminalPane from "./TerminalPane.svelte";
  import {
    AGENT_STATE_LABELS,
    agentPufferState,
    type AgentStatus
  } from "../../data/mockProjects";
  import type {
    PermissionTimelineItem,
    SessionDetail,
    SessionListItem,
    SettingsSnapshot,
    TimelineItem
  } from "../../types";
  import type { AgentState } from "../../shell/tweaks";

  type Props = {
    // Live session data from the backend.
    session: SessionListItem | null;
    sessionDetail: SessionDetail | null;
    timeline: TimelineItem[];
    pendingPermissions: PermissionTimelineItem[];
    loading: boolean;
    turnRunning?: boolean;
    settingsSnapshot?: SettingsSnapshot | null;
    onBack: () => void;
    onSubmitMessage: (message: string) => void;
    onResolvePermission: (permissionId: string, choice: string) => void;
    onCancelTurn?: () => void;
    onModelChange?: (providerId: string, modelId: string) => void;
  };

  let {
    session,
    sessionDetail,
    timeline,
    pendingPermissions,
    loading,
    turnRunning = false,
    settingsSnapshot = null,
    onBack,
    onSubmitMessage,
    onResolvePermission,
    onCancelTurn,
    onModelChange
  }: Props = $props();

  type Tab = "chat" | "diff" | "terminal" | "files";
  type DiffSubTab = "agent" | "git" | "divergence";
  let tab = $state<Tab>("chat");
  let diffTab = $state<DiffSubTab>("agent");

  // Header identity comes straight from the live session record. No
  // local board persona — the daemon is the source of truth.
  let displayName = $derived(session?.displayName ?? session?.title ?? "Session");
  let displayTitle = $derived(session?.title ?? session?.note ?? "");
  let displayBranch = $derived(sessionDetail?.repoStatus?.branch ?? "");
  let displayProject = $derived(session?.folderPath?.split("/").pop() ?? "");
  let displayWorktree = $derived("");
  let status = $derived<AgentStatus>(inferStatusFromSession(sessionDetail));

  function inferStatusFromSession(d: SessionDetail | null): AgentStatus {
    if (!d) return "idle";
    const hasPending = d.timeline.some((t) => t.kind === "permission");
    if (hasPending) return "awaiting";
    if (d.repoStatus?.pullRequest) return "review";
    if (d.repoStatus?.hasUncommittedChanges) return "running";
    return "idle";
  }

  let pufferState = $derived<AgentState>(agentPufferState(status));
  let diffCount = $derived(timeline.filter((t) => t.kind === "diff").length);
  let agentDiff = $derived(sessionDetail?.agentDiff ?? { files: [], entries: [] });
  let divergence = $derived(
    sessionDetail?.divergence ?? { agentOnly: [], gitOnly: [], agentTotal: 0, gitTotal: 0 }
  );
  let divergenceCount = $derived(divergence.agentOnly.length + divergence.gitOnly.length);

  function kindIcon(kind: string): "edit" | "file" | "x" | "branch" {
    switch (kind) {
      case "write":
        return "file";
      case "remove":
        return "x";
      case "move":
        return "branch";
      default:
        return "edit";
    }
  }
</script>

<div class="pf-agent-detail">
  <div class="pf-agent-detail-head">
    <button type="button" class="pf-agent-back" onclick={onBack} title="Back to workspace" aria-label="Back">
      <Icon name="chevL" size={13} />
    </button>
    <Puffer size={20} state={pufferState} />
    <div class="pf-agent-identity">
      <div class="name">
        {displayName}
        {#if displayTitle}
          <span class="sep">·</span>
          <span class="title">{displayTitle}</span>
        {/if}
      </div>
      <div class="meta">
        {#if displayProject}
          <span class="mono">{displayProject}</span>
          <span class="sep">·</span>
        {/if}
        {#if displayBranch}
          <span class="branch mono"><Icon name="branch" size={10} />{displayBranch}</span>
          {#if displayWorktree}
            <span class="sep">·</span>
          {/if}
        {/if}
        {#if displayWorktree}
          <span class="mono">{displayWorktree}</span>
        {/if}
      </div>
    </div>
    {#if onModelChange}
      <ModelPicker
        snapshot={settingsSnapshot}
        onChange={(providerId, modelId) => onModelChange?.(providerId, modelId)}
      />
    {/if}
    <span class="pf-agent-status-pill" data-status={status}>
      {#if status === "running"}
        <span class="pip"></span>
      {/if}
      {AGENT_STATE_LABELS[status] ?? status}
    </span>
    <div class="pf-agent-tabs">
      <button class="pf-agent-tab" class:on={tab === "chat"} onclick={() => (tab = "chat")}>
        <Icon name="sparkles" size={12} />Chat
      </button>
      <button class="pf-agent-tab" class:on={tab === "diff"} onclick={() => (tab = "diff")}>
        <Icon name="git" size={12} />Diff
        {#if diffCount > 0}
          <span class="pf-agent-tab-badge">{diffCount}</span>
        {/if}
      </button>
      <button class="pf-agent-tab" class:on={tab === "terminal"} onclick={() => (tab = "terminal")}>
        <Icon name="terminal" size={12} />Terminal
      </button>
      <button class="pf-agent-tab" class:on={tab === "files"} onclick={() => (tab = "files")}>
        <Icon name="folder" size={12} />Files
      </button>
    </div>
  </div>

  <div class="pf-agent-detail-body">
    {#if tab === "chat"}
      <ConversationView
        session={session}
        agentName={displayName}
        agentState={pufferState}
        timeline={timeline}
        pendingPermissions={pendingPermissions}
        loading={loading}
        turnRunning={turnRunning}
        onSubmitMessage={onSubmitMessage}
        onResolvePermission={onResolvePermission}
        onCancelTurn={onCancelTurn}
      />
    {:else if tab === "diff"}
      <div class="diff-subtabs">
        <button
          class="diff-subtab"
          class:on={diffTab === "agent"}
          onclick={() => (diffTab = "agent")}
        >
          <Icon name="sparkles" size={11} />Agent
          {#if agentDiff.files.length > 0}
            <span class="pf-agent-tab-badge">{agentDiff.files.length}</span>
          {/if}
        </button>
        <button
          class="diff-subtab"
          class:on={diffTab === "git"}
          onclick={() => (diffTab = "git")}
        >
          <Icon name="git" size={11} />Git
          {#if divergence.gitTotal > 0}
            <span class="pf-agent-tab-badge">{divergence.gitTotal}</span>
          {/if}
        </button>
        <button
          class="diff-subtab"
          class:on={diffTab === "divergence"}
          onclick={() => (diffTab = "divergence")}
          title={divergenceCount > 0
            ? "Agent and git disagree on which files changed"
            : "Agent and git agree"}
        >
          <Icon name="bolt" size={11} />Divergence
          {#if divergenceCount > 0}
            <span class="pf-agent-tab-badge warn">{divergenceCount}</span>
          {/if}
        </button>
      </div>

      {#if diffTab === "agent"}
        {#if agentDiff.files.length > 0}
          <div class="diff-wrap">
            <div class="agent-diff-list">
              {#each agentDiff.files as file (file.path)}
                <article class="agent-diff-card">
                  <header>
                    <Icon
                      name={kindIcon(file.latestKind)}
                      size={12}
                      color="var(--muted-foreground)"
                    />
                    <span class="path mono" title={file.path}>{file.path}</span>
                    <span class="kind">{file.latestKind}</span>
                    {#if file.editCount > 1}
                      <span class="count">×{file.editCount}</span>
                    {/if}
                  </header>
                  <pre class="diff-snippet"><code>{file.latestSummary}</code></pre>
                </article>
              {/each}
            </div>
          </div>
        {:else}
          <div class="pane-empty">
            <Icon name="sparkles" size={20} color="var(--muted-foreground)" />
            <div class="title">No agent edits yet</div>
            <div class="sub">
              Once the agent writes or replaces a file, the per-edit summary lands here —
              independent of git, so you can see what the model intended even if a hook
              rolled it back.
            </div>
          </div>
        {/if}
      {:else if diffTab === "git"}
        {#if sessionDetail?.latestDiff}
          <div class="diff-wrap">
            <DiffView diff={sessionDetail.latestDiff} />
          </div>
        {:else}
          <div class="pane-empty">
            <Icon name="git" size={20} color="var(--muted-foreground)" />
            <div class="title">No git changes</div>
            <div class="sub">
              The session has no working-tree changes against HEAD. Edits the agent
              already committed won't appear here — switch to the Agent tab for those.
            </div>
          </div>
        {/if}
      {:else}
        <div class="diff-wrap divergence-pane">
          {#if divergenceCount === 0}
            <div class="pane-empty">
              <Icon name="check" size={20} color="var(--muted-foreground)" />
              <div class="title">Agent and git agree</div>
              <div class="sub">
                Every file the agent edited shows up in git diff, and nothing else has
                changed on disk. {divergence.agentTotal} agent · {divergence.gitTotal} git.
              </div>
            </div>
          {:else}
            {#if divergence.agentOnly.length > 0}
              <section class="diverge-block">
                <header>
                  <Icon name="sparkles" size={12} />
                  Agent edited, not in git ({divergence.agentOnly.length})
                </header>
                <p class="hint">
                  The agent claims to have edited these files but they don't appear in
                  the current git diff. Possible causes: a hook reverted the change, the
                  edit was committed earlier this session, or the apply silently failed.
                </p>
                <ul>
                  {#each divergence.agentOnly as path (path)}
                    <li class="mono">{path}</li>
                  {/each}
                </ul>
              </section>
            {/if}
            {#if divergence.gitOnly.length > 0}
              <section class="diverge-block">
                <header>
                  <Icon name="git" size={12} />
                  Changed on disk, no agent edit ({divergence.gitOnly.length})
                </header>
                <p class="hint">
                  Git sees these files as modified but no agent tool call touched them.
                  Possible causes: a post-tool hook rewrote the file, the user
                  hand-edited between turns, or a build artifact slipped in.
                </p>
                <ul>
                  {#each divergence.gitOnly as path (path)}
                    <li class="mono">{path}</li>
                  {/each}
                </ul>
              </section>
            {/if}
          {/if}
        </div>
      {/if}
    {:else if tab === "terminal"}
      <TerminalPane cwd={session?.cwd ?? displayProject} />
    {:else if tab === "files"}
      <FilesPane cwd={session?.cwd ?? displayProject} />
    {/if}
  </div>
</div>

<style>
  .pf-agent-detail {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--background);
  }
  .pf-agent-detail-head {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: color-mix(in oklab, var(--background) 96%, var(--muted));
    border-bottom: 1px solid var(--border);
    min-height: 52px;
  }
  .pf-agent-back {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--background);
    color: var(--muted-foreground);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    flex-shrink: 0;
    transition: background 120ms, color 120ms;
  }
  .pf-agent-back:hover { background: var(--accent); color: var(--foreground); }
  .pf-agent-identity {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 0 1 auto;
    max-width: 420px;
  }
  .pf-agent-identity .name {
    font-size: 14px;
    font-weight: 600;
    letter-spacing: -0.01em;
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .pf-agent-identity .name .sep { color: var(--muted-foreground); opacity: 0.5; }
  .pf-agent-identity .name .title {
    font-weight: 500;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pf-agent-identity .meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--muted-foreground);
  }
  .pf-agent-identity .meta .mono { font-family: var(--font-mono); }
  .pf-agent-identity .meta .sep { opacity: 0.4; }
  .pf-agent-identity .meta .branch {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--muted);
  }

  .pf-agent-status-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    font-weight: 600;
    font-family: var(--font-mono);
    padding: 3px 8px;
    border-radius: 999px;
    background: var(--muted);
    color: var(--muted-foreground);
    text-transform: lowercase;
    flex-shrink: 0;
    margin-left: auto;
  }
  .pf-agent-status-pill[data-status="running"]  { background: color-mix(in oklab, oklch(0.7 0.17 70) 15%, var(--background)); color: oklch(0.55 0.17 70); }
  .pf-agent-status-pill[data-status="awaiting"] { background: color-mix(in oklab, oklch(0.72 0.18 30) 16%, var(--background)); color: oklch(0.55 0.2 30); }
  .pf-agent-status-pill[data-status="review"]   { background: color-mix(in oklab, oklch(0.7 0.16 40) 15%, var(--background));  color: oklch(0.55 0.17 40); }
  .pf-agent-status-pill .pip {
    width: 6px; height: 6px; border-radius: 50%;
    background: oklch(0.7 0.17 70);
    animation: pf-pulse-dot 1.6s infinite;
  }
  @keyframes pf-pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .pf-agent-tabs {
    display: flex;
    gap: 1px;
    background: var(--muted);
    padding: 3px;
    border-radius: 8px;
    flex-shrink: 0;
  }
  .pf-agent-tab {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 10px;
    font-size: 12px;
    font-weight: 500;
    color: var(--muted-foreground);
    border: 0;
    background: transparent;
    border-radius: 5px;
    cursor: pointer;
    transition: background 120ms, color 120ms;
    font: inherit;
  }
  .pf-agent-tab:hover { color: var(--foreground); }
  .pf-agent-tab.on {
    background: var(--background);
    color: var(--foreground);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.06);
  }
  .pf-agent-tab-badge {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 3px;
    background: oklch(0.7 0.16 40);
    color: white;
    margin-left: 2px;
  }

  .pf-agent-detail-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .diff-wrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .pane-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px;
    color: var(--muted-foreground);
    text-align: center;
  }
  .pane-empty .title { font-size: 14px; font-weight: 600; color: var(--foreground); }
  .pane-empty .sub { font-size: 12.5px; max-width: 360px; line-height: 1.55; }

  .diff-subtabs {
    display: flex;
    gap: 4px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in oklab, var(--background) 96%, var(--muted));
  }
  .diff-subtab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border: 1px solid transparent;
    border-radius: 999px;
    background: transparent;
    color: var(--muted-foreground);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: color 100ms, border-color 100ms, background 100ms;
  }
  .diff-subtab:hover { color: var(--foreground); }
  .diff-subtab.on {
    color: var(--foreground);
    background: var(--background);
    border-color: var(--border);
  }
  .pf-agent-tab-badge.warn {
    background: color-mix(in oklab, oklch(0.62 0.22 25) 18%, var(--background));
    color: oklch(0.55 0.2 30);
    border: 1px solid color-mix(in oklab, oklch(0.62 0.22 25) 35%, var(--border));
  }

  .agent-diff-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 16px;
  }
  .agent-diff-card {
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    background: var(--background);
  }
  .agent-diff-card header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: color-mix(in oklab, var(--background) 96%, var(--muted));
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .agent-diff-card .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--foreground);
  }
  .agent-diff-card .kind {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--muted);
    color: var(--muted-foreground);
  }
  .agent-diff-card .count {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--muted-foreground);
  }
  .diff-snippet {
    margin: 0;
    padding: 10px 12px;
    font-family: var(--font-mono);
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--foreground);
    white-space: pre;
    overflow-x: auto;
  }
  .diff-snippet code { color: inherit; }

  .divergence-pane {
    padding: 14px 16px;
  }
  .diverge-block {
    margin-bottom: 18px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--background);
    overflow: hidden;
  }
  .diverge-block header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    font-weight: 600;
    font-size: 12.5px;
    color: var(--foreground);
    background: color-mix(in oklab, var(--background) 96%, var(--muted));
  }
  .diverge-block .hint {
    margin: 0;
    padding: 10px 14px;
    color: var(--muted-foreground);
    font-size: 12px;
    line-height: 1.55;
  }
  .diverge-block ul {
    list-style: none;
    margin: 0;
    padding: 0 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .diverge-block li {
    font-size: 12px;
    color: var(--foreground);
    padding: 4px 8px;
    border-radius: 4px;
    background: color-mix(in oklab, var(--muted) 50%, var(--background));
  }

  @media (max-width: 720px) {
    .pf-agent-detail-head { flex-wrap: wrap; row-gap: 6px; padding: 8px 10px; }
    .pf-agent-tabs { order: 3; width: 100%; overflow-x: auto; }
    .pf-agent-status-pill { order: 2; margin-left: 0; }
  }
</style>
