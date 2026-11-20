// Pure helpers of the Shared Variables settings page: key rules, value
// drafts, labels. The rules mirror deckpilot_core::variables (VariableKey,
// VariableDefinition::check_value); the store checks again on save. Texts
// are in the window's language, read when used (components that show them
// re-render with `useTranslation`).

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
import { type TranslationKey, i18n } from "@/i18n"
import { describeTrigger } from "@/workflow-builder/trigger-model"

export const RESERVED_NAMESPACES = ["steps", "system", "workflow"]
const MAX_SEGMENTS = 4
const SEGMENT = /^[a-z][A-Za-z0-9_]{0,31}$/

/** What is wrong with a new key, or `null`. */
export function keyProblem(key: string, existing: string[]): string | null {
  const trimmed = key.trim()
  if (!trimmed) return i18n.t("workflows:sharedVariables.keyRule.empty")
  const segments = trimmed.split(".")
  if (segments.length < 2 || segments.length > MAX_SEGMENTS) {
    return i18n.t("workflows:sharedVariables.keyRule.parts")
  }
  if (!segments.every((s) => SEGMENT.test(s))) {
    return i18n.t("workflows:sharedVariables.keyRule.part")
  }
  const [namespace = ""] = segments
  if (RESERVED_NAMESPACES.includes(namespace))
    return i18n.t("workflows:sharedVariables.keyRule.reserved", { namespace })
  if (existing.includes(trimmed)) return i18n.t("workflows:sharedVariables.keyRule.exists")
  return null
}

/** `{ ...fields, label }` whose `label` is `key` in the window's language, read when used. */
function labelled<T extends object>(
  fields: T,
  key: TranslationKey,
): T & { readonly label: string } {
  return Object.defineProperty({ ...fields }, "label", {
    enumerable: true,
    get: () => i18n.t(key),
  }) as T & { readonly label: string }
}

export const VALUE_TYPES: { type: VariableType["type"]; readonly label: string }[] = [
  labelled({ type: "string" as const }, "workflows:sharedVariables.valueType.string"),
  labelled({ type: "integer" as const }, "workflows:sharedVariables.valueType.integer"),
  labelled({ type: "boolean" as const }, "workflows:sharedVariables.valueType.boolean"),
  labelled({ type: "projectId" as const }, "workflows:sharedVariables.valueType.projectId"),
  labelled({ type: "profileId" as const }, "workflows:sharedVariables.valueType.profileId"),
  labelled({ type: "resource" as const }, "workflows:sharedVariables.valueType.resource"),
  labelled({ type: "json" as const }, "workflows:sharedVariables.valueType.json"),
  labelled({ type: "secretRef" as const }, "workflows:sharedVariables.valueType.secretRef"),
]

export const RESOURCE_KINDS: { kind: ResourceKind; readonly label: string }[] = [
  labelled({ kind: "directory" as const }, "workflows:sharedVariables.kind.directory"),
  labelled({ kind: "application" as const }, "workflows:sharedVariables.kind.application"),
  labelled({ kind: "terminal" as const }, "workflows:sharedVariables.kind.terminal"),
  labelled(
    { kind: "dockerEnvironment" as const },
    "workflows:sharedVariables.kind.dockerEnvironment",
  ),
  labelled({ kind: "service" as const }, "workflows:sharedVariables.kind.service"),
  labelled({ kind: "url" as const }, "workflows:sharedVariables.kind.url"),
  labelled({ kind: "other" as const }, "workflows:sharedVariables.kind.other"),
]

export function typeLabel(type: VariableType): string {
  const label = VALUE_TYPES.find((t) => t.type === type.type)?.label ?? type.type
  if (type.type === "resource" && type.kind) {
    return `${label} (${RESOURCE_KINDS.find((k) => k.kind === type.kind)?.label ?? type.kind})`
  }
  return label
}

