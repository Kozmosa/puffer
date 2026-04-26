import { invoke } from "@tauri-apps/api/core";
import type {
  AuthProviderStatus,
  DiffSnapshot,
  ExternalCredential,
  FolderGroup,
  ProviderSummary,
  PullRequest,
  RemoteConnection,
  RemoteOperation,
  RepoActionResult,
  RepoStatus,
  SessionDetail,
  SessionListItem,
  SettingsSnapshot,
  TimelineItem
} from "../types";
import {
  mockCreatePrResult,
  mockFolders,
  mockMergePrResult,
  mockRepoStatus,
  mockSessionDetail,
  mockSessionDetailFor,
  mockSettingsSnapshot
} from "../mockData";

type BackendFolderGroup = {
  folderId: string;
  folderLabel: string;
  folderPath: string;
  sessionCount: number;
  sessions: BackendSessionListItem[];
};

type BackendSessionListItem = {
  sessionId: string;
  displayName: string | null;
  title: string;
  cwd: string;
  folderPath: string;
  updatedAtMs: number;
  createdAtMs: number;
  eventCount: number;
  slug: string | null;
  tags: string[];
  note: string | null;
  parentSessionId: string | null;
};

type BackendDiff = {
  id: string;
  source: string;
  commandLabel: string;
  statusText: string;
  unstagedDiffstat: string;
  stagedDiffstat: string;
  patch: string;
  patchExcerpt: string;
};

type BackendPullRequest = {
  number: number;
  title: string;
  url: string;
  state: string;
  isDraft: boolean;
  mergeStateStatus: string | null;
  headRefName: string | null;
  baseRefName: string | null;
};

type BackendRepoStatus = {
  sessionId: string;
  cwd: string;
  repoRoot: string | null;
  branch: string | null;
  headSha: string | null;
  isClean: boolean;
  statusLines: string[];
  hasGh: boolean;
  ghAuthenticated: boolean;
  canCreatePullRequest: boolean;
  canMergePullRequest: boolean;
  createPullRequestReason: string | null;
  mergePullRequestReason: string | null;
  openPullRequest: BackendPullRequest | null;
  warnings: string[];
};

type BackendRepoActionResult = {
  ok: boolean;
  action: string;
  message: string;
  repoStatus: BackendRepoStatus;
  pullRequest: BackendPullRequest | null;
};

type BackendTimelineItem =
  | { kind: "user_message"; id: string; text: string }
  | { kind: "assistant_message"; id: string; text: string }
  | { kind: "system_message"; id: string; text: string }
  | { kind: "command"; id: string; commandName: string; commandArgs: string }
  | {
      kind: "tool_call";
      id: string;
      toolId: string;
      status: string;
      summary: string | null;
      inputText: string;
      inputJson: Record<string, unknown> | null;
      outputText: string;
    }
  | {
      kind: "permission_dialog";
      id: string;
      toolId: string;
      state: string;
      summary: string | null;
      reason: string;
      inputText: string | null;
    }
  | { kind: "diff_snapshot"; id: string; snapshot: BackendDiff };

type BackendAgentDiffFile = {
  path: string;
  latestKind: string;
  editCount: number;
  latestSummary: string;
};

type BackendAgentDiffEntry = {
  callId: string;
  toolId: string;
  kind: string;
  path: string;
  success: boolean;
  summary: string;
};

type BackendAgentDiff = {
  files: BackendAgentDiffFile[];
  entries: BackendAgentDiffEntry[];
};

type BackendDivergenceReport = {
  agentOnly: string[];
  gitOnly: string[];
  agentTotal: number;
  gitTotal: number;
};

type BackendSessionDetail = BackendSessionListItem & {
  timeline: BackendTimelineItem[];
  latestDiff: BackendDiff | null;
  diffHistory: BackendDiff[];
  repoStatus: BackendRepoStatus;
  agentDiff: BackendAgentDiff;
  divergence: BackendDivergenceReport;
};

function remoteArgs(remote?: RemoteConnection): Record<string, unknown> {
  if (!remote || !remote.enabled || !remote.target.trim()) {
    return {};
  }
  return {
    remoteTarget: remote.target,
    remoteCwd: remote.cwd || null,
    remotePassword: remote.password || null
  };
}

