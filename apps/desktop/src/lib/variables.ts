// What the variable picker and the live value inspector need besides the
// agent: the grammar of references in text, type compatibility, catalog
// filtering and how statuses and contexts are described. Values are never
// computed here; they come from the agent's resolver (`previewReferences`).
// See docs/action-architecture.md §5, §6 and §11.

import type {
  CatalogEntry,
  CatalogFilter,
  DeviceId,
  MissingReason,
  ParameterKind,
  ProjectSource,
  ReferenceStatus,
  ResolutionContext,
  ValueKind,
  ValueSource,
  VariableGroup,
  VariableScope,
  VariableType,
  VariableValue,
} from "./types"
import { type TranslationKey, i18n } from "@/i18n"

/** The node whose outputs are the workflow's variables (`steps.vars.output.<key>`). */
export const WORKFLOW_VARIABLES_NODE = "vars"

/** Tauri event carrying the agent's context events (protocol 7, see §10). */
export const VARIABLES_EVENT = "agent-variables"

/** Context events of the agent; none of them carries a value. */
export type VariableContextEvent =
  | {
      type: "projectChanged"
      profile: string
      project: string | null
      source: ProjectSource
      plugin?: string
    }
  | { type: "profileChanged"; profile: string }
  | { type: "sharedVariablesChanged"; keys: string[] }
  | { type: "resourceBindingsChanged" }
  | { type: "deviceContextChanged"; device: DeviceId | null }
  | { type: "executionContextChanged"; execution: string; project: string | null }

/**
 * Whether `event` can change what references resolve to in `context`. A
 * change of the application's active project does not touch a context
 * that uses a run's own project.
 */
export function eventAffects(event: VariableContextEvent, context: ResolutionContext): boolean {
  switch (event.type) {
    case "projectChanged":
      return (
        event.profile === context.profile &&
        (context.projectSource ?? "application") === "application"
      )
    case "profileChanged":
      return event.profile === context.profile
    case "executionContextChanged":
      return event.execution === context.execution
    case "sharedVariablesChanged":
    case "resourceBindingsChanged":
    case "deviceContextChanged":
      return true
  }
}

// --- References in text -----------------------------------------------------

/** `{{ reference }}`, as inserted into text. */
export function formatReference(reference: string): string {
  return `{{ ${reference.trim()} }}`
}

const ROOTS = ["steps", "shared", "system", "workflow"] as const

/**
 * Namespaces of the modules that declare variables: the only module roots
 * a reference may have. Mirrors `MODULE_NAMESPACES` in deckpilot-core
 * (checked by a Rust test); any other root is not a reference.
 */
export const MODULE_NAMESPACES = ["projects", "workSession"]

/**
 * The root of every plugin's variables:
 * `plugins.<publisherCamel>.<nameCamel>.<path>`. Mirrors `PLUGIN_ROOT` in
 * deckpilot-core (checked by a Rust test). Not a list: such a reference is
 * a module reference whether or not the plugin is installed; the agent
 * decides whether it is valid and the catalog names its variables.
 */
export const PLUGIN_ROOT = "plugins"

/**
 * The root of a reference (`shared`, `system`, …), `module` for a known
 * module's namespace (`projects.current.key`) or a plugin's
 * (`plugins.acme.githubPrs.open`), or `null` for none of them.
 */
export function referenceRoot(reference: string): (typeof ROOTS)[number] | "module" | null {
  const segments = reference.trim().split(".")
  const root = segments[0] ?? ""
  if ((ROOTS as readonly string[]).includes(root)) return root as (typeof ROOTS)[number]
  // `plugins`, publisher, name and at least one segment of the variable.
  if (root === PLUGIN_ROOT) return segments.length >= 4 ? "module" : null
  return segments.length >= 2 && MODULE_NAMESPACES.includes(root) ? "module" : null
}

/** `steps.<node>.output.<key>`: outputs of earlier nodes, known only while running. */
export function isStepReference(reference: string): boolean {
  return referenceRoot(reference) === "steps"
}

