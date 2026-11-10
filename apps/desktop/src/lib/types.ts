// TypeScript mirrors of the serialized Rust domain types. Field names follow
// the camelCase serde representation used by the backend.

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
}

export interface WorkflowStep {
  action: string
  onFailure: "stop" | "continue"
}

export interface Workflow {
  id: string
  name: string
  steps: WorkflowStep[]
}

export interface Project {
  id: string
  key: string
  name: string
  resources: Record<string, string>
}

export interface DeviceLayout {
  id: string
  name: string
  model: string
  homePage: string
  pages: { id: string; name: string; bindings: unknown[] }[]
}

export interface Profile {
  id: string
  name: string
  description: string
  actions: ActionInstance[]
  workflows: Workflow[]
  projects: Project[]
  layouts: DeviceLayout[]
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
}

export interface ActionResult {
  status: "succeeded" | "failed"
  message: string
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

export interface Overview {
  appVersion: string
  os: Os
  databasePath: string
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
    touch: { zones: number; image: ImageSpec | null; gestures: string[] } | null
  }
  support: "descriptorOnly" | "verified"
}
