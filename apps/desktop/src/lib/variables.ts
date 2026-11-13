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

/** The node whose outputs are the workflow's variables (`steps.vars.output.<key>`). */
export const WORKFLOW_VARIABLES_NODE = "vars"

/** Tauri event carrying the agent's context events (protocol 7, see §10). */
export const VARIABLES_EVENT = "agent-variables"

/** Context events of the agent; none of them carries a value. */
export type VariableContextEvent =
  | { type: "projectChanged"; profile: string; project: string | null; source: ProjectSource }
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

/** A module namespace: a lowercase start, then letters, digits and `_`. */
const MODULE_ROOT = /^[a-z][A-Za-z0-9_]*$/

/**
 * The root of a reference (`shared`, `system`, …), `module` for a module's
 * namespace (`projects.current.key`), or `null` for none of them.
 */
export function referenceRoot(reference: string): (typeof ROOTS)[number] | "module" | null {
  const segments = reference.trim().split(".")
  const root = segments[0] ?? ""
  if ((ROOTS as readonly string[]).includes(root)) return root as (typeof ROOTS)[number]
  return segments.length >= 2 && MODULE_ROOT.test(root) ? "module" : null
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

const TYPE_LABEL: Record<VariableType["type"], string> = {
  string: "text",
  integer: "number",
  boolean: "yes/no",
  projectId: "project",
  profileId: "profile",
  resource: "resource",
  json: "JSON",
  secretRef: "credential name",
}

export function typeLabel(type: VariableType): string {
  return type.type === "resource" && type.kind ? `resource (${type.kind})` : TYPE_LABEL[type.type]
}

export const SCOPE_LABEL: Record<VariableScope, string> = {
  global: "global",
  profile: "profile",
  project: "project",
  execution: "run",
}

// --- Catalog ----------------------------------------------------------------

/** Groups in the order the picker shows them. */
export const GROUPS: { group: VariableGroup; label: string }[] = [
  { group: "shared", label: "Shared" },
  { group: "system", label: "System" },
  { group: "profile", label: "Profile" },
  { group: "project", label: "Project" },
  { group: "module", label: "Modules" },
  { group: "workflow", label: "Workflow" },
  { group: "stepOutput", label: "Step outputs" },
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

/** Entries sorted into the picker's groups, empty groups left out. */
export function groupEntries(
  entries: CatalogEntry[],
): { group: VariableGroup; label: string; entries: CatalogEntry[] }[] {
  return GROUPS.map(({ group, label }) => ({
    group,
    label,
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

/**
 * System references, for listing when the agent cannot (an agent before
 * protocol 7). Metadata only: without the agent there are no values.
 */
export const SYSTEM_ENTRIES: CatalogEntry[] = [
  system(
    "system.project.current.id",
    "Project id",
    "Id of the project in the context",
    "projectId",
  ),
  system("system.project.current.key", "Project key", "Key of the project in the context"),
  system("system.project.current.name", "Project name", "Name of the project in the context"),
  system(
    "system.profile.current.id",
    "Profile id",
    "Id of the profile in the context",
    "profileId",
  ),
  system("system.profile.current.name", "Profile name", "Name of the profile in the context"),
  system("system.device.current.id", "Device", "The device whose control started the run"),
  system("system.platform", "Platform", "Operating system of this computer"),
  {
    ...system("workflow.id", "Workflow id", "Id of the running workflow"),
    group: "workflow",
  },
  {
    ...system("workflow.execution.id", "Run id", "Id of this run of the workflow"),
    group: "workflow",
    availability: "runtimeOnly",
  },
]

function system(
  reference: string,
  label: string,
  description: string,
  type: "string" | "projectId" | "profileId" = "string",
): CatalogEntry {
  return {
    reference,
    label,
    description,
    group: "system",
    valueType: { type },
    dynamic: reference !== "system.platform",
    availability: "available",
  }
}

// --- Statuses, values and contexts ------------------------------------------

const MISSING: Record<MissingReason, string> = {
  unknownVariable: "There is no such variable.",
  noValue: "No value is set and there is no default.",
  noProject: "No project is active.",
  noDevice: "No device is in the context.",
  noWorkflow: "No workflow is in the context.",
  noExecution: "No run is in the context.",
  noOutput: "The step has not produced it.",
  otherProfile: "The value belongs to another profile.",
}

export type StatusTone = "ok" | "info" | "error"

export interface StatusDescription {
  tone: StatusTone
  label: string
  detail?: string
}

/** How a status is shown. Never contains a value. */
export function describeStatus(status: ReferenceStatus): StatusDescription {
  switch (status.status) {
    case "resolved":
      return { tone: "ok", label: "Resolved" }
    case "redacted":
      return {
        tone: "info",
        label: "Redacted",
        detail: "The value is sensitive and never shown.",
      }
    case "unavailableUntilRuntime":
      return {
        tone: "info",
        label: "Available when it runs",
        detail: "The value exists only while the workflow runs.",
      }
    case "missing":
      return { tone: "error", label: "Missing", detail: MISSING[status.reason] }
    case "typeMismatch":
      return {
        tone: "error",
        label: "Type mismatch",
        detail: `The value is ${typeLabel(status.found)}, which does not fit this field.`,
      }
    case "permissionDenied":
      return {
        tone: "error",
        label: "Permission denied",
        detail: "This action may not read it.",
      }
    case "unavailable":
      return {
        tone: "error",
        label: "Unavailable",
        detail: `The ${status.owner} module is disabled or not installed. The reference is kept.`,
      }
    case "invalid":
      return { tone: "error", label: "Invalid reference", detail: status.message }
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
        : `role ${value.value.role} of the project`
    case "json":
      return JSON.stringify(value.value)
  }
}

const SOURCE_LABEL: Record<VariableScope, string> = {
  global: "Global value",
  profile: "Profile value",
  project: "Project value",
  execution: "Value set by the run",
}

export function describeSource(source: ValueSource): string {
  switch (source.type) {
    case "scope":
      return SOURCE_LABEL[source.scope.kind]
    case "default":
      return "Default value"
    case "system":
      return "From the context"
    case "stepOutput":
      return `Output of step “${source.node}”`
  }
}

export const AVAILABILITY_LABEL: Record<CatalogEntry["availability"], string> = {
  available: "Available in this context",
  runtimeOnly: "Only while the workflow runs",
  notInContext: "Not in this context",
  unavailable: "Its module is disabled or not installed",
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
  const parts = [`Profile ${names.profile ?? context.profile}`]
  const project = names.project ?? context.project
  switch (context.projectSource ?? "application") {
    case "application":
      parts.push(project ? `active project ${project}` : "the active project")
      break
    case "execution":
      parts.push(
        context.execution
          ? `the project of run ${context.execution}`
          : "the project of the run (known when it runs)",
      )
      break
    case "explicit":
      parts.push(project ? `project ${project} (chosen for this preview)` : "no project")
      break
  }
  if (context.workflow) parts.push(`workflow ${names.workflow ?? context.workflow}`)
  if (context.device) {
    parts.push(`device ${names.device ?? `${context.device.model}#${context.device.serial}`}`)
  }
  return parts.join(" · ")
}