const SCOPE_TEXT = {
  global: {
    label: "workflows:sharedVariables.scope.global",
    hint: "workflows:sharedVariables.scope.globalHint",
  },
  profile: {
    label: "workflows:sharedVariables.scope.profile",
    hint: "workflows:sharedVariables.scope.profileHint",
  },
  project: {
    label: "workflows:sharedVariables.scope.project",
    hint: "workflows:sharedVariables.scope.projectHint",
  },
} as const satisfies Record<string, { label: TranslationKey; hint: TranslationKey }>

/** Scopes whose values are stored; execution values exist only during a run. */
export const STORED_SCOPES: {
  scope: VariableScope
  readonly label: string
  readonly hint: string
}[] = (["global", "profile", "project"] as const).map((scope) => ({
  scope,
  get label() {
    return i18n.t(SCOPE_TEXT[scope].label)
  },
  get hint() {
    return i18n.t(SCOPE_TEXT[scope].hint)
  },
}))

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
      if (!/^-?\d+$/.test(trimmed))
        return { error: i18n.t("workflows:sharedVariables.parse.wholeNumber") }
      const number = Number(trimmed)
      if (!Number.isSafeInteger(number))
        return { error: i18n.t("workflows:sharedVariables.parse.tooLarge") }
      return { value: { type: "integer", value: number } }
    }
    case "boolean":
      return { value: { type: "boolean", value: text === "true" } }
    case "projectId":
      return text
        ? { value: { type: "projectId", value: text } }
        : { error: i18n.t("workflows:sharedVariables.parse.project") }
    case "profileId":
      return text
        ? { value: { type: "profileId", value: text } }
        : { error: i18n.t("workflows:sharedVariables.parse.profile") }
    case "resource": {
      const trimmed = text.trim()
      if (!trimmed) return { error: i18n.t("workflows:sharedVariables.parse.resource") }
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
        return { error: i18n.t("workflows:sharedVariables.parse.json") }
      }
    case "secretRef":
      return text
        ? { value: { type: "secretRef", value: text } }
        : { error: i18n.t("workflows:sharedVariables.parse.credential") }
  }
}

/** How a value is shown; sensitive variables never show theirs. */
export function formatValue(
  definition: VariableDefinition,
  value: VariableValue | undefined,
  projects: Project[],
  profiles: ProfileSummary[],
): string {
  if (definition.sensitive) return i18n.t("workflows:sharedVariables.shown.redacted")
  if (!value) return i18n.t("workflows:sharedVariables.notSet")
  switch (value.type) {
    case "string":
      return value.value === "" ? i18n.t("workflows:sharedVariables.shown.empty") : value.value
    case "integer":
      return String(value.value)
    case "boolean":
      return value.value
        ? i18n.t("workflows:sharedVariables.shown.yes")
        : i18n.t("workflows:sharedVariables.shown.no")
    case "projectId": {
      const project = projects.find((p) => p.id === value.value)
      return project
        ? `${project.key} — ${project.name}`
        : i18n.t("workflows:sharedVariables.shown.unknownProject")
    }
    case "profileId":
      return (
        profiles.find((p) => p.id === value.value)?.name ??
        i18n.t("workflows:sharedVariables.shown.unknownProfile")
      )
    case "resource":
      return value.value.type === "fixed"
        ? i18n.t("workflows:sharedVariables.shown.resource", { key: value.value.key })
        : i18n.t("workflows:sharedVariables.shown.projectRole", { role: value.value.role })
    case "json":
      return JSON.stringify(value.value)
    case "secretRef":
      // The name of the credential, never its secret.
      return i18n.t("workflows:sharedVariables.shown.credential", { name: value.value })
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
      return i18n.t("workflows:sharedVariables.slot.global")
    case "profile":
      return i18n
        .t("workflows:sharedVariables.slot.profile", {
          name: profiles.find((p) => p.id === scope.profile)?.name ?? "",
        })
        .trim()
    case "project": {
      const project = projects.find((p) => p.id === scope.project)
      return project
        ? i18n.t("workflows:sharedVariables.slot.project", { key: project.key })
        : i18n.t("workflows:sharedVariables.slot.anyProject")
    }
    case "execution":
      return i18n.t("workflows:sharedVariables.slot.execution")
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
        return i18n.t("workflows:sharedVariables.reference.action", { owner: ownerName }) + where
      case "workflow":
        return (
          (use.node
            ? i18n.t("workflows:sharedVariables.reference.workflowNode", {
                owner: ownerName,
                node: use.node,
              })
            : i18n.t("workflows:sharedVariables.reference.workflow", { owner: ownerName })) + where
        )
      case "binding":
        return (
          i18n.t("workflows:sharedVariables.reference.binding", {
            owner: ownerName,
            control: controlLabel(use.control),
          }) + where
        )
      case "trigger":
        return i18n.t("workflows:sharedVariables.reference.trigger", {
          owner: ownerName,
          trigger: describeTrigger(use.kind),
        })
    }
  })()
  return `${profileName}: ${text}`
}

