<script lang="ts">
  import { onMount } from "svelte";
  import SessionSidebar from "./lib/components/SessionSidebar.svelte";
  import ConversationPane from "./lib/components/ConversationPane.svelte";
  import DiffView from "./lib/components/DiffView.svelte";
  import SettingsView from "./lib/components/SettingsView.svelte";
  import LoginView from "./lib/components/LoginView.svelte";
  import {
    createPullRequest,
    loginWithApiKey,
    loginWithOauth,
    listGroupedSessions,
    loadSettingsSnapshot,
    loadSessionDetail,
    mergePullRequest,
    logoutProvider,
    readRemoteFile,
    refreshRepoStatus,
    runRemoteBash,
    writeRemoteFile
  } from "./lib/api/desktop";
  import type {
    AppView,
    DesktopPreferences,
    FolderGroup,
    PermissionTimelineItem,
    RemoteConnection,
    RemoteOperation,
    SessionDetail,
    SessionListItem,
    SettingsSnapshot,
    TimelineItem
  } from "./lib/types";

  let groups: FolderGroup[] = [];
  let selectedSession: SessionListItem | null = null;
  let sessionDetail: SessionDetail | null = null;
  let settingsSnapshot: SettingsSnapshot | null = null;
  let view: AppView = "workspace";
  let statusMessage = "Desktop workspace ready.";
  let groupsLoading = true;
  let sessionLoading = false;
  let settingsLoading = false;
  let actionBusy = false;
  let authBusyProviderId: string | null = null;
  let authError: string | null = null;
  let remoteOperation: RemoteOperation | null = null;
  let remoteBusy = false;
  let preferredSessionId: string | null = null;
  let remotePassword = "";
  let submittedMessages: TimelineItem[] = [];
  let dismissedPermissionIds: string[] = [];

  const defaultDesktopPreferences: DesktopPreferences = {
    rememberSession: true,
    rememberInspectorLayout: true,
    launchInspectorOpen: true,
    defaultInspectorTab: "latest-diff",
    defaultInspectorWidth: 50,
    remoteEnabled: false,
    remoteTarget: "",
    remoteCwd: ""
  };
  let desktopPreferences: DesktopPreferences = { ...defaultDesktopPreferences };

  const storageKeys = {
    sessionId: "puffer-desktop:selected-session",
    inspectorOpen: "puffer-desktop:inspector-open",
    inspectorTab: "puffer-desktop:inspector-tab",
    inspectorWidth: "puffer-desktop:inspector-width",
    prefs: "puffer-desktop:preferences"
  } as const;

  $: baseTimeline = sessionDetail?.timeline ?? [];
  $: timeline = [...baseTimeline, ...submittedMessages];
  $: conversationTimeline = timeline.filter((item) => item.kind !== "permission" && item.kind !== "diff");
  $: pendingPermissions = timeline.filter(
    (item) => item.kind === "permission" && !dismissedPermissionIds.includes(item.id)
  ) as PermissionTimelineItem[];
  $: pendingPermissionCount = timeline.filter((item) => item.kind === "permission").length;
  $: toolCount = timeline.filter((item) => item.kind === "tool").length;
  $: diffCount = timeline.filter((item) => item.kind === "diff").length;
  $: remoteConnection = {
    enabled:
      desktopPreferences.remoteEnabled && desktopPreferences.remoteTarget.trim().length > 0,
    target: desktopPreferences.remoteTarget.trim(),
    cwd: desktopPreferences.remoteCwd.trim(),
    password: remotePassword
  } satisfies RemoteConnection;

  function buildPrDefaults(session: SessionListItem) {
    return {
      title: session.displayName ?? session.title,
      body: [
        `Generated from session: ${session.title}`,
        session.note ? `Context: ${session.note}` : null
      ]
        .filter(Boolean)
        .join("\n")
    };
  }

  function restoreDesktopState() {
    if (typeof window === "undefined") {
      return;
    }
    const rawPrefs = window.localStorage.getItem(storageKeys.prefs);
    if (rawPrefs) {
      try {
        desktopPreferences = {
          ...defaultDesktopPreferences,
          ...JSON.parse(rawPrefs)
        };
      } catch {
        desktopPreferences = { ...defaultDesktopPreferences };
      }
    }
    preferredSessionId = desktopPreferences.rememberSession
      ? window.localStorage.getItem(storageKeys.sessionId)
      : null;
  }

  function persistDesktopState() {
    if (typeof window === "undefined") {
      return;
    }
    window.localStorage.setItem(storageKeys.prefs, JSON.stringify(desktopPreferences));
    if (desktopPreferences.rememberSession && selectedSession?.id) {
      window.localStorage.setItem(storageKeys.sessionId, selectedSession.id);
    } else if (!desktopPreferences.rememberSession) {
      window.localStorage.removeItem(storageKeys.sessionId);
    }
  }

  async function openSettingsView() {
    view = "settings";
    await refreshSettings();
  }

  async function refreshSettings() {
    settingsLoading = true;
    try {
      settingsSnapshot = await loadSettingsSnapshot(remoteConnection);
      if ((settingsSnapshot.auth?.length ?? 0) === 0) {
        view = "login";
      } else if (view === "login") {
        view = "workspace";
      }
      statusMessage = "Settings snapshot refreshed.";
    } catch (error) {
      statusMessage = String(error);
    } finally {
      settingsLoading = false;
    }
  }

  async function handleOauthLogin(providerId: string) {
    authBusyProviderId = providerId;
    authError = null;
    try {
      settingsSnapshot = await loginWithOauth(providerId, remoteConnection);
      view = "workspace";
      statusMessage = `Logged in to ${providerId}.`;
      await refreshGroups(preferredSessionId ?? undefined);
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
      settingsSnapshot = await loginWithApiKey(providerId, apiKey, remoteConnection);
      view = "workspace";
      statusMessage = `Stored API key for ${providerId}.`;
      await refreshGroups(preferredSessionId ?? undefined);
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
      settingsSnapshot = await logoutProvider(providerId, remoteConnection);
      statusMessage = `Logged out from ${providerId}.`;
      if ((settingsSnapshot.auth?.length ?? 0) === 0) {
        groups = [];
        selectedSession = null;
        sessionDetail = null;
        view = "login";
      }
    } catch (error) {
      authError = String(error);
      statusMessage = authError;
    } finally {
      authBusyProviderId = null;
    }
  }

  async function handleRemoteBash(command: string) {
    if (!remoteConnection.enabled) {
      return;
    }
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
    if (!remoteConnection.enabled) {
      return;
    }
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
    if (!remoteConnection.enabled) {
      return;
    }
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

  function updateDesktopPreference<K extends keyof DesktopPreferences>(
    key: K,
    value: DesktopPreferences[K]
  ) {
    desktopPreferences = { ...desktopPreferences, [key]: value };
  }

  function resetDesktopPreferences() {
    desktopPreferences = { ...defaultDesktopPreferences };
    preferredSessionId = null;
    if (typeof window !== "undefined") {
      window.localStorage.removeItem(storageKeys.sessionId);
      window.localStorage.setItem(storageKeys.prefs, JSON.stringify(desktopPreferences));
    }
    statusMessage = "Desktop preferences reset.";
  }

  async function openSession(session: SessionListItem) {
    sessionLoading = true;
    try {
      const detail = await loadSessionDetail(session.id, remoteConnection);
      selectedSession = detail.session;
      sessionDetail = detail;
      submittedMessages = [];
      dismissedPermissionIds = [];
      statusMessage = `Loaded ${detail.timeline.length} conversation items.`;
    } catch (error) {
      statusMessage = String(error);
    } finally {
      sessionLoading = false;
    }
  }

  async function refreshGroups(preferredSessionId?: string) {
    groupsLoading = true;
    try {
      groups = await listGroupedSessions(remoteConnection);
      const allSessions = groups.flatMap((group) => group.sessions);
      const selectedSessionId = selectedSession?.id ?? null;
      const nextSession =
        allSessions.find((session) => session.id === preferredSessionId) ??
        (selectedSessionId
          ? allSessions.find((session) => session.id === selectedSessionId)
          : null) ??
        allSessions[0] ??
        null;

      if (!nextSession) {
        selectedSession = null;
        sessionDetail = null;
        submittedMessages = [];
        dismissedPermissionIds = [];
        statusMessage = "No sessions found in this workspace yet.";
        return;
      }

      await openSession(nextSession);
    } catch (error) {
      statusMessage = String(error);
    } finally {
      groupsLoading = false;
    }
  }

  async function refreshSelectedRepo() {
    if (!selectedSession || !sessionDetail) {
      return;
    }
    actionBusy = true;
    try {
      const repoStatus = await refreshRepoStatus(selectedSession.id, remoteConnection);
      sessionDetail = { ...sessionDetail, repoStatus };
      statusMessage = "Repository status refreshed.";
    } catch (error) {
      statusMessage = String(error);
    } finally {
      actionBusy = false;
    }
  }

  async function runRepoAction(action: "create" | "merge") {
    if (!selectedSession || !sessionDetail) {
      return;
    }

    actionBusy = true;
    try {
      if (action === "create") {
        const defaults = buildPrDefaults(selectedSession);
        const result = await createPullRequest(
          selectedSession.id,
          defaults.title,
          defaults.body,
          remoteConnection
        );
        sessionDetail = { ...sessionDetail, repoStatus: result.repoStatus };
        statusMessage = result.message;
      } else {
        const result = await mergePullRequest(
          selectedSession.id,
          sessionDetail.repoStatus.pullRequest?.number,
          "merge",
          remoteConnection
        );
        sessionDetail = { ...sessionDetail, repoStatus: result.repoStatus };
        statusMessage = result.message;
      }
    } catch (error) {
      statusMessage = String(error);
    } finally {
      actionBusy = false;
    }
  }

  function submitMessage(message: string) {
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
    statusMessage = "Added a local draft message to the transcript.";
  }

  function resolvePermission(permissionId: string, choice: string) {
    dismissedPermissionIds = [...dismissedPermissionIds, permissionId];
    statusMessage = `${choice} selected for the pending permission request.`;
  }

  onMount(() => {
    restoreDesktopState();
    void (async () => {
      await refreshSettings();
      if (view !== "login") {
        await refreshGroups(preferredSessionId ?? undefined);
      }
    })();
  });

  $: persistDesktopState();
</script>

<div class:single-column={view !== "workspace"} class="shell">
  {#if view === "workspace"}
    <SessionSidebar
      groups={groups}
      activeSessionId={selectedSession?.id ?? null}
      loading={groupsLoading}
      onSelect={(session) => void openSession(session)}
    />
  {/if}

  <main class="workspace">
    {#if view === "workspace"}
      <div
        class:loading={sessionLoading}
        class:has-diff={Boolean(sessionDetail?.latestDiff)}
        class:no-diff={!sessionDetail?.latestDiff}
        class="content workspace-split"
      >
        <ConversationPane
          session={selectedSession}
          timeline={conversationTimeline}
          loading={sessionLoading}
          {pendingPermissions}
          onSubmitMessage={submitMessage}
          onResolvePermission={resolvePermission}
        />

        <section class="diff-pane">
          {#if sessionDetail?.latestDiff}
            <DiffView diff={sessionDetail.latestDiff} />
          {:else}
            <div class="empty-diff">
              <strong>No diff captured</strong>
              <span>The latest session diff will appear here in GitHub-style format.</span>
            </div>
          {/if}
        </section>

        {#if sessionLoading}
          <div class="loading-overlay">
            <div class="loading-card">
              <strong>Loading session</strong>
              <span>Refreshing transcript, diffs, and repository state.</span>
            </div>
          </div>
        {/if}
      </div>
    {:else if view === "settings"}
      <SettingsView
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
        onRunRemoteBash={(command) => void handleRemoteBash(command)}
        onReadRemoteFile={(path) => void handleRemoteRead(path)}
        onWriteRemoteFile={(path, contents) => void handleRemoteWrite(path, contents)}
      />
    {:else}
      <LoginView
        snapshot={settingsSnapshot}
        loading={settingsLoading}
        remoteEnabled={remoteConnection.enabled}
        busyProviderId={authBusyProviderId}
        errorMessage={authError}
        onLoginOauth={(providerId) => void handleOauthLogin(providerId)}
        onLoginApiKey={(providerId, apiKey) => void handleApiKeyLogin(providerId, apiKey)}
        onRefresh={() => void refreshSettings()}
      />
    {/if}
  </main>
</div>
