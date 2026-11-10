import { invoke } from "@tauri-apps/api/core"

import type {
  ActionInstance,
  ActionParameters,
  ActionResult,
  ActionType,
  DeviceModel,
  MachineConfig,
  Overview,
  ProfileSummary,
  ProfilesView,
  ProfileView,
  Project,
  ResourceKind,
} from "./types"

/** Typed wrappers around the backend commands. */
export const api = {
  getOverview: () => invoke<Overview>("get_overview"),
  listActionTypes: () => invoke<ActionType[]>("list_action_types"),
  listDeviceModels: () => invoke<DeviceModel[]>("list_device_models"),

  listProfiles: () => invoke<ProfilesView>("list_profiles"),
  getProfile: (profileId: string) => invoke<ProfileView>("get_profile", { profileId }),
  createProfile: (name: string, description: string) =>
    invoke<ProfileSummary>("create_profile", { name, description }),
  deleteProfile: (profileId: string) => invoke<null>("delete_profile", { profileId }),
  setActiveProfile: (profileId: string) => invoke<null>("set_active_profile", { profileId }),

  createAction: (
    profileId: string,
    action: { name: string; actionType: string; parameters: ActionParameters },
  ) => invoke<ActionInstance>("create_action", { profileId, action }),
  deleteAction: (profileId: string, actionId: string) =>
    invoke<null>("delete_action", { profileId, actionId }),
  runAction: (profileId: string, actionId: string) =>
    invoke<ActionResult>("run_action", { profileId, actionId }),

  createProject: (
    profileId: string,
    project: { key: string; name: string; resources: Record<string, string> },
  ) => invoke<Project>("create_project", { profileId, project }),
  deleteProject: (profileId: string, projectId: string) =>
    invoke<null>("delete_project", { profileId, projectId }),
  cycleProject: (profileId: string, direction: "next" | "previous") =>
    invoke<string | null>("cycle_project", { profileId, direction }),
  selectProject: (profileId: string, projectId: string) =>
    invoke<null>("select_project", { profileId, projectId }),

  getMachineConfig: () => invoke<MachineConfig>("get_machine_config"),
  setResourceBinding: (key: string, kind: ResourceKind, value: string) =>
    invoke<MachineConfig>("set_resource_binding", { key, kind, value }),
  removeResourceBinding: (key: string) => invoke<MachineConfig>("remove_resource_binding", { key }),
}

/** Normalizes errors thrown by `invoke` (plain strings) and other sources. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error
  if (error instanceof Error) return error.message
  return "Unexpected error"
}
