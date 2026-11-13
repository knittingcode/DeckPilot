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
  /** A list stored as one item per line, e.g. command arguments. */
  | { type: "lines"; itemLabel?: string; maxItems?: number; maxLength?: number }
  /** Rows stored as one `name<separator>value` per line, e.g. headers. */
  | {
      type: "pairs"
      separator: string
      nameLabel?: string
      valueLabel?: string
      maxItems?: number
      caseInsensitive?: boolean
      noSecrets?: boolean
    }
  /** Source text such as a JSON body. */
  | { type: "code"; language: string; maxLength?: number }
  /** The name of a credential stored on this computer. */
  | { type: "credential" }

export type ParameterValue =
  | { type: "text"; value: string }
  | { type: "integer"; value: number }
  | { type: "boolean"; value: boolean }
  | { type: "choice"; value: string }
  | { type: "resource"; value: ResourceRef }
  | { type: "project"; value: string }
  /**
   * The whole value from one reference, without braces, e.g.
   * `shared.projects.favorite`; keeps the native type. Resolved by the agent
   * when the action runs.
   */
  | { type: "reference"; value: string }
  /**
   * Text with `{{ … }}` references, for text kinds only. Literal `text`
   * values are never parsed for `{{`.
   */
  | { type: "template"; value: string }

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
  /**
   * Replaced by another type: saved actions of this type keep working, but
   * it is not offered for new ones. Omitted when false.
   */
  deprecated?: boolean
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
  /** Values the workflow takes when a Sub-workflow node calls it (graph workflows). */
  inputs?: WorkflowInput[]
  /** Values the workflow returns to a Sub-workflow node, set by its Output nodes. */
  outputs?: WorkflowOutput[]
  /** Values that live for one run, set by Set variable nodes (graph workflows). */
  variables?: WorkflowVariable[]
  /** Schedules and lifecycle events that start the workflow in the agent. */
  triggers?: WorkflowTrigger[]
  /** None of the triggers starts the workflow. */
  triggersPaused?: boolean
}

export type Weekday = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun"

/** When a scheduled trigger is due; times are local `HH:MM`. */
export type Schedule =
  | { every: "interval"; minutes: number }
  | { every: "daily"; at: string }
  | { every: "weekly"; days: Weekday[]; at: string }

export type LifecycleEvent =
  "agentStarted" | "profileActivated" | "projectChanged" | "deviceConnected" | "deviceDisconnected"

/** What a firing knows, offered to workflow inputs. */
export type EventField =
  | "firedAt"
  | "profileName"
  | "projectKey"
  | "projectName"
  | "previousProjectKey"
  | "deviceId"
  | "deviceModel"

export type TriggerValue = { type: "fixed"; value: string } | { type: "event"; field: EventField }

/** Something that starts a workflow by itself (crates/deckpilot-core/src/trigger.rs). */
export type WorkflowTrigger = (
  { type: "schedule"; schedule: Schedule } | { type: "lifecycle"; event: LifecycleEvent }
) & {
  /** Values for the workflow's inputs, by input key. */
  inputs?: Record<string, TriggerValue>
}

/**
 * A value a workflow takes from the Sub-workflow node that calls it; its
 * nodes read it as an output of the start node.
 */
export interface WorkflowInput {
  key: string
  label?: string
  kind: ValueKind
  /** A calling node must give it a value. */
  required?: boolean
  /** Used when the calling node gives none, and when the workflow runs on its own. */
  default?: string
  /** May be secret: only sensitive outputs may take it. */
  sensitive?: boolean
}

/**
 * A value that lives for one run of the workflow. Set by Set variable nodes
 * and read as `steps.vars.output.<key>`; never shared with other runs or
 * sub-workflows.
 */
export interface WorkflowVariable {
  key: string
  label?: string
  kind: ValueKind
  /** The value when a run starts. */
  initial?: string
  /** May hold a secret: never shown in history. */
  sensitive?: boolean
}