type BackendSettingsConfig = SettingsSnapshot["config"];
type BackendResourceCounts = SettingsSnapshot["resources"];
type BackendSettingsSessionSummary = SettingsSnapshot["sessions"];
type BackendAuthProviderStatus = AuthProviderStatus;
type BackendProviderSummary = ProviderSummary;

type BackendSettingsSnapshot = {
  workspaceRoot: string;
  workspaceConfigFile: string;
  userConfigFile: string;
  authStoreFile: string;
  builtinResourcesDir: string;
  config: BackendSettingsConfig;
  resources: BackendResourceCounts;
  sessions: BackendSettingsSessionSummary;
  auth: BackendAuthProviderStatus[];
  providers: BackendProviderSummary[];
};

type BackendRemoteOperation = RemoteOperation;

function canInvokeTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Exposed to Svelte components so they can branch on whether the daemon
 *  is reachable (Tauri desktop shell) vs. running in pure web preview. In
 *  preview mode calls to `ensureLocalDaemonClient` throw — UIs that need
 *  live data should skip the RPC and render a friendly banner instead. */
export function isDaemonReachable(): boolean {
  return canInvokeTauri();
}

function preview(text: string, maxLength = 160): string {
  return text.length > maxLength ? `${text.slice(0, maxLength).trimEnd()}...` : text;
}

function normalizePullRequest(value: BackendPullRequest | null): PullRequest | null {
  if (!value) {
    return null;
  }
  return {
    number: value.number,
    title: value.title,
    url: value.url,
    state: value.state,
    isDraft: value.isDraft,
    mergeStateStatus: value.mergeStateStatus,
    headRefName: value.headRefName,
    baseRefName: value.baseRefName
  };
}

function normalizeRepoStatus(value: BackendRepoStatus): RepoStatus {
  return {
    sessionId: value.sessionId,
    cwd: value.cwd,
    isGitRepo: value.repoRoot !== null,
    repoRoot: value.repoRoot,
    branch: value.branch,
    headSha: value.headSha,
    isClean: value.isClean,
    hasUncommittedChanges: !value.isClean,
    statusLines: value.statusLines,
    ghAvailable: value.hasGh,
    ghAuthenticated: value.ghAuthenticated,
    canCreatePr: value.canCreatePullRequest,
    canMergePr: value.canMergePullRequest,
    createPrReason: value.createPullRequestReason,
    mergePrReason: value.mergePullRequestReason,
    pullRequest: normalizePullRequest(value.openPullRequest),
    warnings: value.warnings
  };
}

function normalizeDiff(value: BackendDiff): DiffSnapshot {
  return {
    id: value.id,
    source: value.source,
    title: value.commandLabel,
    command: value.commandLabel,
    status: value.statusText,
    unstagedDiffstat: value.unstagedDiffstat,
    stagedDiffstat: value.stagedDiffstat,
    patch: value.patch || value.patchExcerpt
  };
}

function normalizeSessionListItem(value: BackendSessionListItem): SessionListItem {
  return {
    id: value.sessionId,
    displayName: value.displayName,
    title: value.title,
    cwd: value.cwd,
    folderPath: value.folderPath,
    updatedAtMs: value.updatedAtMs,
    createdAtMs: value.createdAtMs,
    eventCount: value.eventCount,
    slug: value.slug,
    tags: value.tags,
    note: value.note,
    parentSessionId: value.parentSessionId
  };
}