export interface TemplateReferences {
  /** The references, trimmed, in order, without duplicates. */
  references: string[]
  /** A `{{` without its `}}`. */
  unclosed: boolean
}

/** Every `{{ … }}` of a template; the agent decides whether each is valid. */
export function templateReferences(template: string): TemplateReferences {
  const references: string[] = []
  let rest = template
  for (;;) {
    const start = rest.indexOf("{{")
    if (start < 0) return { references, unclosed: false }
    const after = rest.slice(start + 2)
    const end = after.indexOf("}}")
    if (end < 0) return { references, unclosed: true }
    const reference = after.slice(0, end).trim()
    if (!references.includes(reference)) references.push(reference)
    rest = after.slice(end + 2)
  }
}

/** Where an autocomplete started by typing `{{` is, and what was typed after it. */
export interface Autocomplete {
  /** Index of the `{{`. */
  start: number
  query: string
}

/**
 * The autocomplete at `cursor`: a `{{` before it that is not closed yet,
 * followed only by characters of a reference.
 */
export function autocompleteAt(text: string, cursor: number): Autocomplete | null {
  const before = text.slice(0, cursor)
  const start = before.lastIndexOf("{{")
  if (start < 0) return null
  const typed = before.slice(start + 2)
  if (!/^[ A-Za-z0-9_.]*$/.test(typed)) return null
  return { start, query: typed.trim() }
}

/**
 * `text` with `{{ reference }}` in place of `[start, end)`, and the cursor
 * after it. A `}}` right after `end` (typed before picking) is replaced too.
 */
export function insertReference(
  text: string,
  start: number,
  end: number,
  reference: string,
): { text: string; cursor: number } {
  const closing = text.slice(start, end).startsWith("{{") ? /^ *\}\}/.exec(text.slice(end)) : null
  const tail = text.slice(end + (closing?.[0].length ?? 0))
  const inserted = formatReference(reference)
  return { text: text.slice(0, start) + inserted + tail, cursor: start + inserted.length }
}

// --- Types ------------------------------------------------------------------

const TEXT_KINDS = new Set<ParameterKind["type"]>(["text", "lines", "pairs", "code"])

/**
 * Whether a value of `type` can fill a parameter of `kind` as a whole-value
 * reference. Mirrors `VariableType::assignable_to` in deckpilot-core.
 */
export function assignableTo(type: VariableType, kind: ParameterKind): boolean {
  switch (type.type) {
    case "secretRef":
      return kind.type === "credential"
    case "integer":
      return kind.type === "integer" || TEXT_KINDS.has(kind.type)
    case "boolean":
      return kind.type === "boolean" || TEXT_KINDS.has(kind.type)
    case "projectId":
      return kind.type === "project" || TEXT_KINDS.has(kind.type)
    case "string":
      return kind.type === "choice" || TEXT_KINDS.has(kind.type)
    case "profileId":
      return TEXT_KINDS.has(kind.type)
    case "resource":
      return (
        kind.type === "resource" &&
        (type.kind === undefined || kind.kind === undefined || type.kind === kind.kind)
      )
    case "json":
      return (kind.type === "code" && kind.language === "json") || kind.type === "text"
  }
}

/** What a reference inside a template is checked as: text. */
export const TEMPLATE_KIND: ParameterKind = { type: "text", multiline: false }

/** Whether a parameter of `kind` is edited as text that can hold `{{ … }}`. */
export function takesTemplate(kind: ParameterKind): boolean {
  return kind.type === "text"
}

/** The variable type of a workflow value kind (step outputs, inputs, variables). */
export function variableTypeOf(kind: ValueKind): VariableType {
  switch (kind) {
    case "integer":
      return { type: "integer" }
    case "boolean":
      return { type: "boolean" }
    default:
      return { type: "string" }
  }
}

