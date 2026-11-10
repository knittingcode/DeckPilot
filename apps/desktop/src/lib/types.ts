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

export type ParameterValue =
  | { type: "text"; value: string }
  | { type: "integer"; value: number }
  | { type: "boolean"; value: boolean }
  | { type: "choice"; value: string }
  | { type: "resource"; value: ResourceRef }

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