function normalizeTimelineItem(value: BackendTimelineItem): TimelineItem {
  switch (value.kind) {
    case "user_message":
      return {
        id: value.id,
        kind: "user",
        title: "User message",
        summary: preview(value.text),
        body: value.text,
        meta: []
      };
    case "assistant_message":
      return {
        id: value.id,
        kind: "assistant",
        title: "Assistant response",
        summary: preview(value.text),
        body: value.text,
        meta: []
      };
    case "system_message":
      return {
        id: value.id,
        kind: "system",
        title: "System message",
        summary: preview(value.text),
        body: value.text,
        meta: []
      };
    case "command":
      return {
        id: value.id,
        kind: "command",
        title: `/${value.commandName}`,
        summary: preview(value.commandArgs || `/${value.commandName}`),
        body: [value.commandName, value.commandArgs].filter(Boolean).join(" "),
        meta: ["slash command"]
      };
    case "tool_call":
      return {
        id: value.id,
        kind: "tool",
        title: `Tool call: ${value.toolId}`,
        summary: value.summary ?? preview(value.outputText || value.inputText),
        body: value.outputText || "Tool call completed without textual output.",
        meta: [value.toolId, value.status],
        toolName: value.toolId,
        status: value.status,
        input: value.inputText,
        output: value.outputText,
        inputJson: value.inputJson
      };
    case "permission_dialog":
      return {
        id: value.id,
        kind: "permission",
        title: "Permission request",
        summary: value.summary ?? `${value.toolId} requires approval`,
        body: `Tool: ${value.toolId}\nReason: ${value.reason}`,
        meta: [value.state],
        toolName: value.toolId,
        status: value.state,
        permissionDialog: {
          state: value.state,
          reason: value.reason,
          summary: value.summary,
          inputText: value.inputText,
          toolName: value.toolId,
          choices: ["Allow once", "Allow for session", "Deny"]
        },
        scopeLabel: "workspace",
        choices: ["Allow once", "Allow for session", "Deny"]
      };
    case "diff_snapshot": {
      const diff = normalizeDiff(value.snapshot);
      return {
        id: value.id,
        kind: "diff",
        title: diff.title,
        summary: diff.status,
        body: diff.patch,
        meta: [diff.command],
        diff
      };
    }
  }
}

function normalizeSessionDetail(value: BackendSessionDetail): SessionDetail {
  const session = normalizeSessionListItem(value);
  // Older daemons may not emit agentDiff/divergence yet — fall back to
  // empty defaults so the UI still renders a sensible (empty) diff tab
  // instead of throwing on undefined.
  const agentDiff = value.agentDiff ?? { files: [], entries: [] };
  const divergence = value.divergence ?? {
    agentOnly: [],
    gitOnly: [],
    agentTotal: 0,
    gitTotal: 0
  };
  return {
    session,
    timeline: value.timeline.map(normalizeTimelineItem),
    latestDiff: value.latestDiff ? normalizeDiff(value.latestDiff) : null,
    diffHistory: value.diffHistory.map(normalizeDiff),
    repoStatus: normalizeRepoStatus(value.repoStatus),
    agentDiff,
    divergence
  };
}

export async function listGroupedSessions(remote?: RemoteConnection): Promise<FolderGroup[]> {
  if (!canInvokeTauri()) {
    return mockFolders;
  }
  const response = await invoke<BackendFolderGroup[]>("list_grouped_sessions", remoteArgs(remote));
  return response.map((group) => ({
    id: group.folderId,
    label: group.folderLabel,
    path: group.folderPath,
    sessionCount: group.sessionCount,
    sessions: group.sessions.map(normalizeSessionListItem)
  }));
}

export async function loadSessionDetail(
  sessionId: string,
  remote?: RemoteConnection
): Promise<SessionDetail> {
  if (!canInvokeTauri()) {
    return mockSessionDetailFor(sessionId);
  }
  const response = await invoke<BackendSessionDetail>("load_session_detail", {
    sessionId,
    ...remoteArgs(remote)
  });
  return normalizeSessionDetail(response);
}

export async function refreshRepoStatus(
  sessionId: string,
  remote?: RemoteConnection
): Promise<RepoStatus> {
  if (!canInvokeTauri()) {
    return mockRepoStatus;
  }
  const response = await invoke<BackendRepoStatus>("refresh_repo_status", {
    sessionId,
    ...remoteArgs(remote)
  });
  return normalizeRepoStatus(response);
}

export async function createPullRequest(
  sessionId: string,
  title?: string,
  body?: string,
  remote?: RemoteConnection
): Promise<RepoActionResult> {
  if (!canInvokeTauri()) {
    return mockCreatePrResult();
  }
  const response = await invoke<BackendRepoActionResult>("create_pull_request", {
    sessionId,
    title: title ?? null,
    body: body ?? null,
    ...remoteArgs(remote)
  });
  return {
    ok: response.ok,
    action: response.action,
    message: response.message,
    repoStatus: normalizeRepoStatus(response.repoStatus),
    pullRequest: normalizePullRequest(response.pullRequest)
  };
}

