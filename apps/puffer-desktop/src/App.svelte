<script lang="ts">
  import { onMount } from "svelte";

  import TitleBar, { type TitleTab } from "./lib/shell/TitleBar.svelte";
  import Sidebar, { type ActiveAgent, type UserChip } from "./lib/shell/Sidebar.svelte";
  import TweaksPanel from "./lib/shell/TweaksPanel.svelte";
  import {
    applyTweaksToDocument,
    defaultTweaks,
    loadTweaks,
    persistTweaks,
    type ScreenId,
    type Tweaks,
    type AgentState
  } from "./lib/shell/tweaks";

  import Workspace from "./lib/screens/Workspace.svelte";
  import WorkspacePicker from "./lib/screens/WorkspacePicker.svelte";
  import AgentDetail from "./lib/screens/agent/AgentDetail.svelte";
  import Pipelines from "./lib/screens/Pipelines.svelte";
  import Deployments from "./lib/screens/Deployments.svelte";
  import Settings from "./lib/screens/Settings.svelte";
  import Onboarding from "./lib/screens/Onboarding.svelte";

  import {
    createPullRequest,
    importExternalCredential,
    listExternalCredentials,
    loginWithApiKey,
    loginWithApiKeyViaDaemon,
    loginWithOauth,
    listGroupedSessionsFromDaemon,
    loadSettingsSnapshot,
    loadSessionDetailFromDaemon,
    mergePullRequest,
    logoutProvider,
    logoutProviderViaDaemon,
    readRemoteFile,
    refreshRepoStatus,
    runRemoteBash,
    writeRemoteFile,
    runAgentTurn,
    resolvePermission as resolveTurnPermission,
    cancelTurn,
    createSession,
    loadDefaultWorkspace
  } from "./lib/api/desktop";
  import {
    subscribeSessionEvents,
    type SessionStreamEvent
  } from "./lib/api/sessionEvents";
  import {
    currentDaemonClient,
    ensureLocalDaemonClient,
    type ConnectionState
  } from "./lib/api/daemonClient";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import type {
    DesktopPreferences,
    ExternalCredential,
    FolderGroup,
    PermissionTimelineItem,
    RemoteConnection,
    RemoteOperation,
    SessionDetail,
    SessionListItem,
    SettingsSnapshot,
    TimelineItem
  } from "./lib/types";

  // ─────────────────────────────────────────────────────────────
  // Shell state
  // ─────────────────────────────────────────────────────────────
  let tweaks = $state<Tweaks>({ ...defaultTweaks });
  let onboarding = $state(true);
  // Dev bypass so we can screenshot every screen without live auth.
  const urlParams = typeof window !== "undefined" ? new URLSearchParams(window.location.search) : new URLSearchParams();
  const skipOnboarding =
    typeof window !== "undefined" &&
    (urlParams.has("skipOnboarding") ||
      window.localStorage.getItem("puffer-desktop:skip-onboarding") === "1");
  const forceOnboarding = urlParams.has("forceOnboarding");
  let statusMessage = $state("Desktop workspace ready.");
  // Auto-dismiss the status strip a few seconds after each message so it
  // doesn't linger in the sidebar corner looking like a truncated widget.
  let statusDismissTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    // Re-arm whenever `statusMessage` changes to a non-default value.
    if (!statusMessage || statusMessage === "Desktop workspace ready.") return;
    if (statusDismissTimer) clearTimeout(statusDismissTimer);
    statusDismissTimer = setTimeout(() => {
      statusMessage = "Desktop workspace ready.";
      statusDismissTimer = null;
    }, 4000);
  });
  let showWorkspacePicker = $state(false);

  // Backend-backed state
  let groups = $state<FolderGroup[]>([]);
  let groupsLoading = $state(false);
  let selectedSession = $state<SessionListItem | null>(null);
  let sessionDetail = $state<SessionDetail | null>(null);
  let sessionLoading = $state(false);

  // Drill-in marker: which session id is currently expanded in AgentDetail.
  // Cleared when the user backs out to the workspace board.
  let openAgentSessionId = $state<string | null>(null);
  let submittedMessages = $state<TimelineItem[]>([]);
  let dismissedPermissionIds = $state<string[]>([]);

  // Live turn state: items synthesized from streaming events while a turn is
  // running. When the turn finishes we reload the session detail so the real
  // persisted transcript replaces these placeholders.
  let currentTurnId = $state<string | null>(null);
  let liveStreamItems = $state<TimelineItem[]>([]);
  let turnPermissionLookup = $state<Record<string, { turnId: string; requestId: string }>>({});
  let sessionEventUnlisten: UnlistenFn | null = null;
  let subscribedSessionId: string | null = null;
  let connectionState = $state<ConnectionState>("idle");

  let settingsSnapshot = $state<SettingsSnapshot | null>(null);
  let settingsLoading = $state(false);
  let authBusyProviderId = $state<string | null>(null);
  let authError = $state<string | null>(null);
  let externalCredentials = $state<ExternalCredential[]>([]);
  let importBusyKey = $state<string | null>(null);
  let actionBusy = $state(false);
  let remoteOperation = $state<RemoteOperation | null>(null);
  let remoteBusy = $state(false);
  let remotePassword = $state("");

  // Tauri is stateless: preferences live in Puffer's workspace config, not
  // here. We keep an in-memory copy to drive the Settings pane but never
  // persist it — relaunching the app re-reads from the daemon.
  const defaultDesktopPreferences: DesktopPreferences = {
    rememberSession: false,
    rememberInspectorLayout: false,
    launchInspectorOpen: true,
    defaultInspectorTab: "latest-diff",
    defaultInspectorWidth: 50,
    remoteEnabled: false,
    remoteTarget: "",
    remoteCwd: ""
  };
  let desktopPreferences = $state<DesktopPreferences>({ ...defaultDesktopPreferences });

  // The daemon's default workspace (host, path). Shown in the sidebar /
  // workspace header; new sessions default to this cwd.
  let defaultWorkspaceCwd = $state<string>("");

  let remoteConnection = $derived<RemoteConnection>({
    enabled:
      desktopPreferences.remoteEnabled && desktopPreferences.remoteTarget.trim().length > 0,
    target: desktopPreferences.remoteTarget.trim(),
    cwd: desktopPreferences.remoteCwd.trim(),
    password: remotePassword
  });

  // ─────────────────────────────────────────────────────────────
  // Active agent mapping (sessions → sidebar agents)
  //   review  = open PR OR pending manual approval
  //   running = active work (unresolved permission / uncommitted changes)
  //   done    = merged / clean on a branch with closed PR
  //   idle    = otherwise
  // For Phase 1 we can only distinguish by session metadata at this shell level,
  // so we mark everything idle until AgentDetail in Phase 2 has per-session state.
  // ─────────────────────────────────────────────────────────────
  let combinedTimeline = $derived<TimelineItem[]>([
    ...(sessionDetail?.timeline ?? []),
    ...submittedMessages,
    ...liveStreamItems
  ]);
  let pendingPermissions = $derived<PermissionTimelineItem[]>(
    combinedTimeline.filter(
      (t): t is PermissionTimelineItem =>
        t.kind === "permission" && !dismissedPermissionIds.includes(t.id)
    )
  );

  let realAgents = $derived<ActiveAgent[]>(
    groups.flatMap((g) =>
      g.sessions.slice(0, 3).map((s) => ({
        id: s.id,
        name: (s.displayName ?? s.title).slice(0, 24) || "session",
        title: s.title,
        project: g.label,
        branch: "",
        state: "idle" as AgentState
      }))
    )
  );

  let activeAgents = $derived<ActiveAgent[]>(realAgents);

  let userChip = $derived<UserChip | null>(
    settingsSnapshot?.auth.length
      ? {
          initials: (settingsSnapshot.auth[0].email ?? "you").slice(0, 2).toUpperCase(),
          name: settingsSnapshot.auth[0].email ?? "You",
          meta: `${settingsSnapshot.auth[0].providerId}${
            settingsSnapshot.auth[0].planType ? " · " + settingsSnapshot.auth[0].planType : ""
          }`
        }
      : null
  );

  let tabs = $derived<TitleTab[]>(
    selectedSession
      ? [
          {
            id: selectedSession.id,
            title: selectedSession.displayName ?? selectedSession.title,
            state: tweaks.agentState
          }
        ]
      : []
  );
  let activeTab = $state<string>("");
  $effect(() => {
    if (selectedSession) activeTab = selectedSession.id;
  });

  // ─────────────────────────────────────────────────────────────
  // Init
  // ─────────────────────────────────────────────────────────────
  onMount(() => {
    tweaks = loadTweaks();
    applyTweaksToDocument(tweaks);
    if (forceOnboarding) {
      onboarding = true;
    } else if (skipOnboarding) {
      onboarding = false;
    }
    void init();
  });

  $effect(() => {
    applyTweaksToDocument(tweaks);
    persistTweaks(tweaks); // Tweaks are renderer ergonomics, not workspace data.
  });

  async function init() {
    void loadDefaultWorkspace()
      .then((info) => {
        defaultWorkspaceCwd = info.cwd;
      })
      .catch(() => {
        /* daemon might be remote / unavailable; keep default empty */
      });
    // Observe daemon connection state so the banner reflects reality.
    void ensureLocalDaemonClient()
      .then((client) => {
        client.onConnectionChange((s) => {
          connectionState = s;
          // When we reconnect after a drop, refresh groups + re-open the
          // selected session so the UI catches up.
          if (s === "open" && !onboarding) {
            void refreshGroups();
            if (selectedSession) void openSession(selectedSession);
          }
        });
        // Any time a session is created or a turn finishes, refresh the
        // workspace board + sidebar. Coalesced by `refreshGroups`'s own
        // loading guard.
        client.on("workspace:sessions:changed", () => {
          void refreshGroups();
        });
      })
      .catch(() => {
        /* connection may be unavailable (web preview); stay idle */
      });
    await refreshSettings();
    if (!onboarding) {
      await refreshGroups();
      // When drilled into a mock agent via the screenshot harness (or the
      // user just landed after login without picking a session), auto-open
      // the most recent real session so the Chat tab renders a transcript
      // instead of the empty state.
      if (!selectedSession) {
        const firstReal = groups
          .flatMap((g) => g.sessions)
          .sort((a, b) => b.updatedAtMs - a.updatedAtMs)[0];
        if (firstReal) {
          await openSession(firstReal);
        }
      }
    }
  }

  // ─────────────────────────────────────────────────────────────
  // Handlers — mostly lifted from the prior App.svelte
  // ─────────────────────────────────────────────────────────────
  async function refreshSettings() {
    settingsLoading = true;
    try {
      settingsSnapshot = await loadSettingsSnapshot(remoteConnection);
      if (forceOnboarding) {
        onboarding = true;
      } else if (skipOnboarding) {
        onboarding = false;
      } else {
        onboarding = (settingsSnapshot.auth?.length ?? 0) === 0;
      }
      // Re-scan ~/.claude / ~/.codex so the LoginView can offer one-click
      // imports for credentials the user already has on disk. Failure is
      // non-fatal — the manual API-key path still works.
      void listExternalCredentials()
        .then((found) => {
          externalCredentials = found;
        })
        .catch(() => {
          externalCredentials = [];
        });
      statusMessage = "Settings snapshot refreshed.";
    } catch (error) {
      statusMessage = String(error);
      if (skipOnboarding) onboarding = false;
    } finally {
      settingsLoading = false;
    }
  }

  async function handleImportExternal(providerId: string, source: "claude" | "codex") {
    importBusyKey = `${providerId}::${source}`;
    authError = null;
    try {
      settingsSnapshot = await importExternalCredential(providerId, source);
      onboarding = false;
      tweaks = { ...tweaks, screen: "workspace" };
      statusMessage = `Imported ${source} credential into ${providerId}.`;
      void listExternalCredentials()
        .then((found) => {
          externalCredentials = found;
        })
        .catch(() => {});
      await refreshGroups();
    } catch (error) {
      authError = String(error);
      statusMessage = authError;
    } finally {
      importBusyKey = null;
    }
  }

  async function handleOauthLogin(providerId: string) {
    authBusyProviderId = providerId;
    authError = null;
    try {
      settingsSnapshot = await loginWithOauth(providerId, remoteConnection);
      onboarding = false;
      tweaks = { ...tweaks, screen: "workspace" };
      statusMessage = `Connected to ${providerId}.`;
      await refreshGroups();
    } catch (error) {
      authError = String(error);
      statusMessage = authError;
    } finally {
      authBusyProviderId = null;
    }
  }

  async function handleApiKeyLogin(providerId: string, apiKey: string) {
    authBusyProviderId = providerId;
    authError = null;
    try {
      // Prefer the daemon path; it reuses the workspace auth store and
      // lets remote daemons (SSH) pick up credentials server-side. Falls
      // back to the Tauri-invoke path inside the wrapper when no daemon
      // is reachable. For genuinely remote connections we stay on the
      // Tauri path so `remoteConnection` (SSH command) is honored.
      if (remoteConnection.enabled) {
        settingsSnapshot = await loginWithApiKey(providerId, apiKey, remoteConnection);
      } else {
        settingsSnapshot = await loginWithApiKeyViaDaemon(providerId, apiKey);
      }
      onboarding = false;
      tweaks = { ...tweaks, screen: "workspace" };
      statusMessage = `Stored API key for ${providerId}.`;
      await refreshGroups();
    } catch (error) {
      authError = String(error);
      statusMessage = authError;
    } finally {
      authBusyProviderId = null;
    }
  }

  async function handleLogout(providerId: string) {
    authBusyProviderId = providerId;
    authError = null;
    try {
      if (remoteConnection.enabled) {
        settingsSnapshot = await logoutProvider(providerId, remoteConnection);
      } else {
        settingsSnapshot = await logoutProviderViaDaemon(providerId);
      }
      statusMessage = `Disconnected ${providerId}.`;
      if ((settingsSnapshot.auth?.length ?? 0) === 0) {
        groups = [];
        selectedSession = null;
        sessionDetail = null;
        onboarding = true;
      }
    } catch (error) {
      authError = String(error);
      statusMessage = authError;
    } finally {
      authBusyProviderId = null;
    }
  }

  async function refreshGroups() {
    groupsLoading = true;
    try {
      groups = await listGroupedSessionsFromDaemon();
      statusMessage =
        groups.length === 0
          ? "No sessions in this workspace yet."
          : `${groups.length} project${groups.length === 1 ? "" : "s"} loaded.`;
    } catch (error) {
      statusMessage = String(error);
    } finally {
      groupsLoading = false;
    }
  }

  async function openSession(session: SessionListItem) {
    sessionLoading = true;
    try {
      const detail = await loadSessionDetailFromDaemon(session.id);
      selectedSession = detail.session;
      sessionDetail = detail;
      // New session lands: drop any lingering live-stream items + local draft
      // so the composer feels fresh.
      submittedMessages = [];
      liveStreamItems = [];
      turnPermissionLookup = {};
      statusMessage = `Loaded ${detail.timeline.length} conversation items.`;
    } catch (error) {
      statusMessage = String(error);
    } finally {
      sessionLoading = false;
    }
  }

  /** Creates a blank session via the daemon in the given cwd (or the daemon's
   *  default workspace if unset) and opens AgentDetail on it. The workspace
   *  list refreshes so the new session appears as an agent card. */
  async function handleNewAgent(cwd: string) {
    try {
      const created = await createSession(cwd || undefined);
      await refreshGroups();
      const newSession =
        groups.flatMap((g) => g.sessions).find((s) => s.id === created.sessionId) ?? null;
      if (newSession) {
        await openSession(newSession);
      } else {
        // Fall back to a synthetic SessionListItem so the AgentDetail can
        // still open; reloading later will pick up the real record.
        const fallback: SessionListItem = {
          id: created.sessionId,
          displayName: null,
          title: "New agent",
          cwd: created.cwd,
          folderPath: created.cwd,
          updatedAtMs: created.createdAtMs,
          createdAtMs: created.createdAtMs,
          eventCount: 0,
          slug: null,
          tags: [],
          note: null,
          parentSessionId: null
        };
        await openSession(fallback);
      }
      openAgentSessionId = created.sessionId;
      tweaks = { ...tweaks, screen: "workspace" };
      statusMessage = `New agent in ${cwd || defaultWorkspaceCwd || "default workspace"}.`;
    } catch (error) {
      statusMessage = `Failed to create session: ${error}`;
    }
  }

  function updateDesktopPreference<K extends keyof DesktopPreferences>(key: K, value: DesktopPreferences[K]) {
    desktopPreferences = { ...desktopPreferences, [key]: value };
  }

  function resetDesktopPreferences() {
    desktopPreferences = { ...defaultDesktopPreferences };
    statusMessage = "Desktop preferences reset.";
  }

  async function handleRemoteBash(command: string) {
    if (!remoteConnection.enabled) return;
    remoteBusy = true;
    try {
      remoteOperation = await runRemoteBash(remoteConnection, command);
      statusMessage = remoteOperation.success ? "Remote bash finished." : "Remote bash failed.";
    } catch (error) {
      statusMessage = String(error);
      remoteOperation = { success: false, stdout: "", stderr: String(error) };
    } finally {
      remoteBusy = false;
    }
  }

  async function handleRemoteRead(path: string) {
    if (!remoteConnection.enabled) return;
    remoteBusy = true;
    try {
      remoteOperation = await readRemoteFile(remoteConnection, path);
      statusMessage = remoteOperation.success ? `Read remote file ${path}.` : `Reading ${path} failed.`;
    } catch (error) {
      statusMessage = String(error);
      remoteOperation = { success: false, stdout: "", stderr: String(error) };
    } finally {
      remoteBusy = false;
    }
  }

  async function handleRemoteWrite(path: string, contents: string) {
    if (!remoteConnection.enabled) return;
    remoteBusy = true;
    try {
      remoteOperation = await writeRemoteFile(remoteConnection, path, contents);
      statusMessage = remoteOperation.success ? `Wrote remote file ${path}.` : `Writing ${path} failed.`;
    } catch (error) {
      statusMessage = String(error);
      remoteOperation = { success: false, stdout: "", stderr: String(error) };
    } finally {
      remoteBusy = false;
    }
  }

  // Phase 2+: these aren't surfaced yet in the new UI, but we keep the handlers
  // live so PR / repo actions continue to work through whatever embeds them.
  // Referenced via the noop below so TS/svelte-check don't treat them as dead.
  const _keepAlive = { createPullRequest, mergePullRequest, refreshRepoStatus, cancelTurn };
  void _keepAlive;

  function updateTweak<K extends keyof Tweaks>(key: K, value: Tweaks[K]) {
    tweaks = { ...tweaks, [key]: value };
  }

  function onSelectScreen(id: ScreenId) {
    tweaks = { ...tweaks, screen: id };
  }

  function onSelectTab(id: string) {
    activeTab = id;
  }

  function onOpenAgent(id: string) {
    const realTarget = groups.flatMap((g) => g.sessions).find((s) => s.id === id);
    if (!realTarget) return;
    openAgentSessionId = realTarget.id;
    tweaks = { ...tweaks, screen: "workspace" };
    void openSession(realTarget);
  }

  function onCloseAgent() {
    openAgentSessionId = null;
  }

  /** Fired by ConnectProjectModal once a clone+create has landed. Refreshes
   *  the workspace board and drills straight into the new session. */
  async function handleSessionReady(sessionId: string) {
    // Refresh the default workspace info in case we just connected to a
    // remote daemon — the workspace root changed.
    void loadDefaultWorkspace()
      .then((info) => {
        defaultWorkspaceCwd = info.cwd;
      })
      .catch(() => {});
    await refreshGroups();
    const session = groups.flatMap((g) => g.sessions).find((s) => s.id === sessionId);
    if (session) {
      await openSession(session);
    }
    openAgentSessionId = sessionId;
    tweaks = { ...tweaks, screen: "workspace" };
  }

  async function submitMessage(message: string) {
    if (!selectedSession) {
      statusMessage = "Select a session to send a message.";
      return;
    }
    const now = Date.now();
    submittedMessages = [
      ...submittedMessages,
      {
        id: `local-user-${now}`,
        kind: "user",
        title: "User",
        summary: message,
        body: message,
        meta: []
      }
    ];
    try {
      const turnId = await runAgentTurn(selectedSession.id, message);
      currentTurnId = turnId;
      statusMessage = `Agent turn ${turnId.slice(0, 8)} started.`;
    } catch (error) {
      statusMessage = `run_agent_turn failed: ${error}`;
    }
  }

  function mapPermissionAction(choice: string): "allow_once" | "allow_session" | "allow_all_session" | "deny" {
    const n = choice.toLowerCase();
    if (n.includes("always") && n.includes("session")) return "allow_all_session";
    if (n.includes("always")) return "allow_all_session";
    if (n.includes("session")) return "allow_session";
    if (n.includes("deny") || n.includes("never")) return "deny";
    return "allow_once";
  }

  async function resolvePermission(permissionId: string, choice: string) {
    dismissedPermissionIds = [...dismissedPermissionIds, permissionId];
    const mapping = turnPermissionLookup[permissionId];
    if (mapping) {
      try {
        await resolveTurnPermission(mapping.turnId, mapping.requestId, mapPermissionAction(choice));
        statusMessage = `${choice} sent to agent.`;
      } catch (error) {
        statusMessage = `resolve_permission failed: ${error}`;
      }
      const { [permissionId]: _drop, ...rest } = turnPermissionLookup;
      turnPermissionLookup = rest;
    } else {
      statusMessage = `${choice} selected (no in-flight turn).`;
    }
  }

  function appendLive(item: TimelineItem) {
    liveStreamItems = [...liveStreamItems, item];
  }

  function upsertStreamingAssistant(delta: string) {
    const last = liveStreamItems[liveStreamItems.length - 1];
    if (last && last.kind === "assistant" && last.id.startsWith("live-stream-assistant")) {
      const updated = { ...last, body: last.body + delta, summary: last.body + delta };
      liveStreamItems = [...liveStreamItems.slice(0, -1), updated];
    } else {
      appendLive({
        id: `live-stream-assistant-${Date.now()}`,
        kind: "assistant",
        title: "Assistant",
        summary: delta,
        body: delta,
        meta: []
      });
    }
  }

  function handleSessionEvent(sid: string, ev: SessionStreamEvent) {
    if (!selectedSession || selectedSession.id !== sid) return;
    switch (ev.type) {
      case "turn-start":
        liveStreamItems = [];
        break;
      case "text-delta":
        upsertStreamingAssistant(ev.delta);
        break;
      case "tool-calls-requested":
        // Render an immediate pending card per requested call so the user
        // sees *what* the agent is doing before it finishes. The id is
        // `live-tool-<callId>` — we replace in place when `tool-invocations`
        // arrives with the matching callId.
        for (const req of ev.requests) {
          const id = `live-tool-${req.callId}`;
          if (liveStreamItems.some((x) => x.id === id)) continue;
          appendLive({
            id,
            kind: "tool",
            title: req.toolId,
            summary: `${req.toolId} · running`,
            body: "",
            meta: [],
            toolName: req.toolId,
            status: "running",
            input: req.input,
            output: "",
            inputJson: safeParseJson(req.input)
          });
        }
        break;
      case "tool-invocations":
        for (const inv of ev.invocations) {
          const id = `live-tool-${inv.callId}`;
          const existingIdx = liveStreamItems.findIndex((x) => x.id === id);
          const payload: TimelineItem = {
            id,
            kind: "tool",
            title: inv.toolId,
            summary: `${inv.toolId} · ${inv.success ? "success" : "error"}`,
            body: inv.output,
            meta: [],
            toolName: inv.toolId,
            status: inv.success ? "success" : "error",
            input: inv.input,
            output: inv.output,
            inputJson: safeParseJson(inv.input)
          };
          if (existingIdx >= 0) {
            // Upgrade the pending card in place. Svelte needs a new array
            // reference to observe the change.
            liveStreamItems = [
              ...liveStreamItems.slice(0, existingIdx),
              payload,
              ...liveStreamItems.slice(existingIdx + 1)
            ];
          } else {
            appendLive(payload);
          }
        }
        break;
      case "permission-request": {
        const id = `live-perm-${ev.requestId}`;
        appendLive({
          id,
          kind: "permission",
          title: `Permission · ${ev.toolId}`,
          summary: ev.summary,
          body: ev.reason ?? ev.summary,
          meta: [],
          toolName: ev.toolId,
          status: "pending",
          permissionDialog: {
            state: "pending",
            reason: ev.reason ?? ev.summary,
            summary: ev.summary,
            inputText: null,
            toolName: ev.toolId,
            choices: ["Allow once", "Always allow", "Deny"]
          },
          scopeLabel: null,
          choices: ["Allow once", "Always allow", "Deny"]
        });
        turnPermissionLookup = {
          ...turnPermissionLookup,
          [id]: { turnId: ev.turnId, requestId: ev.requestId }
        };
        break;
      }
      case "turn-complete":
      case "turn-error":
        currentTurnId = null;
        if (ev.type === "turn-error") {
          // Surface the daemon's error so the user sees *why* the agent
          // didn't reply — otherwise we'd silently reload an empty
          // transcript. Renders inline as a system-style timeline item
          // and a status-strip toast.
          const detail = ev.error?.trim() || "Unknown agent error.";
          statusMessage = `Agent error: ${detail}`;
          appendLive({
            id: `live-turn-error-${ev.turnId}`,
            kind: "system",
            title: "Agent error",
            summary: detail,
            body: detail,
            meta: ["turn-error"]
          });
        }
        // Reload the persisted transcript; then drop live items.
        if (selectedSession) {
          void openSession(selectedSession).then(() => {
            // Preserve a turn-error placeholder so the user can still
            // read the failure after the persisted transcript reloads.
            const errorItems = liveStreamItems.filter((item) =>
              item.id.startsWith("live-turn-error-")
            );
            liveStreamItems = errorItems;
            submittedMessages = [];
          });
        }
        break;
    }
  }

  function safeParseJson(text: string): Record<string, unknown> | null {
    try {
      const v = JSON.parse(text);
      return typeof v === "object" && v !== null ? (v as Record<string, unknown>) : null;
    } catch {
      return null;
    }
  }

  async function ensureSessionSubscription() {
    if (!selectedSession) {
      if (sessionEventUnlisten) {
        sessionEventUnlisten();
        sessionEventUnlisten = null;
      }
      subscribedSessionId = null;
      return;
    }
    if (subscribedSessionId === selectedSession.id && sessionEventUnlisten) return;
    if (sessionEventUnlisten) sessionEventUnlisten();
    const sid = selectedSession.id;
    subscribedSessionId = sid;
    sessionEventUnlisten = await subscribeSessionEvents(sid, (ev) => handleSessionEvent(sid, ev));
  }

  $effect(() => {
    void ensureSessionSubscription();
  });