const TYPE_LABEL: Record<VariableType["type"], TranslationKey> = {
  string: "editor:variableText.type.string",
  integer: "editor:variableText.type.integer",
  boolean: "editor:variableText.type.boolean",
  projectId: "editor:variableText.type.projectId",
  profileId: "editor:variableText.type.profileId",
  resource: "editor:variableText.type.resource",
  json: "editor:variableText.type.json",
  secretRef: "editor:variableText.type.secretRef",
}

/** A value type in words, in the window's language. */
export function typeLabel(type: VariableType): string {
  return type.type === "resource" && type.kind
    ? i18n.t("editor:variableText.type.resourceOfKind", { kind: type.kind })
    : i18n.t(TYPE_LABEL[type.type])
}

/** Translation keys, resolved with `t` when shown. */
export const SCOPE_LABEL: Record<VariableScope, TranslationKey> = {
  global: "editor:variableText.scope.global",
  profile: "editor:variableText.scope.profile",
  project: "editor:variableText.scope.project",
  execution: "editor:variableText.scope.execution",
}

// --- Catalog ----------------------------------------------------------------

/** Groups in the order the picker shows them. */
export const GROUPS: { group: VariableGroup; label: TranslationKey }[] = [
  { group: "shared", label: "editor:variableText.group.shared" },
  { group: "system", label: "editor:variableText.group.system" },
  { group: "profile", label: "editor:variableText.group.profile" },
  { group: "project", label: "editor:variableText.group.project" },
  { group: "module", label: "editor:variableText.group.module" },
  { group: "workflow", label: "editor:variableText.group.workflow" },
  { group: "stepOutput", label: "editor:variableText.group.stepOutput" },
]

/** Mirrors `CatalogFilter::matches` in deckpilot-core, for entries listed here. */
export function matchesFilter(entry: CatalogEntry, filter: CatalogFilter): boolean {
  if (filter.groups?.length && !filter.groups.includes(entry.group)) return false
  if (filter.compatibleWith && !assignableTo(entry.valueType, filter.compatibleWith)) return false
  if (!filter.includeUnavailable && entry.availability === "notInContext") return false
  const haystack = [
    entry.reference,
    entry.label,
    entry.description ?? "",
    entry.scope ?? "",
    entry.valueType.type,
    entry.module ?? "",
  ]
    .join("\n")
    .toLowerCase()
  return (filter.query ?? "")
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => haystack.includes(word.toLowerCase()))
}

/** Entries sorted into the picker's groups (labels in the window's language), empty groups left out. */
export function groupEntries(
  entries: CatalogEntry[],
): { group: VariableGroup; label: string; entries: CatalogEntry[] }[] {
  return GROUPS.map(({ group, label }) => ({
    group,
    label: i18n.t(label),
    entries: entries.filter((entry) => entry.group === group),
  })).filter((g) => g.entries.length > 0)
}

/** An output of an earlier node (or a workflow variable), as the builder knows it. */
export interface StepOutput {
  node: string
  output: string
  label: string
  kind: ValueKind
  sensitive?: boolean
}

/** Catalog entries for outputs of the graph being edited: metadata only, run time only. */
export function stepOutputEntries(outputs: StepOutput[]): CatalogEntry[] {
  return outputs.map((o) => ({
    reference: `steps.${o.node}.output.${o.output}`,
    label: o.label,
    group: o.node === WORKFLOW_VARIABLES_NODE ? "workflow" : "stepOutput",
    valueType: variableTypeOf(o.kind),
    sensitive: o.sensitive,
    dynamic: true,
    availability: "runtimeOnly",
  }))
}