export async function mergePullRequest(
  sessionId: string,
  pullRequestNumber?: number,
  mergeMethod?: string,
  remote?: RemoteConnection
): Promise<RepoActionResult> {
  if (!canInvokeTauri()) {
    return mockMergePrResult();
  }
  const response = await invoke<BackendRepoActionResult>("merge_pull_request", {
    sessionId,
    pullRequestNumber: pullRequestNumber ?? null,
    mergeMethod: mergeMethod ?? null,
    ...remoteArgs(remote)
  });
  return {
    ok: response.ok,
    action: response.action,
    message: response.message,
    repoStatus: normalizeRepoStatus(response.repoStatus),
    pullRequest: normalizePullRequest(response.pullRequest)
  };
}

export async function loadSettingsSnapshot(remote?: RemoteConnection): Promise<SettingsSnapshot> {
  if (!canInvokeTauri()) {
    return mockSettingsSnapshot;
  }
  return invoke<BackendSettingsSnapshot>("load_settings_snapshot", remoteArgs(remote));
}

export async function loginWithOauth(
  providerId: string,
  remote?: RemoteConnection
): Promise<SettingsSnapshot> {
  if (!canInvokeTauri()) {
    return mockSettingsSnapshot;
  }
  return invoke<BackendSettingsSnapshot>("login_with_oauth", {
    providerId,
    ...remoteArgs(remote)
  });
}

export async function loginWithApiKey(
  providerId: string,
  apiKey: string,
  remote?: RemoteConnection
): Promise<SettingsSnapshot> {
  if (!canInvokeTauri()) {
    return mockSettingsSnapshot;
  }
  return invoke<BackendSettingsSnapshot>("login_with_api_key", {
    providerId,
    apiKey,
    ...remoteArgs(remote)
  });
}

/** Lists credentials Puffer can adopt without an interactive flow — typically
 *  whatever is already stored under `~/.claude` or `~/.codex`. The desktop
 *  shell surfaces these so the user does not have to paste an API key they
 *  already have on disk. */
export async function listExternalCredentials(): Promise<ExternalCredential[]> {
  if (!canInvokeTauri()) return [];
  return invoke<ExternalCredential[]>("list_external_credentials");
}

/** Adopts a credential discovered by `listExternalCredentials` for the given
 *  provider, then returns the refreshed settings snapshot. */
export async function importExternalCredential(
  providerId: string,
  source: "claude" | "codex"
): Promise<SettingsSnapshot> {
  if (!canInvokeTauri()) return mockSettingsSnapshot;
  return invoke<BackendSettingsSnapshot>("import_external_credential", {
    providerId,
    source
  });
}

export async function logoutProvider(
  providerId: string,
  remote?: RemoteConnection
): Promise<SettingsSnapshot> {
  if (!canInvokeTauri()) {
    return mockSettingsSnapshot;
  }
  return invoke<BackendSettingsSnapshot>("logout_provider", {
    providerId,
    ...remoteArgs(remote)
  });
}

export async function runRemoteBash(
  remote: RemoteConnection,
  command: string
): Promise<RemoteOperation> {
  if (!canInvokeTauri()) {
    return { success: true, stdout: "mock remote bash\n", stderr: "" };
  }
  return invoke<BackendRemoteOperation>("run_remote_bash", {
    remoteTarget: remote.target,
    remoteCwd: remote.cwd || null,
    remotePassword: remote.password || null,
    command
  });
}

export async function readRemoteFile(
  remote: RemoteConnection,
  path: string
): Promise<RemoteOperation> {
  if (!canInvokeTauri()) {
    return { success: true, stdout: "mock remote file\n", stderr: "" };
  }
  return invoke<BackendRemoteOperation>("read_remote_file", {
    remoteTarget: remote.target,
    remoteCwd: remote.cwd || null,
    remotePassword: remote.password || null,
    path
  });
}

