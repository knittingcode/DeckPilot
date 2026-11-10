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

export type Capability = "profile-read" | "project-context" | "machine-resources"

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

export interface KeyFace {
  style: FaceStyle
  title: string
  detail?: string | null
  highlighted?: boolean
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

export interface AgentConnection {
  running: boolean
  status: AgentStatus | null
  canStart: boolean
  compatibility: AgentCompatibility | null
  appVersion: string
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
}

export type ImportMode = "merge" | "replace"
export type ConflictResolution = "keepBoth" | "replace" | "skip"

export interface ImportOptions {
  mode: ImportMode
  profiles: string[]
  conflicts: Record<string, ConflictResolution>
  moduleSettings: boolean
}

export interface ImportSummary {
  imported: [string, string][]
  removed: [string, string][]
  skipped: string[]
  renamed: string[]
  modules: string[]
}
