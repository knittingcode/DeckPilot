// TypeScript mirrors of the serialized Rust types (camelCase serde names).

export type Os = "linux" | "macOs" | "windows" | "other"

export type ResourceKind =
  "directory" | "application" | "terminal" | "dockerEnvironment" | "service" | "url" | "other"

export type ResourceRef = { type: "fixed"; key: string } | { type: "projectRole"; role: string }

export type ParameterKind =
  | { type: "text"; multiline: boolean; maxLength?: number }
  | { type: "integer"; min?: number; max?: number }
  | { type: "boolean" }
  | { type: "choice"; options: string[] }
  | { type: "resource"; kind?: ResourceKind }
  /** A project of the profile the action belongs to. */
  | { type: "project" }

export type ParameterValue =
  | { type: "text"; value: string }
  | { type: "integer"; value: number }
  | { type: "boolean"; value: boolean }
  | { type: "choice"; value: string }
  | { type: "resource"; value: ResourceRef }
  | { type: "project"; value: string }

export type ActionParameters = Record<string, ParameterValue>

export interface ParameterSpec {
  key: string
  label: string
  description?: string
  kind: ParameterKind
  required: boolean
  default?: ParameterValue
}

export interface ActionType {
  id: string
  name: string
  description: string
  category: string
  parameters: ParameterSpec[]
  requiresProject: boolean
  /** Always sent by the backend; optional for fixtures. */
  node?: NodeSpec
}

export interface ActionInstance {
  id: string
  name: string
  actionType: string
  parameters: ActionParameters
  description?: string
  icon?: string | null
  label?: string | null
}

export type StepCondition =
  | { type: "always" }
  | { type: "previousSucceeded" }
  | { type: "previousFailed" }
  | { type: "activeProject"; project: string }

export interface WorkflowStep {
  action: string
  onFailure: "stop" | "continue"
  condition?: StepCondition
  overrides?: ActionParameters
}

export interface Workflow {
  id: string
  name: string
  description?: string
  steps: WorkflowStep[]
  /** The visual form; when present it is what runs (crates/deckpilot-core/src/graph). */
  graph?: WorkflowGraph
  /**
   * Whether runs may change DeckPilot's active project (`application`) or
   * keep project changes to themselves (`execution`). Unset for workflows
   * saved before the setting existed; see `effectiveProjectScope`.
   */
  projectScope?: ProjectScope
}

export type ProjectScope = "execution" | "application"

/** What a finished run did to the active project. */
export type ProjectOutcome =
  | { type: "unchanged" }
  | { type: "keptInRun"; project?: string | null }
  | { type: "applied"; project: string }
  | { type: "notApplied"; project?: string | null; reason: string }

export type GraphPort = "out" | "error" | "true" | "false"

export interface GraphReference {
  node: string
  output: string
}

export type GraphPredicate =
  | { type: "equals"; reference: GraphReference; value: string }
  | { type: "notEmpty"; reference: GraphReference }
  | { type: "succeeded"; node: string }
  | { type: "activeProject"; key: string }

export type GraphMapping =
  | { type: "output"; reference: GraphReference; default?: string }
  | { type: "template"; template: string }

export interface ErrorPolicy {
  onError: "stop" | "continue"
  retry: { attempts: number; backoffMs: number }
  timeoutMs?: number
}

export type GraphNodeKind =
  | { type: "trigger" }
  | {
      type: "action"
      action: string
      overrides?: ActionParameters
      mappings?: Record<string, GraphMapping>
      policy: ErrorPolicy
    }
  | { type: "condition"; predicate: GraphPredicate }
  | { type: "parallel" }
  | { type: "join" }
  | { type: "loop"; workflow: string; times: number }
  | { type: "delay"; ms: number }
  | { type: "stop" }

export type GraphNode = GraphNodeKind & {
  id: string
  label?: string
  position: { x: number; y: number }
}

export interface GraphEdge {
  from: string
  port: GraphPort
  to: string
}