const SYSTEM_TEXT = {
  projectId: {
    label: "editor:variableText.system.projectId.label",
    description: "editor:variableText.system.projectId.description",
  },
  projectKey: {
    label: "editor:variableText.system.projectKey.label",
    description: "editor:variableText.system.projectKey.description",
  },
  projectName: {
    label: "editor:variableText.system.projectName.label",
    description: "editor:variableText.system.projectName.description",
  },
  profileId: {
    label: "editor:variableText.system.profileId.label",
    description: "editor:variableText.system.profileId.description",
  },
  profileName: {
    label: "editor:variableText.system.profileName.label",
    description: "editor:variableText.system.profileName.description",
  },
  device: {
    label: "editor:variableText.system.device.label",
    description: "editor:variableText.system.device.description",
  },
  platform: {
    label: "editor:variableText.system.platform.label",
    description: "editor:variableText.system.platform.description",
  },
  workflowId: {
    label: "editor:variableText.system.workflowId.label",
    description: "editor:variableText.system.workflowId.description",
  },
  executionId: {
    label: "editor:variableText.system.executionId.label",
    description: "editor:variableText.system.executionId.description",
  },
} satisfies Record<string, { label: TranslationKey; description: TranslationKey }>

type SystemText = keyof typeof SYSTEM_TEXT

/**
 * System references, for listing when the agent cannot (an agent before
 * protocol 7). Metadata only: without the agent there are no values.
 */
export const SYSTEM_ENTRIES: CatalogEntry[] = [
  system("system.project.current.id", "projectId", "projectId"),
  system("system.project.current.key", "projectKey"),
  system("system.project.current.name", "projectName"),
  system("system.profile.current.id", "profileId", "profileId"),
  system("system.profile.current.name", "profileName"),
  system("system.device.current.id", "device"),
  system("system.platform", "platform"),
  system("workflow.id", "workflowId", "string", { group: "workflow" }),
  system("workflow.execution.id", "executionId", "string", {
    group: "workflow",
    availability: "runtimeOnly",
  }),
]

/**
 * A system entry whose label and description are read in the window's
 * language when used (the reference stays as it is).
 */
function system(
  reference: string,
  text: SystemText,
  type: "string" | "projectId" | "profileId" = "string",
  fields: Partial<Pick<CatalogEntry, "group" | "availability">> = {},
): CatalogEntry {
  const entry = {
    reference,
    group: "system",
    valueType: { type },
    dynamic: reference !== "system.platform",
    availability: "available",
    ...fields,
  } as Omit<CatalogEntry, "label" | "description">
  return Object.defineProperties(entry, {
    label: { enumerable: true, get: () => i18n.t(SYSTEM_TEXT[text].label) },
    description: { enumerable: true, get: () => i18n.t(SYSTEM_TEXT[text].description) },
  }) as CatalogEntry
}

// --- Statuses, values and contexts ------------------------------------------

const MISSING: Record<MissingReason, TranslationKey> = {
  unknownVariable: "editor:variableText.missing.unknownVariable",
  noValue: "editor:variableText.missing.noValue",
  noProject: "editor:variableText.missing.noProject",
  noDevice: "editor:variableText.missing.noDevice",
  noWorkflow: "editor:variableText.missing.noWorkflow",
  noExecution: "editor:variableText.missing.noExecution",
  noOutput: "editor:variableText.missing.noOutput",
  otherProfile: "editor:variableText.missing.otherProfile",
}

export type StatusTone = "ok" | "info" | "error"

export interface StatusDescription {
  tone: StatusTone
  label: string
  detail?: string
}

/** How a status is shown, in the window's language. Never contains a value. */
export function describeStatus(status: ReferenceStatus): StatusDescription {
  switch (status.status) {
    case "resolved":
      return { tone: "ok", label: i18n.t("editor:variableText.status.resolved") }
    case "redacted":
      return {
        tone: "info",
        label: i18n.t("editor:variableText.status.redacted"),
        detail: i18n.t("editor:variableText.status.redactedDetail"),
      }
    case "unavailableUntilRuntime":
      return {
        tone: "info",
        label: i18n.t("editor:variableText.status.runtime"),
        detail: i18n.t("editor:variableText.status.runtimeDetail"),
      }
    case "missing":
      return {
        tone: "error",
        label: i18n.t("editor:variableText.status.missing"),
        detail: i18n.t(MISSING[status.reason]),
      }
    case "typeMismatch":
      return {
        tone: "error",
        label: i18n.t("editor:variableText.status.typeMismatch"),
        detail: i18n.t("editor:variableText.status.typeMismatchDetail", {
          type: typeLabel(status.found),
        }),
      }
    case "permissionDenied":
      return {
        tone: "error",
        label: i18n.t("editor:variableText.status.permissionDenied"),
        detail: i18n.t("editor:variableText.status.permissionDeniedDetail"),
      }
    case "unavailable":
      return {
        tone: "error",
        label: i18n.t("editor:variableText.status.unavailable"),
        detail: i18n.t("editor:variableText.status.unavailableDetail", { module: status.owner }),
      }
    case "invalid":
      return {
        tone: "error",
        label: i18n.t("editor:variableText.status.invalid"),
        detail: status.message,
      }
  }
}