export async function writeRemoteFile(
  remote: RemoteConnection,
  path: string,
  contents: string
): Promise<RemoteOperation> {
  if (!canInvokeTauri()) {
    return { success: true, stdout: "", stderr: "" };
  }
  return invoke<BackendRemoteOperation>("write_remote_file", {
    remoteTarget: remote.target,
    remoteCwd: remote.cwd || null,
    remotePassword: remote.password || null,
    path,
    contentsBase64: btoa(unescape(encodeURIComponent(contents)))
  });
}

// ============================================================================
// Daemon-backed workspace + runtime. The daemon is authoritative: Puffer owns
// sessions, transcripts, and provider state. This module is a thin adapter.
// ============================================================================

import { ensureLocalDaemonClient, switchDaemonClient } from "./daemonClient";

/** A blank session created on the fly. The daemon places it in the given
 *  `cwd` (defaults to the daemon's boot workspace). Returns the session id
 *  so the UI can open an `AgentDetail` immediately. */
export async function createSession(cwd?: string): Promise<{
  sessionId: string;
  cwd: string;
  createdAtMs: number;
}> {
  const client = await ensureLocalDaemonClient();
  return client.request("create_session", cwd ? { cwd } : {});
}

/** The daemon's boot workspace — useful for showing "new agent in <path>" in
 *  the UI so the user isn't surprised where their session lands. */
export async function loadDefaultWorkspace(): Promise<{
  cwd: string;
  workspaceRoot: string;
}> {
  const client = await ensureLocalDaemonClient();
  return client.request("default_workspace");
}

/** Result of a completed git clone — fired via the `clone:<id>:done`
 *  channel and resolved through the `done` promise. */
export type GitCloneDone = {
  ok: boolean;
  dest: string;
  stdout: string;
  stderr: string;
  exitCode: number | null;
};

/** Handle returned from `cloneRepo`. The `cloneId` + `dest` land
 *  synchronously; `done` resolves once git exits. Subscribe to
 *  `onProgress(line)` to surface live stderr (git's `--progress` output)
 *  in the UI while the clone is still running. */
export type GitCloneHandle = {
  cloneId: string;
  dest: string;
  done: Promise<GitCloneDone>;
  onProgress(handler: (line: string) => void): () => void;
};

/** Clones a git repository into the daemon's filesystem and returns a
 *  handle that streams progress. Remote clones are a special case of
 *  "clone on the currently-connected daemon" — the caller connects to
 *  the SSH daemon first (via connectSshDaemon) and the clone lands on
 *  that remote machine.
 *
 *  The daemon RPC returns `{cloneId, dest}` IMMEDIATELY, before the
 *  clone finishes; progress arrives on `clone:<id>:progress` events and
 *  the final status on `clone:<id>:done`. */
export async function cloneRepo(
  url: string,
  dest: string,
  options: { depth?: number } = {}
): Promise<GitCloneHandle> {
  const client = await ensureLocalDaemonClient();
  const start = await client.request<{ cloneId: string; dest: string }>("git_clone", {
    url,
    dest,
    ...(options.depth != null ? { depth: options.depth } : {})
  });
  const progressChannel = `clone:${start.cloneId}:progress`;
  const doneChannel = `clone:${start.cloneId}:done`;
  const done = new Promise<GitCloneDone>((resolve) => {
    const offDone = client.on<{
      cloneId: string;
      ok: boolean;
      dest: string;
      stdout: string;
      stderr: string;
      exitCode: number | null;
    }>(doneChannel, (payload) => {
      offDone();
      resolve({
        ok: payload.ok,
        dest: payload.dest,
        stdout: payload.stdout,
        stderr: payload.stderr,
        exitCode: payload.exitCode
      });
    });
  });
  return {
    cloneId: start.cloneId,
    dest: start.dest,
    done,
    onProgress(handler: (line: string) => void) {
      return client.on<{ line: string; cloneId: string }>(progressChannel, (payload) => {
        if (payload && typeof payload.line === "string") handler(payload.line);
      });
    }
  };
}

/** Tears down the current local daemon and spawns a fresh one rooted at
 *  `cwd`, then swaps the shared DaemonClient to the new handshake.
 *  Returns the new handshake so callers can show "now in <workspaceRoot>"
 *  after the switch. Used by the WorkspacePicker. */
