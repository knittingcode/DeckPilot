// Fixtures shaped like the backend's data, for tests. Module infos follow the
// built-in modules (ids, action type ids, capabilities) closely enough for
// UI flows; parameters are simplified.

import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionInstance,
  ActionType,
  AgentCompatibility,
  AgentConnection,
  AgentStatus,
  ModuleInfo,
  ParameterSpec,
  Profile,
  ProfileView,
  ProfilesView,
  Project,
  Workflow,
} from "@/lib/types"

export { moduleInfo }

const text = (key: string, label: string, required = true): ParameterSpec => ({
  key,
  label,
  kind: { type: "text", multiline: false },
  required,
})

function actionType(
  id: string,
  name: string,
  category: string,
  parameters: ParameterSpec[] = [],
  requiresProject = false,
): ActionType {
  return { id, name, description: `${name}.`, category, parameters, requiresProject }
}

export const ACTION_TYPES = {
  log: actionType("general.log", "Log message", "General", [text("message", "Message")]),
  openUrl: actionType("system.open-url", "Open URL", "System", [text("url", "URL")]),
  runCommand: actionType("system.run-command", "Run command", "System", [
    text("command", "Command"),
  ]),
  gitStatus: actionType("git.status", "Git status", "Git", [], true),
  dockerStatus: actionType("docker.status", "Docker status", "Docker"),
  composeUp: actionType("docker.compose-up", "Compose up", "Docker", [text("file", "File")]),
  httpRequest: actionType("http.request", "HTTP request", "HTTP", [text("url", "URL")]),
  notify: actionType("notifications.show", "Show notification", "Notifications", [
    text("title", "Title"),
    text("body", "Body", false),
  ]),
} satisfies Record<string, ActionType>

/** The built-in modules with action types; override per test, e.g. `enabled`. */
export function builtInModules(overrides: Record<string, Partial<ModuleInfo>> = {}): ModuleInfo[] {
  const modules: ModuleInfo[] = [
    moduleInfo({
      id: "general",
      name: "General",
      capabilities: ["profile-read"],
      actionTypes: [ACTION_TYPES.log],
    }),
    moduleInfo({
      id: "system",
      name: "System",
      capabilities: ["open-external", "process-execution", "process-inspect"],
      actionTypes: [ACTION_TYPES.openUrl, ACTION_TYPES.runCommand],
    }),
    moduleInfo({
      id: "git",
      name: "Git",
      capabilities: ["machine-resources", "project-context", "process-execution"],
      dependencies: ["system"],
      actionTypes: [ACTION_TYPES.gitStatus],
    }),
    moduleInfo({
      id: "docker",
      name: "Docker",
      capabilities: ["process-execution"],
      actionTypes: [ACTION_TYPES.dockerStatus, ACTION_TYPES.composeUp],
    }),
    moduleInfo({
      id: "http",
      name: "HTTP",
      capabilities: ["network", "credentials"],
      actionTypes: [ACTION_TYPES.httpRequest],
    }),
    moduleInfo({
      id: "notifications",
      name: "Notifications",
      capabilities: ["notifications"],
      actionTypes: [ACTION_TYPES.notify],
    }),
  ]
  return modules.map((m) => ({ ...m, ...overrides[m.id] }))
}

export function project(key: string, change: Partial<Project> = {}): Project {
  return {
    id: `p-${key.toLowerCase()}`,
    key,
    name: `${key} project`,
    resources: {},
    ...change,
  }
}

export function action(
  id: string,
  name: string,
  actionType: string,
  change: Partial<ActionInstance> = {},
): ActionInstance {
  return { id, name, actionType, parameters: {}, ...change }
}

export function stepWorkflow(id: string, name: string, actions: string[] = []): Workflow {
  return {
    id,
    name,
    description: "",
    steps: actions.map((a) => ({ action: a, onFailure: "stop" })),
  }
}

export function profile(change: Partial<Profile> = {}): Profile {
  return {
    id: "profile-1",
    name: "Work",
    description: "",
    actions: [],
    workflows: [],
    projects: [],
    defaultProject: null,
    layouts: [],
    activeLayouts: {},
    ...change,
  }
}

export function profileView(change: Partial<Profile> = {}, view: Partial<ProfileView> = {}) {
  return {
    profile: profile(change),
    activeProjectId: null,
    issues: [],
    actionUsages: {},
    workflowUsages: {},
    ...view,
  } satisfies ProfileView
}

export function profiles(
  list: { id: string; name: string }[] = [{ id: "profile-1", name: "Work" }],
  activeProfileId: string | null = list[0]?.id ?? null,
): ProfilesView {
  return { profiles: list.map((p) => ({ ...p, updatedAt: 0 })), activeProfileId }
}

export function agentStatus(change: Partial<AgentStatus> = {}): AgentStatus {
  return {
    version: "0.5.0",
    protocol: 4,
    pid: 1,
    startedAt: 1_700_000_000,
    dataDir: "/data",
    logDir: "/logs",
    endpoint: "/sock",
    backend: "hid",
    profiles: [],
    activeProfile: { id: "profile-1", name: "Work" },
    activeProject: null,
    devices: [],
    brightness: 60,
    lastInputAt: null,
    ...change,
  }
}

/** A connection to the agent: running and compatible unless stated otherwise. */
export function agentConnection(
  change: Partial<Omit<AgentConnection, "compatibility">> & {
    compatibility?: AgentCompatibility | null
  } = {},
): AgentConnection {
  const running = change.running ?? true
  return {
    running,
    canStart: true,
    appVersion: "0.5.0",
    compatibility: { type: "compatible" },
    status: running ? agentStatus() : null,
    ...change,
  }
}