export interface WorkflowGraph {
  version: number
  nodes: GraphNode[]
  edges: GraphEdge[]
}

/** A problem the engine found in a graph; `node` is missing for the graph as a whole. */
export interface GraphIssue {
  node?: string
  severity: "error" | "warning"
  message: string
}

/** The graph form of a step workflow, with notes where it behaves differently. */
export interface Conversion {
  graph: WorkflowGraph
  notes: string[]
}

export type ValueKind = "text" | "integer" | "boolean" | "path" | "url" | "projectKey"

/** How an action type behaves as a workflow node (module SDK contract). */
export interface NodeSpec {
  version: number
  icon: string | null
  /** `sensitive` outputs reach later nodes but are never kept in history. */
  outputs: { key: string; label: string; kind: ValueKind; always: boolean; sensitive?: boolean }[]
  timeoutMs: number
  retrySafe: boolean
  idempotent: boolean
  changesContext: boolean
  /** The action stops promptly when cancelled or at its timeout. */
  cancellable?: boolean
  /** What running it does: only reads, opens something, or changes state. */
  effect?: ActionEffect
}

export type ActionEffect = "readOnly" | "opens" | "modifies"

/** A notification an action asked the agent to show; plain text. */
export interface ActionNotification {
  title: string
  body?: string
  severity: "info" | "success" | "warning" | "error"
  desktop: boolean
  statusBar: boolean
}

/** A credential stored on this computer; the secret is never sent to the app. */
export interface CredentialInfo {
  id: string
  description?: string
  /** Milliseconds since the Unix epoch. */
  updatedAt: number
}

export interface Project {
  id: string
  key: string
  name: string
  resources: Record<string, string>
}

export type TouchGesture = "tap" | "longPress" | "swipe"

export type ControlId =
  | { kind: "key"; index: number }
  | { kind: "encoder"; index: number }
  | { kind: "touchZone"; index: number }

export type Trigger =
  | { type: "press" }
  | { type: "release" }
  | { type: "rotateClockwise" }
  | { type: "rotateCounterClockwise" }
  | { type: "touch"; gesture: TouchGesture }

export type BindingTarget =
  | { type: "action"; action: string }
  | { type: "workflow"; workflow: string }
  | { type: "openPage"; page: string }
  | { type: "back" }

export interface ControlBinding {
  control: ControlId
  trigger: Trigger
  target: BindingTarget
  label?: string | null
}

export interface LayoutPage {
  id: string
  name: string
  bindings: ControlBinding[]
}

export interface DeviceLayout {
  id: string
  name: string
  model: string
  homePage: string
  pages: LayoutPage[]
}

export interface Profile {
  id: string
  name: string
  description: string
  actions: ActionInstance[]
  workflows: Workflow[]
  projects: Project[]
  defaultProject?: string | null
  layouts: DeviceLayout[]
  activeLayouts?: Record<string, string>
}

export type Usage =
  | { type: "workflowStep"; workflow: string; workflowName: string; position: number }
  | {
      type: "layoutBinding"
      layout: string
      layoutName: string
      page: string
      pageName: string
      control: ControlId
    }

export interface ProfileSummary {
  id: string
  name: string
  updatedAt: number
}

export interface ProfilesView {
  profiles: ProfileSummary[]
  activeProfileId: string | null
}

export interface ProfileView {
  profile: Profile
  activeProjectId: string | null
  issues: string[]
  actionUsages: Record<string, Usage[]>
  workflowUsages: Record<string, Usage[]>
}

export type ActionStatus = "succeeded" | "failed" | "skipped"

export interface ActionResult {
  status: ActionStatus
  message: string
  /** Structured details, e.g. `projectKey`, `resource`, `value`. */
  data?: Record<string, string>
  /** Why it failed, as the module's stable error code. */
  code?: string
}

export interface StepReport {
  action: string
  result: ActionResult
}