export async function restartLocalDaemon(cwd: string): Promise<{
  url: string;
  token: string;
  workspaceRoot: string;
  protocolVersion: string;
}> {
  if (!canInvokeTauri()) {
    throw new Error("Switching local workspace requires the Tauri desktop shell.");
  }
  const handshake = await invoke<{
    url: string;
    token: string;
    protocolVersion: string;
    workspaceRoot: string;
  }>("restart_local_daemon", { cwd });
  await switchDaemonClient(handshake);
  return handshake;
}

/** Starts a remote `puffer daemon` over SSH and makes the app's shared
 *  daemon client connect to it. Returns the handshake (now pointing at a
 *  local forwarded port) so the UI can show the remote workspace path. */
export async function connectSshDaemon(
  sshTarget: string,
  options: { remoteBinary?: string; remoteWorkspace?: string } = {}
): Promise<{ url: string; token: string; workspaceRoot: string; protocolVersion: string }> {
  if (!canInvokeTauri()) {
    throw new Error("SSH remote daemon requires the Tauri desktop shell.");
  }
  const handshake = await invoke<{
    url: string;
    token: string;
    protocolVersion: string;
    workspaceRoot: string;
  }>("start_ssh_daemon", {
    sshTarget,
    remoteBinary: options.remoteBinary ?? null,
    remoteWorkspace: options.remoteWorkspace ?? null
  });
  await switchDaemonClient(handshake);
  return handshake;
}

/** Refresh the grouped-sessions view from the daemon. Used whenever the
 *  workspace board needs to re-read state after create/mutate. Falls back
 *  to mock folders in the pure-web preview (no daemon reachable) so the
 *  design screenshots + visual review keep working without a backend. */
export async function listGroupedSessionsFromDaemon(): Promise<FolderGroup[]> {
  try {
    const client = await ensureLocalDaemonClient();
    const raw = await client.request<BackendFolderGroup[]>("list_grouped_sessions");
    return raw.map((folder) => ({
      id: folder.folderId,
      label: folder.folderLabel,
      path: folder.folderPath,
      sessionCount: folder.sessionCount,
      sessions: folder.sessions.map(normalizeSessionListItem)
    }));
  } catch (_error) {
    if (!canInvokeTauri()) return mockFolders;
    throw _error;
  }
}

/** Load one session's detail (transcript + latest diff + repo state) via
 *  the daemon. Falls back to mock detail in web preview mode. */
export async function loadSessionDetailFromDaemon(
  sessionId: string
): Promise<SessionDetail> {
  try {
    const client = await ensureLocalDaemonClient();
    const raw = await client.request<BackendSessionDetail>("load_session_detail", {
      sessionId
    });
    return normalizeSessionDetail(raw);
  } catch (_error) {
    if (!canInvokeTauri()) return mockSessionDetailFor(sessionId) ?? mockSessionDetail;
    throw _error;
  }
}

export type PermissionAction = "allow_once" | "allow_session" | "allow_all_session" | "deny";

/** Starts a new agent turn on `sessionId` with `message`. Returns the turn id
 *  so the caller can correlate streamed events and reply to permission
 *  prompts. Subscribe to `subscribeSessionEvents(sessionId, handler)` to see
 *  events as the turn runs. */
export async function runAgentTurn(sessionId: string, message: string): Promise<string> {
  try {
    const client = await ensureLocalDaemonClient();
    const result = await client.request<{ turnId: string }>("run_agent_turn", {
      sessionId,
      message
    });
    return result.turnId;
  } catch (daemonError) {
    if (!canInvokeTauri()) throw daemonError;
    // Fallback: the in-process Tauri command (same behavior, just no daemon).
    return invoke<string>("run_agent_turn", { sessionId, message });
  }
}

/** Resolves a pending permission prompt for an in-flight turn. */
export async function resolvePermission(
  turnId: string,
  requestId: string,
  action: PermissionAction
): Promise<void> {
  try {
    const client = await ensureLocalDaemonClient();
    await client.request("resolve_permission", { turnId, requestId, action });
    return;
  } catch (daemonError) {
    if (!canInvokeTauri()) throw daemonError;
    await invoke("resolve_permission", { turnId, requestId, action });
  }
}

