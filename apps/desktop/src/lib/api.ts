import { invoke } from "@tauri-apps/api/core"

import type {
  ActionInstance,
  ActionParameters,
  ActionResult,
  ActionType,
  AgentConnection,
  ControlFace,
  DeviceLayout,
  DeviceModel,
  ExportResult,
  ImportOptions,
  ImportPreview,
  ImportSummary,
  LaunchView,
  SecurityPolicy,
  SecurityView,
  WindowSpec,
  MachineConfig,
  MigrationNote,
  ModuleInfo,
  Overview,
  ProfileSummary,
  ProfilesView,
  ProfileView,
  Project,
  ResourceCheck,
  ResourceKind,
  UpdateInfo,
  UpdatePreferences,
  UpdateState,
  Workflow,
  WorkflowReport,
} from "./types"

export interface ActionInput {
  name: string
  actionType: string
  parameters: ActionParameters
  description: string
  icon: string | null
  label: string | null
}

export interface ProjectInput {
  key: string
  name: string
  resources: Record<string, string>
}

/** Typed wrappers around the backend commands. */
export const api = {
  openWindow: (id: string) => invoke<null>("open_window", { id }),
  currentWindow: () => invoke<WindowSpec | null>("current_window"),
  fitWindow: (width: number, height: number, viewportWidth: number, viewportHeight: number) =>
    invoke<[number, number]>("fit_window", { width, height, viewportWidth, viewportHeight }),
  closeWindow: () => invoke<null>("close_window"),
  showMainWindow: () => invoke<boolean>("show_main_window"),
  finishSecurityPrompt: () => invoke<null>("finish_security_prompt"),
  lastWorkflowReport: () => invoke<WorkflowReport | null>("last_workflow_report"),

  securityStatus: () => invoke<SecurityView>("security_status"),
  setupPassword: (password: string, confirmation: string) =>
    invoke<SecurityView>("setup_password", { input: { password, confirmation } }),
  unlock: (password: string) => invoke<SecurityView>("unlock", { password }),
  /** Opens a system-managed configuration (after a lock: with the system check). */
  systemUnlock: () => invoke<SecurityView>("system_unlock"),
  /** The authentication the security policy asks for; `null` without a password. */
  confirmIdentity: (password: string | null) =>
    invoke<SecurityView>("confirm_identity", { password }),
  /** Authenticates for the next sensitive operation. */
  authenticate: (password: string | null) => invoke<null>("authenticate", { password }),
  usePassword: (password: string, confirmation: string) =>
    invoke<SecurityView>("use_password", { input: { password, confirmation } }),
  useSystemKey: () => invoke<SecurityView>("use_system_key"),
  /** Moves the key file of a missing database aside and starts over. */
  startNewConfiguration: () => invoke<SecurityView>("start_new_configuration"),
  setSecurityPolicy: (policy: SecurityPolicy) =>
    invoke<SecurityView>("set_security_policy", { policy }),
  encryptLegacyDatabase: (password: string, confirmation: string) =>
    invoke<SecurityView>("encrypt_legacy_database", { input: { password, confirmation } }),
  changePassword: (current: string, password: string, confirmation: string) =>
    invoke<SecurityView>("change_password", { input: { current, password, confirmation } }),
  lock: () => invoke<SecurityView>("lock"),
  removeUnencryptedCopies: () => invoke<SecurityView>("remove_unencrypted_copies"),

  exportConfiguration: (request: {
    path: string
    password: string
    confirmation: string
    profiles: string[] | null
    includeModuleSettings: boolean
  }) => invoke<ExportResult>("export_configuration", { request }),
  previewImport: (path: string, password: string) =>
    invoke<{ token: number; preview: ImportPreview }>("preview_import", { path, password }),
  applyImport: (token: number, options: ImportOptions) =>
    invoke<ImportSummary>("apply_import", { token, options }),
  cancelImport: () => invoke<null>("cancel_import"),

  getOverview: () => invoke<Overview>("get_overview"),
  launchInfo: () => invoke<LaunchView>("launch_info"),
  listActionTypes: () => invoke<ActionType[]>("list_action_types"),
  listDeviceModels: () => invoke<DeviceModel[]>("list_device_models"),
  migrationNotes: () => invoke<MigrationNote[]>("migration_notes"),

  listModules: () => invoke<ModuleInfo[]>("list_modules"),
  setModuleEnabled: (moduleId: string, enabled: boolean) =>
    invoke<ModuleInfo[]>("set_module_enabled", { moduleId, enabled }),
  setModuleConfig: (moduleId: string, config: ActionParameters) =>
    invoke<ModuleInfo[]>("set_module_config", { moduleId, config }),

  listProfiles: () => invoke<ProfilesView>("list_profiles"),
  getProfile: (profileId: string) => invoke<ProfileView>("get_profile", { profileId }),
  createProfile: (name: string, description: string) =>
    invoke<ProfileSummary>("create_profile", { name, description }),
  updateProfileDetails: (profileId: string, name: string, description: string) =>
    invoke<null>("update_profile_details", { profileId, name, description }),
  duplicateProfile: (profileId: string, name: string) =>
    invoke<ProfileSummary>("duplicate_profile", { profileId, name }),
  deleteProfile: (profileId: string) => invoke<null>("delete_profile", { profileId }),
  setActiveProfile: (profileId: string) => invoke<null>("set_active_profile", { profileId }),
  setDefaultProject: (profileId: string, projectId: string | null) =>
    invoke<null>("set_default_project", { profileId, projectId }),

  createAction: (profileId: string, action: ActionInput) =>
    invoke<ActionInstance>("create_action", { profileId, action }),
  updateAction: (profileId: string, action: ActionInstance) =>
    invoke<ActionInstance>("update_action", { profileId, action }),
  duplicateAction: (profileId: string, actionId: string, name: string) =>
    invoke<ActionInstance>("duplicate_action", { profileId, actionId, name }),
  deleteAction: (profileId: string, actionId: string) =>
    invoke<null>("delete_action", { profileId, actionId }),
  runAction: (profileId: string, actionId: string) =>
    invoke<ActionResult>("run_action", { profileId, actionId }),

  saveWorkflow: (profileId: string, workflow: Workflow) =>
    invoke<Workflow>("save_workflow", { profileId, workflow }),
  duplicateWorkflow: (profileId: string, workflowId: string, name: string) =>
    invoke<Workflow>("duplicate_workflow", { profileId, workflowId, name }),
  deleteWorkflow: (profileId: string, workflowId: string) =>
    invoke<null>("delete_workflow", { profileId, workflowId }),
  runWorkflow: (profileId: string, workflowId: string) =>
    invoke<WorkflowReport>("run_workflow", { profileId, workflowId }),

  createProject: (profileId: string, project: ProjectInput) =>
    invoke<Project>("create_project", { profileId, project }),
  updateProject: (profileId: string, projectId: string, project: ProjectInput) =>
    invoke<Project>("update_project", { profileId, projectId, project }),
  moveProject: (profileId: string, projectId: string, index: number) =>
    invoke<null>("move_project", { profileId, projectId, index }),
  deleteProject: (profileId: string, projectId: string) =>
    invoke<null>("delete_project", { profileId, projectId }),
  copyProject: (profileId: string, projectId: string, targetProfileId: string) =>
    invoke<Project>("copy_project", { profileId, projectId, targetProfileId }),
  cycleProject: (profileId: string, direction: "next" | "previous") =>
    invoke<string | null>("cycle_project", { profileId, direction }),
  selectProject: (profileId: string, projectId: string) =>
    invoke<null>("select_project", { profileId, projectId }),

  getMachineConfig: () => invoke<MachineConfig>("get_machine_config"),
  setResourceBinding: (key: string, kind: ResourceKind, value: string) =>
    invoke<MachineConfig>("set_resource_binding", { key, kind, value }),
  removeResourceBinding: (key: string) => invoke<MachineConfig>("remove_resource_binding", { key }),
  checkResources: (profileId: string) => invoke<ResourceCheck[]>("check_resources", { profileId }),

  createLayout: (profileId: string, modelId: string, name: string, generate: boolean) =>
    invoke<DeviceLayout>("create_layout", { profileId, modelId, name, generate }),
  saveLayout: (profileId: string, layout: DeviceLayout) =>
    invoke<DeviceLayout>("save_layout", { profileId, layout }),
  duplicateLayout: (profileId: string, layoutId: string, name: string) =>
    invoke<string>("duplicate_layout", { profileId, layoutId, name }),
  deleteLayout: (profileId: string, layoutId: string) =>
    invoke<null>("delete_layout", { profileId, layoutId }),
  activateLayout: (profileId: string, layoutId: string) =>
    invoke<null>("activate_layout", { profileId, layoutId }),
  previewLayout: (
    profileId: string,
    layout: DeviceLayout,
    pageId: string,
    projectId: string | null,
  ) => invoke<ControlFace[]>("preview_layout", { profileId, layout, pageId, projectId }),

  agentStatus: () => invoke<AgentConnection>("agent_status"),
  startAgent: () => invoke<AgentConnection>("start_agent"),
  stopAgent: () => invoke<null>("stop_agent"),
  restartAgent: () => invoke<AgentConnection>("restart_agent"),
  refreshDevices: () => invoke<null>("refresh_devices"),
  setBrightness: (percent: number) => invoke<null>("set_brightness", { percent }),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),

  updateState: () => invoke<UpdateState>("update_state"),
  checkForUpdates: () => invoke<UpdateInfo | null>("check_for_updates"),
  downloadUpdate: () => invoke<null>("download_update"),
  installUpdate: () => invoke<null>("install_update"),
  setUpdatePreferences: (preferences: UpdatePreferences) =>
    invoke<UpdateState>("set_update_preferences", { preferences }),
}

/** Backend event carrying the latest {@link AgentConnection}. */
/** The backend's answer to a sensitive operation without a fresh authentication. */
export const AUTHENTICATION_REQUIRED = "authentication required"

export function isAuthenticationRequired(error: unknown): boolean {
  return errorMessage(error) === AUTHENTICATION_REQUIRED
}

/** Sent to every window when the configuration was locked or unlocked. */
export const SECURITY_CHANGED_EVENT = "deckpilot://security-changed"
export const AGENT_STATUS_EVENT = "agent-status"
/** Backend event carrying the latest {@link UpdateState}. */
export const UPDATE_STATE_EVENT = "update-state"

/** Normalizes errors thrown by `invoke` (plain strings) and other sources. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error
  if (error instanceof Error) return error.message
  return "Unexpected error"
}