export interface WorkflowReport {
  workflow: string
  status: ActionStatus
  steps: StepReport[]
}

export type NodeStatus = "succeeded" | "failed" | "skipped" | "cancelled"

/** A failure, safe to show: no secrets, no raw output. */
export interface StepError {
  /** `action.failed`, `timeout`, `cancelled`, `mapping`, `parallel-context`, `unavailable`, `loop.failed`. */
  code: string
  message: string
  retryable: boolean
  /** For `action.failed`: the action's own error code, e.g. `exit-nonzero`. */
  actionCode?: string
}

export interface NodeReport {
  node: string
  status: NodeStatus
  attempts: number
  /** Milliseconds after the start of the run. */
  startedMs: number
  durationMs: number
  message?: string
  /** Outputs the node's action type declares; nothing else is reported. */
  output?: Record<string, string>
  error?: StepError
  action?: string
}

export type ExecutionState = "running" | "succeeded" | "failed" | "cancelled" | "interrupted"
export type ExecutionTrigger = "app" | "device"

export interface ExecutionSummary {
  id: string
  workflow: string
  workflowName: string
  profile: string
  trigger: ExecutionTrigger
  state: ExecutionState
  /** Milliseconds since the Unix epoch. */
  startedAt: number
  finishedAt?: number
  totalNodes: number
  completedNodes: number
  runningNodes?: string[]
  message?: string
  /** The active project when the run started: the run's own initial context. */
  initialProject?: string
  projectScope?: ProjectScope
  project?: ProjectOutcome
}

export interface ExecutionRecord extends ExecutionSummary {
  nodes: NodeReport[]
}

/** Progress a running action reports. */
export interface NodeProgress {
  percent?: number
  message?: string
}

export type ExecutionEvent =
  | { type: "nodeStarted"; node: string }
  | { type: "retrying"; node: string; attempt: number; waitMs: number }
  | { type: "nodeFinished"; report: NodeReport }
  | { type: "nodeProgress"; node: string; progress: NodeProgress }

/** Payload of {@link EXECUTION_EVENT}: the agent's execution events. */
export type AgentExecutionEvent =
  | { type: "executionStarted"; execution: ExecutionSummary }
  | {
      type: "executionProgress"
      execution: string
      event: ExecutionEvent
      completed: number
      total: number
    }
  | { type: "executionFinished"; execution: ExecutionSummary }

export interface MachineResourceBinding {
  key: string
  kind: ResourceKind
  value: string
}

export interface MachineConfig {
  os: Os
  bindings: MachineResourceBinding[]
}

export type ResourceStatus = "ok" | "missing" | "unbound" | "unchecked"

export interface ResourceCheck {
  key: string
  kind: ResourceKind | null
  value: string | null
  status: ResourceStatus
  usedBy: string[]
}

export type Capability =
  | "profile-read"
  | "project-context"
  | "machine-resources"
  | "open-external"
  | "process-execution"
  | "process-inspect"
  | "network"
  | "notifications"
  | "credentials"

export interface ModulePage {
  id: string
  title: string
  icon: string
  order: number
}

/** A feature module with its state, as reported by the backend. */
export interface ModuleInfo {
  id: string
  version: string
  name: string
  description: string
  /** Supported operating systems; empty means all. */
  platforms: Os[]
  pages: ModulePage[]
  capabilities: Capability[]
  config: ParameterSpec[]
  dependencies: string[]
  enabledByDefault: boolean
  enabled: boolean
  supported: boolean
  configValues: ActionParameters
  actionTypes: ActionType[]
}

export interface MigrationNote {
  schemaVersion: number
  profile: string | null
  message: string
  createdAt: number
}

export interface Overview {
  appVersion: string
  os: Os
  databasePath: string
  development: boolean
  profileCount: number
  activeProfile: ProfileSummary | null
  activeProject: Project | null
  actionTypeCount: number
  deviceModelCount: number
  resourceBindingCount: number
}