/** Best-effort cancel: the current model/tool step completes then the turn
 *  exits. Any pending permission is treated as Deny. */
export async function cancelTurn(turnId: string): Promise<void> {
  try {
    const client = await ensureLocalDaemonClient();
    await client.request("cancel_turn", { turnId });
    return;
  } catch (daemonError) {
    if (!canInvokeTauri()) throw daemonError;
    await invoke("cancel_turn", { turnId });
  }
}

/** Stores an API key credential in the workspace auth store via the daemon
 *  and returns the refreshed settings snapshot. Falls back to the legacy
 *  Tauri-invoke path if the daemon is unreachable. */
export async function loginWithApiKeyViaDaemon(
  providerId: string,
  apiKey: string
): Promise<SettingsSnapshot> {
  try {
    const client = await ensureLocalDaemonClient();
    return await client.request<SettingsSnapshot>("login_with_api_key", {
      providerId,
      apiKey
    });
  } catch (daemonError) {
    if (!canInvokeTauri()) throw daemonError;
    return loginWithApiKey(providerId, apiKey);
  }
}

/** Removes stored credentials for a provider via the daemon and returns the
 *  refreshed settings snapshot. Falls back to Tauri-invoke when the daemon
 *  is unreachable. */
export async function logoutProviderViaDaemon(
  providerId: string
): Promise<SettingsSnapshot> {
  try {
    const client = await ensureLocalDaemonClient();
    return await client.request<SettingsSnapshot>("logout_provider", {
      providerId
    });
  } catch (daemonError) {
    if (!canInvokeTauri()) throw daemonError;
    return logoutProvider(providerId);
  }
}

// ---------------------------------------------------------------------------
// Read-only filesystem RPCs for the Files tab.
// ---------------------------------------------------------------------------

export type DirEntryKind = "file" | "directory" | "symlink";

export type DirEntry = {
  name: string;
  kind: DirEntryKind;
  size: number;
  modifiedMs: number;
};

export type ReadFileResult = {
  path: string;
  encoding: "utf8" | "base64";
  content: string;
  size: number;
  truncated: boolean;
};

/** List one directory. Absolute path required. The daemon enforces an
 *  allowlist (session cwd, workspace root, $HOME), so paths outside those
 *  roots error out. Entries come back sorted dirs-first, then files. */
export async function listDir(path: string): Promise<DirEntry[]> {
  const client = await ensureLocalDaemonClient();
  const result = await client.request<{ entries: DirEntry[] }>("list_dir", { path });
  return result.entries;
}

/** Read a file. `maxBytes` caps the returned content (default 256 KiB);
 *  larger files are truncated and returned with `truncated: true`. Files
 *  larger than 5 MiB are refused outright with an error. Binary files
 *  come back base64-encoded with `encoding: "base64"`. */
export async function readFile(path: string, maxBytes?: number): Promise<ReadFileResult> {
  const client = await ensureLocalDaemonClient();
  return client.request<ReadFileResult>(
    "read_file",
    maxBytes != null ? { path, maxBytes } : { path }
  );
}

/** Start a filesystem watch. `paths` must live under the daemon's allowlist
 *  (session cwd / workspace root / $HOME). The daemon fires
 *  `workspace:fs:changed` events with `{watchId, paths}` debounced on a 100 ms
 *  window. Recursive watches follow into every subdirectory. Dispose via
 *  `fsUnwatch(watchId)` on unmount to free the native watcher. */
export async function fsWatch(
  paths: string[],
  recursive: boolean = true
): Promise<{ watchId: string }> {
  const client = await ensureLocalDaemonClient();
  return client.request<{ watchId: string }>("fs_watch", { paths, recursive });
}

/** Stop a filesystem watch. Idempotent — a stale id is a no-op. */
export async function fsUnwatch(watchId: string): Promise<void> {
  const client = await ensureLocalDaemonClient();
  await client.request("fs_unwatch", { watchId });
}

