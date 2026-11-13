/**
 * References to shared, system and workflow variables in node mappings
 * (`{{ shared.paths.root }}`, `{{ system.project.current.key }}`), mirroring
 * `deckpilot_core::variables::VariableRef::parse` and
 * `VariableType::assignable_to`. Only metadata: values are resolved by the
 * agent when the node runs (and previewed by the variable picker).
 */
import type { ReactNode } from "react"

import type { ParameterKind, VariableDefinition, VariableType } from "@/lib/types"

/** The read-only variables of a run and their types. */
export const SYSTEM_VARIABLES: Record<string, { type: VariableType; label: string }> = {
  "system.project.current.id": { type: { type: "projectId" }, label: "Current project" },
  "system.project.current.key": { type: { type: "string" }, label: "Current project key" },
  "system.project.current.name": { type: { type: "string" }, label: "Current project name" },
  "system.profile.current.id": { type: { type: "profileId" }, label: "Current profile" },
  "system.profile.current.name": { type: { type: "string" }, label: "Current profile name" },
  "system.device.current.id": { type: { type: "string" }, label: "Device that started the run" },
  "system.platform": { type: { type: "string" }, label: "Operating system" },
  "workflow.id": { type: { type: "string" }, label: "This workflow" },
  "workflow.execution.id": { type: { type: "string" }, label: "This run" },
}

const SEGMENT = /^[a-z][A-Za-z0-9_]{0,31}$/
const STEP = /^steps\.[a-z][a-z0-9_]{0,47}\.output\.[A-Za-z0-9_]{1,64}$/

/** Why `text` (without braces) is not a reference, or `null`. */
export function referenceSyntaxProblem(text: string): string | null {
  const reference = text.trim()
  if (!reference) return "Enter a reference, e.g. shared.paths.root"
  if (reference.startsWith("steps.")) {
    return STEP.test(reference) ? null : `“${reference}” is not steps.<node>.output.<key>`
  }
  if (reference.startsWith("shared.")) {
    const key = reference.slice("shared.".length).split(".")
    if (key.length < 2 || key.length > 4 || !key.every((s) => SEGMENT.test(s))) {
      return `“${reference}” is not shared.<namespace>.<name>`
    }
    if (["steps", "system", "workflow"].includes(key[0] ?? "")) {
      return `the namespace “${key[0] ?? ""}” is reserved`
    }
    return null
  }
  if (reference in SYSTEM_VARIABLES) return null
  return `“${reference}” is not a reference; use shared.…, system.…, workflow.… or steps.…`
}

/** The type of a non-`steps` reference, when known before the run. */
export function referenceType(
  reference: string,
  definitions: readonly VariableDefinition[] = [],
): VariableType | undefined {
  const system = SYSTEM_VARIABLES[reference]
  if (system) return system.type
  if (reference.startsWith("shared.")) {
    const key = reference.slice("shared.".length)
    return definitions.find((d) => d.key === key)?.valueType
  }
  return undefined
}

const isText = (kind: ParameterKind) =>
  kind.type === "text" || kind.type === "lines" || kind.type === "pairs" || kind.type === "code"

/**
 * Whether a whole value of `type` fills a parameter of `kind`, keeping its
 * native type (mirrors `VariableType::assignable_to`).
 */
export function assignable(type: VariableType, kind: ParameterKind): boolean {
  switch (type.type) {
    case "secretRef":
      return kind.type === "credential"
    case "integer":
      return kind.type === "integer" || isText(kind)
    case "boolean":
      return kind.type === "boolean" || isText(kind)
    case "projectId":
      return kind.type === "project" || isText(kind)
    case "string":
      return kind.type === "choice" || isText(kind)
    case "profileId":
      return isText(kind)
    case "resource":
      return kind.type === "resource" && (!type.kind || !kind.kind || type.kind === kind.kind)
    case "json":
      return (kind.type === "code" && kind.language === "json") || kind.type === "text"
  }
}

/** Values that never become text: credential names and resources. */
export function rendersAsText(type: VariableType): boolean {
  return type.type !== "secretRef" && type.type !== "resource"
}

/** What a parameter expects, for messages. */
export function expects(kind: ParameterKind): string {
  switch (kind.type) {
    case "integer":
      return "a whole number"
    case "boolean":
      return "true or false"
    case "choice":
      return "one of its choices"
    case "resource":
      return "a resource"
    case "project":
      return "a project"
    case "credential":
      return "a credential"
    default:
      return "text"
  }
}

/**
 * Hook for the universal variable picker (workstream F). The editor renders
 * it beside a reference field; it calls `onPick` with a reference without
 * braces. Until the picker exists, references are typed.
 */
export type VariablePicker = (props: {
  /** The kind of the parameter the reference fills, for type filtering. */
  expected?: ParameterKind
  onPick: (reference: string) => void
}) => ReactNode

/**
 * The picker the builder shows beside reference fields. `undefined` until
 * workstream F's component lands; F sets it here (one place), and every
 * mapping editor in the inspector picks it up.
 */
export const VARIABLE_PICKER: VariablePicker | undefined = undefined
