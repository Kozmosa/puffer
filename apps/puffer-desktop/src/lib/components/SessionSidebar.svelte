<script lang="ts">
  import type { FolderGroup, SessionListItem } from "../types";

  export let groups: FolderGroup[] = [];
  export let activeSessionId: string | null = null;
  export let loading = false;
  export let onSelect: (session: SessionListItem) => void = () => {};

  const timeFormatter = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit"
  });

  let collapsedGroupIds = new Set<string>();
  let initializedCollapseState = false;

  function toggleGroup(groupId: string) {
    const next = new Set(collapsedGroupIds);
    if (next.has(groupId)) {
      next.delete(groupId);
    } else {
      next.add(groupId);
    }
    collapsedGroupIds = next;
  }

  function groupContainsActiveSession(group: FolderGroup): boolean {
    return activeSessionId !== null && group.sessions.some((session) => session.id === activeSessionId);
  }

  function sortSessions(sessions: SessionListItem[]): SessionListItem[] {
    return sessions.slice().sort((left, right) => {
      const leftActive = left.id === activeSessionId;
      const rightActive = right.id === activeSessionId;
      if (leftActive !== rightActive) {
        return leftActive ? -1 : 1;
      }
      return right.updatedAtMs - left.updatedAtMs;
    });
  }

  $: visibleGroups = groups
    .map((group) => ({
      ...group,
      sessions: sortSessions(group.sessions)
    }))
    .sort((left, right) => {
      const leftActive = groupContainsActiveSession(left);
      const rightActive = groupContainsActiveSession(right);
      if (leftActive !== rightActive) {
        return leftActive ? -1 : 1;
      }
      const leftLatest = Math.max(...left.sessions.map((session) => session.updatedAtMs));
      const rightLatest = Math.max(...right.sessions.map((session) => session.updatedAtMs));
      return rightLatest - leftLatest;
    });
  $: totalSessions = groups.reduce((count, group) => count + group.sessions.length, 0);
  $: {
    if (!initializedCollapseState && groups.length > 0) {
      collapsedGroupIds = new Set(
        groups
          .filter((group) => !groupContainsActiveSession(group))
          .map((group) => group.id)
      );
      initializedCollapseState = true;
    }
  }
  $: {
    if (activeSessionId) {
      const activeGroup = groups.find((group) => groupContainsActiveSession(group));
      if (activeGroup && collapsedGroupIds.has(activeGroup.id)) {
        const next = new Set(collapsedGroupIds);
        next.delete(activeGroup.id);
        collapsedGroupIds = next;
      }
    }
  }
</script>

<aside class="sidebar">
  <div class="sidebar-header">
    <h2>Conversations</h2>
    <p class="summary">{totalSessions} sessions indexed</p>
  </div>

  <div class="tree">
    {#if loading}
      <p class="state">Loading sessions...</p>
    {:else if !visibleGroups.length}
      <p class="state">No sessions found.</p>
    {:else}
      {#each visibleGroups as group}
        <section class="group">
          <button class="group-toggle" on:click={() => toggleGroup(group.id)}>
            <span class="group-heading">
              <svg viewBox="0 0 16 16" aria-hidden="true">
                <path
                  d={collapsedGroupIds.has(group.id) ? "M6 4l4 4-4 4" : "M4 6l4 4 4-4"}
                  fill="none"
                  stroke="currentColor"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="1.4"
                />
              </svg>
              <span class="group-name">{group.label}</span>
            </span>
            <span class="group-count">{collapsedGroupIds.has(group.id) ? "+" : group.sessions.length}</span>
          </button>

          {#if !collapsedGroupIds.has(group.id)}
            <div class="session-list">
              {#each group.sessions as session}
                <button
                  class:selected={session.id === activeSessionId}
                  class="session-link"
                  on:click={() => onSelect(session)}
                >
                  <span class="session-name">{session.displayName ?? session.title}</span>
                  <span class="session-meta">
                    {session.id === activeSessionId ? "Active now" : timeFormatter.format(session.updatedAtMs)}
                  </span>
                  {#if session.note && session.id === activeSessionId}
                    <span class="session-note">{session.note}</span>
                  {/if}
                </button>
              {/each}
            </div>
          {/if}
        </section>
      {/each}
    {/if}
  </div>
</aside>

<style>
  .sidebar {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    padding: 1rem 0.85rem 1rem;
    border-radius: 0;
    background:
      linear-gradient(180deg, rgba(32, 46, 54, 0.98), rgba(24, 36, 43, 0.98)),
      var(--sidebar);
    box-shadow: var(--shadow);
  }

  .sidebar-header {
    padding: 0.05rem 0.1rem 0.9rem;
  }

  h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 1.22rem;
    line-height: 1.08;
    letter-spacing: -0.02em;
    color: #f6f0e6;
  }

  .summary {
    margin: 0.22rem 0 0;
    color: var(--sidebar-muted);
    font-size: 0.72rem;
  }

  .tree {
    min-height: 0;
    overflow: auto;
    display: grid;
    gap: 0.9rem;
    padding-right: 0.15rem;
  }

  .group {
    display: grid;
    gap: 0.16rem;
  }

  .group-toggle {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.75rem;
    padding: 0;
    border: 0;
    background: transparent;
    color: #f6f0e6;
    text-align: left;
    cursor: pointer;
  }

  .group-heading {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
  }

  .group-heading svg {
    width: 0.78rem;
    height: 0.78rem;
    color: var(--sidebar-muted);
    flex: 0 0 auto;
  }

  .group-name {
    font-size: 0.84rem;
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  .group-count {
    color: var(--sidebar-muted);
    font-size: 0.68rem;
  }

  .session-list {
    display: grid;
    gap: 0.12rem;
    padding-top: 0.18rem;
    padding-left: 0.72rem;
  }

  .session-link {
    display: grid;
    gap: 0.12rem;
    padding: 0.45rem 0 0.45rem 0.7rem;
    border: 0;
    border-left: 2px solid rgba(255, 255, 255, 0.08);
    box-shadow: 0 0 0 1px transparent inset;
    background: transparent;
    text-align: left;
    cursor: pointer;
    color: #b7c0c6;
    transition: border-color 120ms ease, color 120ms ease, transform 120ms ease,
      background 120ms ease, box-shadow 120ms ease;
  }

  .session-link:hover {
    transform: translateX(2px);
    border-left-color: rgba(135, 201, 172, 0.28);
    color: #f5efe4;
    background: rgba(255, 255, 255, 0.03);
  }

  .session-link.selected {
    border-left-color: #87c9ac;
    color: #f8f3ea;
    background: rgba(255, 255, 255, 0.07);
    box-shadow:
      0 0 0 1px rgba(135, 201, 172, 0.16) inset,
      3px 0 0 rgba(135, 201, 172, 0.9) inset;
  }

  .session-name {
    font-size: 0.82rem;
    font-weight: 600;
    line-height: 1.28;
  }

  .session-meta,
  .session-note,
  .state {
    color: var(--sidebar-muted);
    font-size: 0.68rem;
    line-height: 1.4;
  }

  .session-link.selected .session-meta {
    color: #bcd4c8;
  }

  .session-note {
    max-width: 17rem;
  }

  .state {
    margin: 0;
    padding: 0.35rem 0.15rem;
  }
</style>