/** Payload shape for `workspace:fs:changed` events. `replay: true` means the
 *  daemon is re-delivering the event after a reconnect — a newly-mounted
 *  FilesPane has no cache yet and should ignore replays. */
export type FsChangedEvent = {
  watchId: string;
  paths: string[];
  changedAtMs: number;
  replay?: boolean;
};

// ---------------------------------------------------------------------------
// PTY — user-facing Terminal tab. The daemon spawns a shell in the session's
// cwd; data is base64-encoded on the wire to stay ASCII-safe regardless of
// shell output. One pty_id per open tab.
// ---------------------------------------------------------------------------

/** Open a new PTY in `cwd` (defaults to the daemon's cwd). Returns the
 *  opaque pty_id to use on subsequent write/resize/close calls and to
 *  subscribe to `pty:<id>:data` + `pty:<id>:exit` events. */
export async function openPty(params: {
  cwd?: string;
  cols?: number;
  rows?: number;
}): Promise<{ ptyId: string }> {
  const client = await ensureLocalDaemonClient();
  return client.request<{ ptyId: string }>("pty_open", params);
}

/** Send keystrokes to the PTY. `dataB64` is the base64-encoding of the
 *  raw bytes (typically the UTF-8 of xterm's onData string). */
export async function writePty(ptyId: string, dataB64: string): Promise<void> {
  const client = await ensureLocalDaemonClient();
  await client.request("pty_write", { ptyId, data: dataB64 });
}

/** Resize the PTY; xterm's FitAddon supplies cols/rows after measuring. */
export async function resizePty(
  ptyId: string,
  cols: number,
  rows: number
): Promise<void> {
  const client = await ensureLocalDaemonClient();
  await client.request("pty_resize", { ptyId, cols, rows });
}

/** Tear down the PTY. Idempotent — calling on an already-exited pty_id is
 *  a no-op on the daemon side. */
export async function closePty(ptyId: string): Promise<void> {
  const client = await ensureLocalDaemonClient();
  await client.request("pty_close", { ptyId });
}

// ---------------------------------------------------------------------------
// Settings persistence — MCP servers, provider models, permissions, config.
// All round-trip through the daemon so remote workspaces get the same UX.
// ---------------------------------------------------------------------------

export type McpServerInfo = {
  id: string;
  displayName: string;
  description: string;
  transport: string;
  endpoint: string;
  target: string;
  sourceKind: string;
  sourcePath: string | null;
};

export type ModelDescriptorInfo = {
  id: string;
  displayName: string;
  provider: string;
  api: string;
  contextWindow: number;
  maxOutputTokens: number;
  supportsReasoning: boolean;
};

export type PermissionsSnapshot = {
  path: string;
  tools: Record<string, string>;
};

export type ConfigPatch = {
  defaultProvider?: string | null;
  defaultModel?: string | null;
  theme?: string;
  openaiBaseUrl?: string | null;
};

export async function listMcpServers(): Promise<McpServerInfo[]> {
  const client = await ensureLocalDaemonClient();
  const result = await client.request<{ servers: McpServerInfo[] }>("list_mcp_servers");
  return result.servers;
}

export async function listProviderModels(providerId: string): Promise<ModelDescriptorInfo[]> {
  const client = await ensureLocalDaemonClient();
  const result = await client.request<{ providerId: string; models: ModelDescriptorInfo[] }>(
    "list_provider_models",
    { providerId }
  );
  return result.models;
}

export async function listPermissions(): Promise<PermissionsSnapshot> {
  const client = await ensureLocalDaemonClient();
  return client.request<PermissionsSnapshot>("list_permissions");
}

export async function savePermissions(
  tools: Record<string, string>
): Promise<PermissionsSnapshot> {
  const client = await ensureLocalDaemonClient();
  return client.request<PermissionsSnapshot>("save_permissions", { tools });
}

/** Patch the user config file and return the fresh settings snapshot. The
 *  daemon reloads its own in-memory config under the lock, so subsequent
 *  turns pick up the new default_model without a daemon restart. */
export async function updateConfig(patch: ConfigPatch): Promise<SettingsSnapshot> {
  const client = await ensureLocalDaemonClient();
  return client.request<SettingsSnapshot>("update_config", patch);
}