export interface ImageSpec {
  width: number
  height: number
  format: "jpeg" | "bmp"
}

export interface DeviceModel {
  id: string
  vendor: string
  name: string
  usb: { vendorId: number; productId: number } | null
  capabilities: {
    keys: { rows: number; columns: number; image: ImageSpec | null } | null
    encoders: { count: number; pressable: boolean } | null
    touch: { zones: number; image: ImageSpec | null; gestures: TouchGesture[] } | null
  }
  support: "descriptorOnly" | "unverified" | "verified"
}

export type FaceStyle = "empty" | "action" | "workflow" | "folder" | "back" | "notice"

/** Live status mark of a key (see "Button state" in docs/modules.md). */
export type Indicator = "ok" | "warning" | "error" | "running" | "unknown" | "loading"

export interface KeyFace {
  style: FaceStyle
  title: string
  detail?: string | null
  highlighted?: boolean
  /** Set only by the agent's live states; previews never have them. */
  indicator?: Indicator | null
  progress?: number | null
  toggle?: boolean | null
}

export interface ControlFace {
  control: ControlId
  face: KeyFace
}

export type ConnectionState = "connected" | "disconnected"

export type LayoutStatus =
  | { type: "noProfile" }
  | { type: "unknownModel" }
  | { type: "missing" }
  | { type: "compatible"; layout: string; name: string }
  | { type: "invalid"; layout: string; name: string; issues: string[] }

export interface DeviceId {
  model: string
  serial: string
}

export interface DeviceInfo {
  id: DeviceId
  modelName: string
  state: ConnectionState
  layout: LayoutStatus
  page: string | null
  source: string
}

export interface ProfileInfo {
  id: string
  name: string
}

export interface AgentStatus {
  version: string
  protocol: number
  /** Workflows running in the background; absent from older agents. */
  runningExecutions?: number
  pid: number
  startedAt: number
  dataDir: string
  logDir: string
  endpoint: string
  backend: string
  profiles: ProfileInfo[]
  activeProfile: ProfileInfo | null
  activeProject: Project | null
  devices: DeviceInfo[]
  brightness: number
  lastInputAt: number | null
}

export type AgentCompatibility =
  | { type: "compatible" }
  | { type: "agentOutdated"; agent: string; app: string }
  | { type: "agentNewer"; agent: string; app: string }
  | { type: "unknown"; agent: string; app: string }
  /** The agent is too old for any protocol this app speaks; replace it. */
  | { type: "updateRequired"; agent: string; app: string }

/** What a connection to the agent can do, from its negotiated protocol. */
export type AgentCapability =
  | "workflows"
  | "prepareForUpdate"
  | "moduleNamespaces"
  | "encryptedDatabase"
  | "backgroundExecutions"
  | "notifications"

export interface AgentConnection {
  running: boolean
  status: AgentStatus | null
  canStart: boolean
  compatibility: AgentCompatibility | null
  appVersion: string
  /** Protocol of the connection: the lower of the app's and the agent's. */
  protocol?: number | null
  capabilities?: AgentCapability[]
}

export type UpdateMode = "notifyOnly" | "downloadAndAsk" | "installWhenIdle"

export interface UpdatePreferences {
  mode: UpdateMode
  automaticChecks: boolean
  checkIntervalHours: number
  lastCheck: number | null
}

export interface UpdateInfo {
  version: string
  notes: string | null
  date: string | null
}

export interface UpdateState {
  currentVersion: string
  configured: boolean
  target: string
  endpoint: string
  preferences: UpdatePreferences
  phase: "idle" | "checking" | "downloading" | "ready" | "installing"
  available: UpdateInfo | null
  progress: { downloaded: number; total: number | null } | null
  error: string | null
}

export interface LaunchView {
  launch: {
    previousVersion: string | null
    currentVersion: string
    firstAfterUpdate: boolean
  } | null
  encryption: { enabled: boolean; passwordRequired: boolean }
}

