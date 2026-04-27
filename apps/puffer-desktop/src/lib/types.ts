export type InspectorTab = "latest-diff" | "history" | "tool-details";
export type AppView = "workspace" | "settings" | "login";

export type TimelineKind =
  | "user"
  | "assistant"
  | "system"
  | "tool"
  | "permission"
  | "question"
  | "diff"
  | "command";

export type FolderGroup = {
  id: string;
  label: string;
  path: string;
  sessionCount: number;
  sessions: SessionListItem[];
};

export type DesktopPinState = {
  pinnedAgentIds: string[];
  pinnedWorkspacePaths: string[];
};

export type SessionListItem = {
  id: string;
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

export type PullRequest = {
  number: number;
  title: string;
  url: string;
  state: string;
  isDraft: boolean;
  mergeStateStatus: string | null;
  headRefName: string | null;
  baseRefName: string | null;
};

export type RepoStatus = {
  sessionId: string;
  cwd: string;
  isGitRepo: boolean;
  repoRoot: string | null;
  branch: string | null;
  headSha: string | null;
  isClean: boolean;
  hasUncommittedChanges: boolean;
  statusLines: string[];
  ghAvailable: boolean;
  ghAuthenticated: boolean;
  canCreatePr: boolean;
  canMergePr: boolean;
  createPrReason: string | null;
  mergePrReason: string | null;
  pullRequest: PullRequest | null;
  warnings: string[];
};

export type RepoActionResult = {
  ok: boolean;
  action: string;
  message: string;
  repoStatus: RepoStatus;
  pullRequest: PullRequest | null;
};

export type PermissionDialog = {
  state: string;
  reason: string;
  summary: string | null;
  inputText: string | null;
  toolName: string | null;
  choices: string[];
};

export type DiffSnapshot = {
  id: string;
  source: string;
  title: string;
  command: string;
  status: string;
  unstagedDiffstat: string;
  stagedDiffstat: string;
  patch: string;
};

type TimelineBase = {
  id: string;
  kind: TimelineKind;
  title: string;
  summary: string;
  body: string;
  meta: string[];
  status?: string | null;
};

export type MessageTimelineItem = TimelineBase & {
  kind: "user" | "assistant" | "system" | "command";
};

export type ToolTimelineItem = TimelineBase & {
  kind: "tool";
  toolName: string;
  status: string;
  input: string;
  output: string;
  inputJson: Record<string, unknown> | null;
};

export type PermissionTimelineItem = TimelineBase & {
  kind: "permission";
  toolName: string | null;
  status: string;
  permissionDialog: PermissionDialog;
  scopeLabel: string | null;
  choices: string[];
};

export type AskUserQuestionOption = {
  label: string;
  description: string;
  preview?: string | null;
};

export type AskUserQuestionItem = {
  question: string;
  header: string;
  options: AskUserQuestionOption[];
  multiSelect?: boolean;
};

export type UserQuestionTimelineItem = TimelineBase & {
  kind: "question";
  status: string;
  questions: AskUserQuestionItem[];
  answers?: Record<string, string | string[]>;
};

export type DiffTimelineItem = TimelineBase & {
  kind: "diff";
  diff: DiffSnapshot;
};

export type TimelineItem =
  | MessageTimelineItem
  | ToolTimelineItem
  | PermissionTimelineItem
  | UserQuestionTimelineItem
  | DiffTimelineItem;

/** A single agent edit reconstructed from a tool-call transcript event.
 *  `kind` is the high-level operation (write/replace/move/remove);
 *  `summary` is a unified-diff-ish snippet rendered server-side. */
export type AgentDiffEntry = {
  callId: string;
  toolId: string;
  kind: string;
  path: string;
  success: boolean;
  summary: string;
};

/** Per-file rollup of the agent's edits — useful for "the agent
 *  touched these N files this session" lists. */
export type AgentDiffFile = {
  path: string;
  latestKind: string;
  editCount: number;
  latestSummary: string;
};

export type AgentDiff = {
  files: AgentDiffFile[];
  entries: AgentDiffEntry[];
};

/** Set difference between agent-touched and git-touched paths. Empty
 *  arrays mean the two views agree; non-empty means there's drift to
 *  surface (hand-edits, hook rewrites, rolled-back applies, …). */
export type DivergenceReport = {
  agentOnly: string[];
  gitOnly: string[];
  agentTotal: number;
  gitTotal: number;
};

export type SessionDetail = {
  session: SessionListItem;
  timeline: TimelineItem[];
  latestDiff: DiffSnapshot | null;
  diffHistory: DiffSnapshot[];
  repoStatus: RepoStatus;
  agentDiff: AgentDiff;
  divergence: DivergenceReport;
};

export type DesktopPreferences = {
  rememberSession: boolean;
  rememberInspectorLayout: boolean;
  launchInspectorOpen: boolean;
  defaultInspectorTab: InspectorTab;
  defaultInspectorWidth: number;
  remoteEnabled: boolean;
  remoteTarget: string;
  remoteCwd: string;
};

export type RemoteConnection = {
  enabled: boolean;
  target: string;
  cwd: string;
  password: string;
};

export type RemoteOperation = {
  success: boolean;
  stdout: string;
  stderr: string;
};

export type SettingsConfig = {
  appName: string;
  defaultProvider: string | null;
  defaultModel: string | null;
  openaiBaseUrl: string | null;
  theme: string;
  mascotId: string;
  mascotDisplayName: string;
  mascotEnabled: boolean;
  uiNoAltScreen: boolean;
  uiTmuxGoldenMode: boolean;
};

export type ResourceCounts = {
  providers: number;
  tools: number;
  agents: number;
  prompts: number;
  hooks: number;
  skills: number;
  mascots: number;
  plugins: number;
  mcpServers: number;
  ides: number;
};

export type SettingsSessionSummary = {
  totalSessions: number;
  folderGroups: number;
};

export type AuthProviderStatus = {
  providerId: string;
  kind: string;
  email: string | null;
  expiresAtMs: number | null;
  scopes: string[];
  planType: string | null;
  organizationName: string | null;
};

export type ProviderSummary = {
  id: string;
  displayName: string;
  baseUrl: string;
  defaultApi: string;
  modelCount: number;
  authModes: string[];
  sourceKind: string;
  sourcePath: string | null;
};

export type SettingsSnapshot = {
  workspaceRoot: string;
  workspaceConfigFile: string;
  userConfigFile: string;
  authStoreFile: string;
  builtinResourcesDir: string;
  config: SettingsConfig;
  resources: ResourceCounts;
  sessions: SettingsSessionSummary;
  auth: AuthProviderStatus[];
  providers: ProviderSummary[];
};

export type ExternalCredential = {
  providerId: string;
  source: "claude" | "codex";
  kind: "api_key" | "oauth";
  description: string;
  sourcePath: string;
};