</script>

<div class="pf-mac">
  <TitleBar
    {tabs}
    {activeTab}
    onSelectTab={onSelectTab}
    onOpenSettings={onboarding ? undefined : () => onSelectScreen("settings")}
  />
  {#if onboarding}
    <div class="pf-app-body">
      <div class="pf-main">
        <div class="pf-stage">
          <Onboarding
            snapshot={settingsSnapshot}
            loading={settingsLoading}
            remoteEnabled={remoteConnection.enabled}
            busyProviderId={authBusyProviderId}
            errorMessage={authError}
            externals={externalCredentials}
            busyImportKey={importBusyKey}
            onLoginOauth={(providerId) => void handleOauthLogin(providerId)}
            onLoginApiKey={(providerId, apiKey) => void handleApiKeyLogin(providerId, apiKey)}
            onImportExternal={(providerId, source) =>
              void handleImportExternal(providerId, source)}
            onRefresh={() => void refreshSettings()}
            forceRepoStep={forceOnboarding}
          />
        </div>
      </div>
    </div>
  {:else}
    <div class="pf-app-body">
      {#if tweaks.showSidebar}
        <Sidebar
          screen={tweaks.screen}
          onSelectScreen={onSelectScreen}
          agents={activeAgents}
          activeAgentId={selectedSession?.id ?? null}
          onOpenAgent={onOpenAgent}
          user={userChip}
        />
      {/if}
      <div class="pf-main">
        <div class="pf-stage">
          {#if tweaks.screen === "workspace"}
            {#if openAgentSessionId}
              <AgentDetail
                session={selectedSession}
                sessionDetail={sessionDetail}
                timeline={combinedTimeline}
                pendingPermissions={pendingPermissions}
                loading={sessionLoading}
                turnRunning={!!currentTurnId}
                onBack={onCloseAgent}
                onSubmitMessage={submitMessage}
                onResolvePermission={resolvePermission}
                onCancelTurn={() => { if (currentTurnId) void cancelTurn(currentTurnId); }}
              />
            {:else}
              <Workspace
                groups={groups}
                defaultWorkspaceCwd={defaultWorkspaceCwd}
                loading={groupsLoading}
                onOpenAgent={(id) => onOpenAgent(id)}
                onOpenBoard={(id) => { console.log("open board", id); }}
                onNewAgent={(cwd) => handleNewAgent(cwd)}
                onSessionReady={(sessionId) => handleSessionReady(sessionId)}
                onOpenWorkspacePicker={() => (showWorkspacePicker = true)}
              />
            {/if}
          {:else if tweaks.screen === "pipelines"}
            <Pipelines />
          {:else if tweaks.screen === "deployments"}
            <Deployments />
          {:else if tweaks.screen === "settings"}
            <Settings
              snapshot={settingsSnapshot}
              loading={settingsLoading}
              preferences={desktopPreferences}
              remoteEnabled={remoteConnection.enabled}
              remotePassword={remotePassword}
              remoteBusy={remoteBusy}
              remoteResult={remoteOperation}
              onPreferenceChange={updateDesktopPreference}
              onRemotePasswordChange={(value) => (remotePassword = value)}
              onResetPreferences={resetDesktopPreferences}
              onRefresh={() => void refreshSettings()}
              onLogout={(providerId) => void handleLogout(providerId)}
              onApiKeyLogin={(providerId, apiKey) => void handleApiKeyLogin(providerId, apiKey)}
              onRunRemoteBash={(command) => void handleRemoteBash(command)}
              onReadRemoteFile={(path) => void handleRemoteRead(path)}
              onWriteRemoteFile={(path, contents) => void handleRemoteWrite(path, contents)}
            />
          {/if}
        </div>
      </div>
    </div>
  {/if}

  <TweaksPanel tweaks={tweaks} onChange={updateTweak} />
</div>

{#if showWorkspacePicker}
  <WorkspacePicker
    onClose={() => (showWorkspacePicker = false)}
    onSwitched={async (hs) => {
      showWorkspacePicker = false;
      // Daemon has swapped — reload the default workspace + groups so the
      // UI reflects the new session store.
      defaultWorkspaceCwd = hs.workspaceRoot;
      selectedSession = null;
      sessionDetail = null;
      submittedMessages = [];
      liveStreamItems = [];
      turnPermissionLookup = {};
      await refreshGroups();
      statusMessage = `Switched workspace to ${hs.workspaceRoot}.`;
    }}
  />
{/if}

{#if !skipOnboarding && !forceOnboarding && !onboarding && statusMessage && statusMessage !== "Desktop workspace ready." && statusMessage !== "Settings snapshot refreshed."}
  <div class="status-strip" aria-live="polite">{statusMessage}</div>
{/if}

{#if connectionState === "reconnecting" || connectionState === "closed"}
  <div class="connection-banner" role="status" aria-live="polite">
    {#if connectionState === "reconnecting"}
      <span class="dot"></span>
      Lost connection to Puffer daemon. Reconnecting…
    {:else}
      <span class="dot err"></span>
      Puffer daemon disconnected.
      <button
        type="button"
        class="sc-btn"
        data-variant="outline"
        data-size="sm"
        onclick={() => void ensureLocalDaemonClient().then((c) => c.connect()).catch(() => {})}
      >Reconnect</button>
    {/if}
  </div>
{/if}

<style>
  .status-strip {
    position: fixed;
    bottom: 8px;
    left: 12px;
    font-size: 11px;
    color: var(--muted-foreground);
    font-family: var(--font-mono);
    max-width: 60vw;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    pointer-events: none;
    z-index: 5;
  }
  .connection-banner {
    position: fixed;
    top: 8px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 14px;
    font-size: 12px;
    color: var(--foreground);
    background: color-mix(in oklab, oklch(0.72 0.18 70) 18%, var(--background));
    border: 1px solid color-mix(in oklab, oklch(0.72 0.18 70) 40%, var(--border));
    border-radius: 999px;
    box-shadow: var(--shadow-md);
    z-index: 80;
    font-family: var(--font-mono);
  }
  .connection-banner .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: oklch(0.72 0.18 70);
    animation: pf-breathe 1.6s ease-in-out infinite;
  }
  .connection-banner .dot.err {
    background: oklch(0.62 0.22 25);
    animation: none;
  }
</style>