export type WindowSize = "fit" | "compact" | "medium" | "large" | "fullscreen"

/** A native window as the window manager knows it. */
export interface WindowSpec {
  id: string
  title: string
  size: WindowSize
  blocking: boolean
}

export type DatabaseState = "missing" | "plaintext" | "encrypted"

/** How the database key is protected; see `docs/security.md`. */
export type ProtectionMode = "systemManaged" | "password"

export type LockReason =
  "startup" | "requested" | "idle" | "sleep" | "screenLocked" | "keyUnavailable"

/**
 * How the user is verified for sensitive operations and after a lock:
 * the DeckPilot password, the operating system (Touch ID / login
 * password), or — with a system key and no system check — only an
 * explicit confirmation.
 */
export type Verification = "password" | "touchId" | "devicePassword" | "confirmationOnly"

export type AutoLock = "never" | "sleep" | "screenLock" | "idle15Minutes" | "idle1Hour"
export type Reauthentication = "sensitiveOnly" | "afterUpdate" | "everyLaunch"

export interface SecurityPolicy {
  autoLock: AutoLock
  reauthentication: Reauthentication
}

/** Whether the configuration can be used; see `docs/security.md`. */
export interface SecurityView {
  database: DatabaseState
  protection: ProtectionMode | null
  unlocked: boolean
  /** The user must authenticate before continuing (security policy). */
  confirmationRequired: boolean
  lockReason: LockReason | null
  /** The agent is reading the key; the credential store may show a prompt. */
  waitingForKeyStore: boolean
  /** A key file exists but its database is missing; nothing is created. */
  databaseMissing: boolean
  /** Whether the agent can detect this computer's screen lock. */
  screenLockDetection: boolean
  /** Why the configuration could not be opened automatically. */
  problem: string | null
  /** Name of the credential store, e.g. "macOS Keychain". */
  keyStore: string
  verification: Verification
  /** Stored policy; present while unlocked. */
  policy: SecurityPolicy | null
  agentRunning: boolean
  appVersion: string
  /** Unencrypted copies kept after encrypting a legacy database. */
  legacyCopies: string[]
  backupDir: string
}

export interface ExportResult {
  path: string
  profiles: number
  modules: number
  bytes: number
}

export type ImportConflict = { type: "sameId"; name: string } | { type: "sameName"; id: string }

export interface ImportProfilePreview {
  id: string
  name: string
  actions: number
  workflows: number
  projects: number
  layouts: number
  conflict: ImportConflict | null
  modules: string[]
}

export interface ImportModulePreview {
  id: string
  name: string | null
  status: "ready" | "missing" | "incompatible"
  enabled: boolean
  skippedSettings: string[]
  problem: string | null
}

export interface MissingResource {
  key: string
  kind: ResourceKind | null
  profiles: string[]
}

export interface ImportPreview {
  exportedAt: number
  appVersion: string
  profiles: ImportProfilePreview[]
  modules: ImportModulePreview[]
  unknownActionTypes: string[]
  missingResources: MissingResource[]
  warnings: string[]
  localProfiles: [string, string][]
  /** Portable application settings in the export. */
  settings: AppSettings
}

export type ImportMode = "merge" | "replace"
export type ConflictResolution = "keepBoth" | "replace" | "skip"

export interface ImportOptions {
  mode: ImportMode
  profiles: string[]
  conflicts: Record<string, ConflictResolution>
  moduleSettings: boolean
  appSettings: boolean
}

/** Application settings that mean the same on every computer. */
export interface AppSettings {
  updates?: { mode: UpdateMode; automaticChecks: boolean; checkIntervalHours: number }
}

/** The end of the agent's log. */
export interface LogView {
  directory: string
  file: string | null
  lines: string[]
  truncated: boolean
}

export interface ImportSummary {
  imported: [string, string][]
  removed: [string, string][]
  skipped: string[]
  renamed: string[]
  modules: string[]
  settings: string[]
}
