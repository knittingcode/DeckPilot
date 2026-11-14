// Pure helpers of the Shared Variables settings page: key rules, value
// drafts, labels. The rules mirror deckpilot_core::variables (VariableKey,
// VariableDefinition::check_value); the store checks again on save.

import type {
  CatalogEntry,
  Project,
  ProfileSummary,
  ResourceKind,
  ScopeKey,
  VariableDefinition,
  VariableScope,
  VariableType,
  VariableReference,
  VariableValue,
} from "./types"
import { controlLabel } from "./layout-model"
import { describeTrigger } from "@/workflow-builder/trigger-model"

export const RESERVED_NAMESPACES = ["steps", "system", "workflow"]
const MAX_SEGMENTS = 4
const SEGMENT = /^[a-z][A-Za-z0-9_]{0,31}$/

/** What is wrong with a new key, or `null`. */
export function keyProblem(key: string, existing: string[]): string | null {
  const trimmed = key.trim()
  if (!trimmed) return "Enter a key."
  const segments = trimmed.split(".")
  if (segments.length < 2 || segments.length > MAX_SEGMENTS) {
    return "Use 2 to 4 parts separated by dots, e.g. paths.root."
  }
  if (!segments.every((s) => SEGMENT.test(s))) {
    return "Each part starts with a lowercase letter and has at most 32 letters, digits or _."
  }
  const [namespace = ""] = segments
  if (RESERVED_NAMESPACES.includes(namespace)) return `“${namespace}” is reserved.`
  if (existing.includes(trimmed)) return "A variable with this key exists."
  return null
}

export const VALUE_TYPES: { type: VariableType["type"]; label: string }[] = [
  { type: "string", label: "Text" },
  { type: "integer", label: "Integer" },
  { type: "boolean", label: "Yes/no" },
  { type: "projectId", label: "Project" },
  { type: "profileId", label: "Profile" },
  { type: "resource", label: "Resource" },
  { type: "json", label: "JSON" },
  { type: "secretRef", label: "Stored credential" },
]

export const RESOURCE_KINDS: { kind: ResourceKind; label: string }[] = [
  { kind: "directory", label: "Directory" },
  { kind: "application", label: "Application" },
  { kind: "terminal", label: "Terminal" },
  { kind: "dockerEnvironment", label: "Docker environment" },
  { kind: "service", label: "Service" },
  { kind: "url", label: "URL" },
  { kind: "other", label: "Other" },
]

export function typeLabel(type: VariableType): string {
  const label = VALUE_TYPES.find((t) => t.type === type.type)?.label ?? type.type
  if (type.type === "resource" && type.kind) {
    return `${label} (${RESOURCE_KINDS.find((k) => k.kind === type.kind)?.label ?? type.kind})`
  }
  return label
}

/** Scopes whose values are stored; execution values exist only during a run. */
export const STORED_SCOPES: { scope: VariableScope; label: string; hint: string }[] = [
  { scope: "global", label: "Global", hint: "One value for every profile" },
  { scope: "profile", label: "Profile", hint: "A value per profile" },
  { scope: "project", label: "Project", hint: "A value per project" },
]

/**
 * A value being edited, as text. `mode` tells resources whether `text` is a
 * fixed resource key or a project role.
 */
export interface ValueDraft {
  text: string
  mode?: "fixed" | "projectRole"
}

export function emptyDraft(type: VariableType): ValueDraft {
  if (type.type === "boolean") return { text: "false" }
  if (type.type === "resource") return { text: "", mode: "fixed" }
  return { text: "" }
}

/** The draft for an existing value. */
export function draftOf(value: VariableValue): ValueDraft {
  switch (value.type) {
    case "integer":
      return { text: String(value.value) }
    case "boolean":
      return { text: value.value ? "true" : "false" }
    case "resource":
      return value.value.type === "fixed"
        ? { text: value.value.key, mode: "fixed" }
        : { text: value.value.role, mode: "projectRole" }
    case "json":
      return { text: JSON.stringify(value.value, null, 2) }
    default:
      return { text: value.value }
  }
}

export type Parsed = { value: VariableValue } | { error: string }

/** The value of a draft for `type`, or why it is not one. */
export function parseValue(type: VariableType, draft: ValueDraft): Parsed {
  const text = draft.text
  switch (type.type) {
    case "string":
      return { value: { type: "string", value: text } }
    case "integer": {
      const trimmed = text.trim()
      if (!/^-?\d+$/.test(trimmed)) return { error: "Enter a whole number." }
      const number = Number(trimmed)
      if (!Number.isSafeInteger(number)) return { error: "The number is too large." }
      return { value: { type: "integer", value: number } }
    }
    case "boolean":
      return { value: { type: "boolean", value: text === "true" } }
    case "projectId":
      return text ? { value: { type: "projectId", value: text } } : { error: "Choose a project." }
    case "profileId":
      return text ? { value: { type: "profileId", value: text } } : { error: "Choose a profile." }
    case "resource": {
      const trimmed = text.trim()
      if (!trimmed) return { error: "Enter a resource key or a project role." }
      return {
        value: {
          type: "resource",
          value:
            draft.mode === "projectRole"
              ? { type: "projectRole", role: trimmed }
              : { type: "fixed", key: trimmed },
        },
      }
    }
    case "json":
      try {
        return { value: { type: "json", value: JSON.parse(text) as unknown } }
      } catch {
        return { error: "Enter valid JSON." }
      }
    case "secretRef":
      return text
        ? { value: { type: "secretRef", value: text } }
        : { error: "Choose a stored credential." }
  }
}