/** A value a workflow returns to the Sub-workflow node that calls it. */
export interface WorkflowOutput {
  key: string
  label?: string
  kind: ValueKind
  /** Kept out of history and live events. */
  sensitive?: boolean
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
  /** The whole value of a shared, system, workflow or step reference, with
   * its native type. */
  | { type: "variable"; reference: string; default?: VariableValue }

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
  | {
      type: "call"
      workflow: string
      /** Values for the called workflow's inputs, by input key. */
      inputs?: Record<string, GraphMapping>
      policy: ErrorPolicy
    }
  | { type: "output"; values?: Record<string, GraphMapping> }
  | { type: "setVariable"; values?: Record<string, GraphMapping> }
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

/**
 * `press`/`release` are raw edges and fire at once. `click`,
 * `doubleClick` and `longPress` are gestures (see
 * docs/gestures-and-work-session.md); a control uses one kind or the other
 * on a page, never both.
 */
export type Trigger =
  | { type: "press" }
  | { type: "release" }
  | { type: "rotateClockwise" }
  | { type: "rotateCounterClockwise" }
  | { type: "touch"; gesture: TouchGesture }
  | { type: "click" }
  | { type: "doubleClick" }
  | { type: "longPress" }

export type BindingTarget =
  /** `overrides` replace some of the instance's parameters for this binding only. */
  | { type: "action"; action: string; overrides?: ActionParameters }
  | { type: "workflow"; workflow: string }
  | { type: "openPage"; page: string }
  | { type: "back" }
  /** Asks a state machine for the operation its rules pick (see `StatefulBinding`). */
  | ({ type: "stateful" } & StatefulBinding)

/** `#rrggbb`. */
export type Color = string
export type FontSize = "small" | "medium" | "large"
export type TitleAlign = "top" | "middle" | "bottom"
/** fit: whole icon, aspect kept; fill: stretched; crop: covers the key. */
export type IconFit = "fit" | "fill" | "crop"

/** How a key looks; every field optional (see docs/data-model.md). */
export interface KeyAppearance {
  background?: Color | null
  textColor?: Color | null
  fontSize?: FontSize | null
  /** Default: middle, or bottom when the key has an icon. */
  titleAlign?: TitleAlign | null
  /** `false` hides the title and the secondary line. Default: shown. */
  showTitle?: boolean | null
  icon?: { id: string; fit?: IconFit } | null
}

/** An image imported into a profile; `png` is base64. */
export interface Icon {
  id: string
  name: string
  width: number
  height: number
  png: string
}

/** An icon as `list_icons` / `import_icon` return it. */
export interface IconInfo {
  id: string
  name: string
  width: number
  height: number
  bytes: number
  dataUrl: string
  usages: number
}

export interface ControlBinding {
  control: ControlId
  trigger: Trigger
  target: BindingTarget
  label?: string | null
  appearance?: KeyAppearance | null
}

export interface LayoutPage {
  id: string
  name: string
  /** The folder the page is in; Back returns there. See `explicitTree`. */
  parent?: string | null
  bindings: ControlBinding[]
  /** Gesture timings of controls that differ from the machine's defaults. */
  gestures?: KeyGestureSettings[]
}

export interface DeviceLayout {
  id: string
  name: string
  model: string
  homePage: string
  pages: LayoutPage[]
  /**
   * The page tree is stored: `parent` is the whole truth and a missing
   * parent means top level. Without it, missing parents are inferred from
   * openPage bindings (use `page_tree`). Set it when editing parents.
   */
  explicitTree?: boolean
}

/** One page of `page_tree`. */
export interface PageNode {
  id: string
  name: string
  /** Stored or inferred folder; null: top level. */
  parent: string | null
  /** Whether `parent` is stored rather than inferred. */
  stored: boolean
  depth: number
  /** The key the runtime adds as Back because the page has none. */
  automaticBack: ControlId | null
}

export interface PageTree {
  pages: PageNode[]
  /** Problems that do not prevent saving, e.g. folders without Back. */
  warnings: string[]
}

/** Result of `move_page` / `copy_page`. */
export interface PageChange {
  /** The page in the destination (new id for copies). */
  page: string
  /** Keys removed from the old layout because they opened moved pages. */
  removedKeys: number
  /** Every changed layout, as saved. */
  layouts: DeviceLayout[]
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
  icons?: Icon[]
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

// Safe deletion of actions (deckpilot_storage::references, workstream E).

/** One place that uses an action. */
export type ReferenceSite =
  /** A key, dial or touch binding of a layout page. */
  | { type: "binding"; layout: string; page: string; control: ControlId; trigger: Trigger }
  /** A step of a step-list workflow (index from 0). */
  | { type: "step"; workflow: string; index: number }
  /** An action node of a graph workflow. */
  | { type: "node"; workflow: string; node: string }
  /**
   * A trigger of a workflow that runs the action. Indirect: it follows the
   * workflow and is never changed by replacing or unbinding.
   */
  | { type: "trigger"; workflow: string; index: number }

export type TriggerKind =
  { type: "schedule"; schedule: Schedule } | { type: "lifecycle"; event: LifecycleEvent }

/** A use of an action, with names for display. */
export interface ActionReference {
  site: ReferenceSite
  /** The layout or workflow name. */
  ownerName: string
  /** The page name of a binding, the node label of a node, "paused" for paused triggers. */
  detail?: string
  /** The trigger of a `trigger` site. */
  trigger?: TriggerKind
}

/** Which uses a replace or unbind acts on. */
export type ReferenceSelection = { type: "all" } | { type: "sites"; sites: ReferenceSite[] }

/** What a replace or unbind changed. */
export interface ReferenceChangeResult {
  changed: ReferenceSite[]
  /** Steps and nodes whose overrides and mappings were removed: the replacement has another type. */
  clearedSettings: ReferenceSite[]
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
  /**
   * `action.failed`, `timeout`, `cancelled`, `mapping`, `parallel-context`,
   * `unavailable`, `loop.failed`, `subworkflow.failed`, `variable-conflict`.
   */
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
  /** Sub-workflow nodes: the workflow that ran; Output nodes: their own workflow. */
  workflow?: string
  /** A Set variable node: `output` holds the variables it set. */
  setsVariables?: boolean
}

export type ExecutionState = "running" | "succeeded" | "failed" | "cancelled" | "interrupted"
export type ExecutionTrigger = "app" | "device" | "schedule" | "lifecycle" | "workSession" | "other"

export interface ExecutionSummary {
  id: string
  workflow: string
  workflowName: string
  profile: string
  trigger: ExecutionTrigger
  /** For schedules and lifecycle events: which one, e.g. `every 15 min`. */
  triggerDetail?: string
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
  | "variables-read"

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
  appearance?: KeyAppearance | null
}

export interface ControlFace {
  control: ControlId
  face: KeyFace
  /** With `render`: the key image as a PNG data URL. */
  image?: string
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
  | "variables"

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

/** Stable releases only (default), or also alpha, beta and rc builds. */
export type UpdateChannel = "stable" | "prerelease"

export interface UpdatePreferences {
  mode: UpdateMode
  automaticChecks: boolean
  checkIntervalHours: number
  lastCheck: number | null
  channel: UpdateChannel
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
  /** Shared variables in the export (deckpilot_portability::VariablesPreview). */
  variables: ImportVariablesPreview
}

/** The shared variables an import brings. */
export interface ImportVariablesPreview {
  /** Variables that do not exist here and would be created. */
  new: string[]
  /** Variables that exist here with the same type; the local definition is kept. */
  existing: string[]
  /** Variables whose type differs from the local one; their values are not imported. */
  incompatible: string[]
  /** Values of profiles and projects. */
  values: number
  /** Global values, applied only with the application settings. */
  globalValues: number
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
  /** Shared variables that were created. */
  variables: string[]
  /** Number of shared variable values that were set. */
  variableValues: number
  /** Values that were not imported, and why. */
  skippedValues: string[]
}

// Shared variables and references (deckpilot_core::variables). Contract
// only: nothing sends or receives these yet. See docs/action-architecture.md.

export type VariableType =
  | { type: "string" }
  | { type: "integer" }
  | { type: "boolean" }
  | { type: "projectId" }
  | { type: "profileId" }
  | { type: "resource"; kind?: ResourceKind }
  | { type: "json" }
  /** The name of a stored credential, never the secret. */
  | { type: "secretRef" }

export type VariableValue =
  | { type: "string"; value: string }
  | { type: "integer"; value: number }
  | { type: "boolean"; value: boolean }
  | { type: "projectId"; value: string }
  | { type: "profileId"; value: string }
  | { type: "resource"; value: ResourceRef }
  | { type: "json"; value: unknown }
  | { type: "secretRef"; value: string }

/** Most specific first: execution, project, profile, global. */
export type VariableScope = "global" | "profile" | "project" | "execution"

export type ScopeKey =
  | { kind: "global" }
  | { kind: "profile"; profile: string }
  | { kind: "project"; profile: string; project: string }
  | { kind: "execution"; execution: string }

/** A shared variable; `key` is `namespace.name` without `shared.`. */
export interface VariableDefinition {
  key: string
  label?: string
  description?: string
  valueType: VariableType
  scopes: VariableScope[]
  default?: VariableValue
  sensitive?: boolean
  module?: string
}

/** Which project a context uses; previews always say which. */
export type ProjectSource = "application" | "execution" | "explicit"

export type ResolutionPhase = "preview" | "execution"

/** The explicit context references are resolved in. */
export interface ResolutionContext {
  profile: string
  project?: string
  projectSource?: ProjectSource
  device?: DeviceId
  workflow?: string
  execution?: string
  phase?: ResolutionPhase
}

export type ValueSource =
  | { type: "scope"; scope: ScopeKey }
  | { type: "default" }
  | { type: "system" }
  | { type: "stepOutput"; node: string }

export type MissingReason =
  | "unknownVariable"
  | "noValue"
  | "noProject"
  | "noDevice"
  | "noWorkflow"
  | "noExecution"
  | "noOutput"
  | "otherProfile"

/** What the agent's resolver reports for one reference. */
export type ReferenceStatus =
  | { status: "resolved"; value: VariableValue; source: ValueSource }
  | { status: "redacted"; valueType: VariableType; source: ValueSource }
  | { status: "unavailableUntilRuntime" }
  | { status: "missing"; reason: MissingReason }
  | { status: "typeMismatch"; found: VariableType }
  | { status: "permissionDenied" }
  | { status: "invalid"; message: string }

/** A reference (without braces) to preview, and the parameter it fills. */
export interface PreviewItem {
  reference: string
  expected?: ParameterKind
}

export interface ReferencePreview {
  reference: string
  status: ReferenceStatus
}

export type VariableGroup = "shared" | "system" | "profile" | "project" | "workflow" | "stepOutput"

export type ReferenceAvailability = "available" | "runtimeOnly" | "notInContext"

/** Metadata of one reference the picker can insert; never a value. */
export interface CatalogEntry {
  reference: string
  label: string
  description?: string
  group: VariableGroup
  valueType: VariableType
  scope?: VariableScope
  module?: string
  sensitive?: boolean
  dynamic?: boolean
  availability: ReferenceAvailability
}

export interface CatalogFilter {
  query?: string
  groups?: VariableGroup[]
  compatibleWith?: ParameterKind
  includeUnavailable?: boolean
}

// Gestures and work sessions (deckpilot_core::gesture, ::work_session).
// Contract only: nothing sends or receives these yet. See
// docs/gestures-and-work-session.md.

export type Gesture = "click" | "doubleClick" | "longPress"

export interface GestureTimings {
  doubleClickMs: number
  longPressMs: number
}

/** Timings of one key of a page; unset values use the defaults. */
export interface KeyGestureSettings {
  control: ControlId
  doubleClickMs?: number
  longPressMs?: number
}

export type WorkSessionState =
  "idle" | "starting" | "working" | "verifyingEnd" | "readyToFinish" | "stopping"

export type WorkSessionProblem =
  | "startFailed"
  | "startInterrupted"
  | "verificationFailed"
  | "verificationInterrupted"
  | "notReady"
  | "approvalExpired"
  | "approvalInvalidated"
  | "needsReview"

export interface WorkSessionNotice {
  problem: WorkSessionProblem
  message: string
  atMs: number
}

export interface VerificationOutcome {
  canFinish: boolean
  details?: string
}

export interface VerificationRecord {
  id: string
  outcome?: VerificationOutcome
  atMs: number
}

/** Elapsed time at `asOfMs`; while the timer runs, add the time since. */
export interface WorkSessionSnapshot {
  state: WorkSessionState
  session?: string
  profile?: string
  project?: string
  startedAtMs?: number
  elapsedMs: number
  asOfMs: number
  verification?: VerificationRecord
  approval?: string
  notices?: WorkSessionNotice[]
}

/** Operations a work session key can ask for (deckpilot_core::stateful). */
export type WorkSessionOperation =
  "start" | "cancelStart" | "verify" | "cancelVerification" | "finish" | "continueWorking"

/** In `state`, `gesture` asks for `operation`. */
export interface StatefulRule<S, O> {
  state: S
  gesture: Gesture
  operation: O
}

/**
 * What a stateful key follows. Without `rules`, the machine's default
 * table applies: idle click → start; starting long press → cancelStart;
 * working click and long press → verify (the agent lets through the one
 * that fits the settings); verifyingEnd long press → cancelVerification;
 * readyToFinish click → finish, long press → verify again.
 * Only on click, double click or long press bindings; `finish` only on
 * click; every stateful binding of a key has the same machine and rules.
 */
export type StatefulBinding = {
  machine: "workSession"
  rules?: StatefulRule<WorkSessionState, WorkSessionOperation>[]
}

/** Read-only variables of the active work session (proposed paths). */
export type WorkSessionVariable =
  | "system.work.session.id"
  | "system.work.session.state"
  | "system.work.session.startedAt"
  | "system.work.session.elapsedSeconds"
  | "system.work.session.projectId"
  | "system.work.session.canFinish"

// Work sessions over protocol 8 (`deckpilot_ipc::work_session`, the Tauri
// commands in `commands/work_session.rs`). docs/gestures-and-work-session.md §9.
// No request starts, verifies or finishes a session: only its key does (D11).

/** How keys and the page show elapsed time: `h:mm` (D4) or `h:mm:ss`. */
export type ElapsedFormat = "minutes" | "seconds"

/** `work_session_settings` of a profile. */
export interface WorkSessionSettings {
  /** The profile they belong to: set in answers; when saving, `profileId` decides. */
  profile?: string
  /** Runs before the timer starts; `null`: the session starts at once. */
  startWorkflow: string | null
  /** Must declare the output `canFinish` (boolean); `null`: hold the key to finish (D12). */
  verificationWorkflow: string | null
  /** 60–86400. */
  approvalTtlSeconds: number
  /** 60–86400. */
  reviewGapSeconds: number
  elapsedFormat: ElapsedFormat
  /** When last saved; unset for defaults never saved. Ignored when saving. */
  updatedAtMs?: number
}

export type WorkSessionReviewKind = "agentDown" | "sleep" | "clockBack" | "clockForward"

/** A gap of uncertain time; counted until the user excludes it (D8). */
export interface WorkSessionReview {
  id: number
  kind: WorkSessionReviewKind
  fromMs: number
  toMs: number
  /** Unset: not reviewed yet (counted). */
  resolution?: "counted" | "excluded"
  resolvedAtMs?: number
}

export interface WorkSessionInterval {
  seq: number
  startedAtMs: number
  /** Unset while the interval is open. */
  endedAtMs?: number
  endReason?: "stopped" | "gapExcluded"
}

export interface WorkSessionTransition {
  seq: number
  atMs: number
  from: string
  event: string
  to: string
}

/** A stored session: the active one, or a finished or aborted one. */
export type WorkSessionRecordState = Exclude<WorkSessionState, "idle"> | "stopped" | "aborted"

/** One entry of the session history (`listWorkSessions`). */
export interface WorkSessionSummary {
  session: string
  profile: string
  project?: string
  state: WorkSessionRecordState
  requestedAtMs: number
  startedAtMs?: number
  stoppedAtMs?: number
  /** Counted time when the answer was made: the sum of the intervals. */
  elapsedMs: number
  intervals: WorkSessionInterval[]
  reviews: WorkSessionReview[]
  transitions: WorkSessionTransition[]
  verification?: VerificationRecord
  /** The session's last notice, until it is cleared. */
  notice?: WorkSessionNotice
}

// Shared Variables settings page (workstream D). Mirrors of the variable
// store (deckpilot_storage::variables, variable_references); the Tauri
// commands come with the store.

/** One place that uses a shared variable. */
export type VariableUse =
  /** A parameter or the label of an action instance. */
  | { type: "action"; action: string }
  /** A step, node, input or other part of a workflow. */
  | { type: "workflow"; workflow: string; node?: string }
  /** The label (or inline action) of a layout binding. */
  | { type: "binding"; layout: string; page: string; control: ControlId; trigger: Trigger }
  /** A trigger of a workflow that uses the variable; follows the workflow. */
  | {
      type: "trigger"
      workflow: string
      index: number
      kind: { type: "schedule"; schedule: Schedule } | { type: "lifecycle"; event: LifecycleEvent }
    }

/** A use of a shared variable, with names for display. */
export interface VariableReference {
  profile: string
  profileName: string
  use: VariableUse
  /** The action, workflow or layout name. */
  ownerName: string
  /** Where inside it, e.g. `parameters.project_id`. */
  path: string
}

/** A value of a shared variable at one scope. */
export interface SharedVariableValue {
  scope: ScopeKey
  /** Left out for sensitive variables: their values are never sent. */
  value?: VariableValue
}

/**
 * A shared variable as the settings page shows it: the definition, the
 * values that apply to the profile being edited (global, the profile, its
 * projects) and every use in any profile.
 */
export interface SharedVariable {
  definition: VariableDefinition
  values: SharedVariableValue[]
  references: VariableReference[]
}