/** A resolved value as text. Only for values that are not sensitive. */
export function formatValue(value: VariableValue): string {
  switch (value.type) {
    case "string":
    case "projectId":
    case "profileId":
    case "secretRef":
      return value.value
    case "integer":
      return String(value.value)
    case "boolean":
      return value.value ? "true" : "false"
    case "resource":
      return value.value.type === "fixed"
        ? value.value.key
        : i18n.t("editor:variableText.projectRole", { role: value.value.role })
    case "json":
      return JSON.stringify(value.value)
  }
}

const SOURCE_LABEL: Record<VariableScope, TranslationKey> = {
  global: "editor:variableText.source.global",
  profile: "editor:variableText.source.profile",
  project: "editor:variableText.source.project",
  execution: "editor:variableText.source.execution",
}

/** Where a value comes from, in the window's language. */
export function describeSource(source: ValueSource): string {
  switch (source.type) {
    case "scope":
      return i18n.t(SOURCE_LABEL[source.scope.kind])
    case "default":
      return i18n.t("editor:variableText.source.default")
    case "system":
      return i18n.t("editor:variableText.source.system")
    case "stepOutput":
      return i18n.t("editor:variableText.source.stepOutput", { node: source.node })
  }
}

/** Translation keys, resolved with `t` when shown. */
export const AVAILABILITY_LABEL: Record<CatalogEntry["availability"], TranslationKey> = {
  available: "editor:variableText.availability.available",
  runtimeOnly: "editor:variableText.availability.runtimeOnly",
  notInContext: "editor:variableText.availability.notInContext",
  unavailable: "editor:variableText.availability.unavailable",
}

/** Names shown for the ids of a context. */
export interface ContextNames {
  profile?: string
  project?: string
  device?: string
  workflow?: string
}

/**
 * The context previews are resolved in, in words. Says where the project
 * comes from, so the application's active project is never taken for the
 * project of a run.
 */
export function describeContext(context: ResolutionContext, names: ContextNames = {}): string {
  const parts = [
    i18n.t("editor:variableText.context.profile", { profile: names.profile ?? context.profile }),
  ]
  const project = names.project ?? context.project
  switch (context.projectSource ?? "application") {
    case "application":
      parts.push(
        project
          ? i18n.t("editor:variableText.context.activeProject", { project })
          : i18n.t("editor:variableText.context.theActiveProject"),
      )
      break
    case "execution":
      parts.push(
        context.execution
          ? i18n.t("editor:variableText.context.runProject", { run: context.execution })
          : i18n.t("editor:variableText.context.runProjectLater"),
      )
      break
    case "explicit":
      parts.push(
        project
          ? i18n.t("editor:variableText.context.chosenProject", { project })
          : i18n.t("editor:variableText.context.noProject"),
      )
      break
  }
  if (context.workflow) {
    parts.push(
      i18n.t("editor:variableText.context.workflow", {
        workflow: names.workflow ?? context.workflow,
      }),
    )
  }
  if (context.device) {
    parts.push(
      i18n.t("editor:variableText.context.device", {
        device: names.device ?? `${context.device.model}#${context.device.serial}`,
      }),
    )
  }
  return parts.join(" · ")
}