/** How a value is shown; sensitive variables never show theirs. */
export function formatValue(
  definition: VariableDefinition,
  value: VariableValue | undefined,
  projects: Project[],
  profiles: ProfileSummary[],
): string {
  if (definition.sensitive) return "Redacted"
  if (!value) return "Not set"
  switch (value.type) {
    case "string":
      return value.value === "" ? "(empty)" : value.value
    case "integer":
      return String(value.value)
    case "boolean":
      return value.value ? "Yes" : "No"
    case "projectId": {
      const project = projects.find((p) => p.id === value.value)
      return project ? `${project.key} — ${project.name}` : "Unknown project"
    }
    case "profileId":
      return profiles.find((p) => p.id === value.value)?.name ?? "Unknown profile"
    case "resource":
      return value.value.type === "fixed"
        ? `Resource ${value.value.key}`
        : `Project role ${value.value.role}`
    case "json":
      return JSON.stringify(value.value)
    case "secretRef":
      // The name of the credential, never its secret.
      return `Credential ${value.value}`
  }
}

/** The places a value can be set for the profile being edited. */
export function slotsFor(
  definition: VariableDefinition,
  profileId: string | null,
  projects: Project[],
): ScopeKey[] {
  const slots: ScopeKey[] = []
  if (definition.scopes.includes("global")) slots.push({ kind: "global" })
  if (profileId && definition.scopes.includes("profile")) {
    slots.push({ kind: "profile", profile: profileId })
  }
  if (profileId && definition.scopes.includes("project")) {
    for (const project of projects) {
      slots.push({ kind: "project", profile: profileId, project: project.id })
    }
  }
  return slots
}

export function sameScope(a: ScopeKey, b: ScopeKey): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

export function scopeLabel(scope: ScopeKey, projects: Project[], profiles: ProfileSummary[]) {
  switch (scope.kind) {
    case "global":
      return "Global"
    case "profile":
      return `Profile ${profiles.find((p) => p.id === scope.profile)?.name ?? ""}`.trim()
    case "project": {
      const project = projects.find((p) => p.id === scope.project)
      return project ? `Project ${project.key}` : "Project"
    }
    case "execution":
      return "During a run"
  }
}

/** Whether deleting must wait for this use (triggers follow their workflow). */
export function isDirectUse(reference: VariableReference): boolean {
  return reference.use.type !== "trigger"
}

export function describeVariableReference(reference: VariableReference): string {
  const { use, ownerName, path, profileName } = reference
  const where = path ? `, ${path}` : ""
  const text = (() => {
    switch (use.type) {
      case "action":
        return `Action “${ownerName}”${where}`
      case "workflow":
        return `Workflow “${ownerName}”${use.node ? `, node ${use.node}` : ""}${where}`
      case "binding":
        return `Layout “${ownerName}”, ${controlLabel(use.control)}${where}`
      case "trigger":
        return `Workflow “${ownerName}” starts by trigger: ${describeTrigger(use.kind)}`
    }
  })()
  return `${profileName}: ${text}`
}

/**
 * The read-only references every context offers (docs/action-architecture.md
 * §6), shown when the agent cannot list them (older agent, not running).
 */
export const SYSTEM_VARIABLES: CatalogEntry[] = [
  {
    reference: "system.project.current.id",
    label: "Current project",
    description: "The run's project inside a run, the active project for a key press.",
    group: "system",
    valueType: { type: "projectId" },
    dynamic: true,
    availability: "available",
  },
  {
    reference: "system.project.current.key",
    label: "Current project key",
    group: "system",
    valueType: { type: "string" },
    dynamic: true,
    availability: "available",
  },
  {
    reference: "system.project.current.name",
    label: "Current project name",
    group: "system",
    valueType: { type: "string" },
    dynamic: true,
    availability: "available",
  },
  {
    reference: "system.profile.current.id",
    label: "Current profile",
    group: "system",
    valueType: { type: "profileId" },
    dynamic: true,
    availability: "available",
  },
  {
    reference: "system.profile.current.name",
    label: "Current profile name",
    group: "system",
    valueType: { type: "string" },
    dynamic: true,
    availability: "available",
  },
  {
    reference: "system.device.current.id",
    label: "Device",
    description: "The device whose control started the action (model#serial).",
    group: "system",
    valueType: { type: "string" },
    dynamic: true,
    availability: "available",
  },
  {
    reference: "system.platform",
    label: "Platform",
    description: "linux, macOs, windows or other.",
    group: "system",
    valueType: { type: "string" },
    availability: "available",
  },
  {
    reference: "workflow.id",
    label: "Workflow",
    description: "The running workflow.",
    group: "workflow",
    valueType: { type: "string" },
    dynamic: true,
    availability: "runtimeOnly",
  },
  {
    reference: "workflow.execution.id",
    label: "Run",
    description: "The current run of the workflow.",
    group: "workflow",
    valueType: { type: "string" },
    dynamic: true,
    availability: "runtimeOnly",
  },
]