/**
 * A fallback catalog entry whose label and description are read in the
 * window's language when used; the reference itself is never translated.
 */
function systemVariable(
  fields: Omit<CatalogEntry, "label" | "description">,
  label: TranslationKey,
  description?: TranslationKey,
): CatalogEntry {
  const entry = labelled(fields, label)
  return description
    ? Object.defineProperty(entry, "description", {
        enumerable: true,
        get: () => i18n.t(description),
      })
    : entry
}

/**
 * The read-only references every context offers (docs/action-architecture.md
 * §6), shown when the agent cannot list them (older agent, not running).
 */
export const SYSTEM_VARIABLES: CatalogEntry[] = [
  systemVariable(
    {
      reference: "system.project.current.id",
      group: "system",
      valueType: { type: "projectId" },
      dynamic: true,
      availability: "available",
    },
    "workflows:systemVariables.projectId.label",
    "workflows:systemVariables.projectId.description",
  ),
  systemVariable(
    {
      reference: "system.project.current.key",
      group: "system",
      valueType: { type: "string" },
      dynamic: true,
      availability: "available",
    },
    "workflows:systemVariables.projectKey.label",
  ),
  systemVariable(
    {
      reference: "system.project.current.name",
      group: "system",
      valueType: { type: "string" },
      dynamic: true,
      availability: "available",
    },
    "workflows:systemVariables.projectName.label",
  ),
  systemVariable(
    {
      reference: "system.profile.current.id",
      group: "system",
      valueType: { type: "profileId" },
      dynamic: true,
      availability: "available",
    },
    "workflows:systemVariables.profileId.label",
  ),
  systemVariable(
    {
      reference: "system.profile.current.name",
      group: "system",
      valueType: { type: "string" },
      dynamic: true,
      availability: "available",
    },
    "workflows:systemVariables.profileName.label",
  ),
  systemVariable(
    {
      reference: "system.device.current.id",
      group: "system",
      valueType: { type: "string" },
      dynamic: true,
      availability: "available",
    },
    "workflows:systemVariables.device.label",
    "workflows:systemVariables.device.description",
  ),
  systemVariable(
    {
      reference: "system.platform",
      group: "system",
      valueType: { type: "string" },
      availability: "available",
    },
    "workflows:systemVariables.platform.label",
    "workflows:systemVariables.platform.description",
  ),
  systemVariable(
    {
      reference: "workflow.id",
      group: "workflow",
      valueType: { type: "string" },
      dynamic: true,
      availability: "runtimeOnly",
    },
    "workflows:systemVariables.workflow.label",
    "workflows:systemVariables.workflow.description",
  ),
  systemVariable(
    {
      reference: "workflow.execution.id",
      group: "workflow",
      valueType: { type: "string" },
      dynamic: true,
      availability: "runtimeOnly",
    },
    "workflows:systemVariables.execution.label",
    "workflows:systemVariables.execution.description",
  ),
]
