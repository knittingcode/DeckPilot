/**
 * Structured data mappings. The engine's format stays the same
 * (`{{ steps.<node>.output.<key> }}` and other references inside text, one
 * output reference, or one variable reference); the editor works on its
 * parts. Nothing is evaluated here: the engine resolves mappings when a
 * node runs.
 */
import type {
  GraphMapping,
  GraphReference,
  ParameterKind,
  ValueKind,
  VariableDefinition,
  VariableValue,
  WorkflowGraph,
} from "@/lib/types"

import { type Lookup, actionOf, nodeOutputs } from "./describe"
import { VARIABLES_NODE, upstream } from "./graph"
import {
  assignable,
  expects,
  referenceSyntaxProblem,
  referenceType,
  rendersAsText,
} from "./variable-reference"

export type Segment =
  | { type: "text"; text: string }
  | { type: "output"; reference: GraphReference }
  /** A shared, system or workflow variable, without braces. */
  | { type: "variable"; reference: string }

const REFERENCE = /\{\{\s*steps\.([a-z][a-z0-9_]*)\.output\.([A-Za-z0-9_.-]+)\s*\}\}/g
const VARIABLE = /\{\{\s*((?:shared|system|workflow)\.[A-Za-z0-9_.]+)\s*\}\}/g

/**
 * The parts of a template: plain text, output references and variable
 * references. Anything else in braces stays text, so editing never changes
 * what was typed.
 */
export function parseTemplate(template: string): Segment[] {
  const found: { index: number; length: number; segment: Segment }[] = []
  for (const match of template.matchAll(REFERENCE)) {
    found.push({
      index: match.index,
      length: match[0].length,
      segment: { type: "output", reference: { node: match[1] ?? "", output: match[2] ?? "" } },
    })
  }
  for (const match of template.matchAll(VARIABLE)) {
    const reference = match[1] ?? ""
    if (referenceSyntaxProblem(reference) !== null) continue
    found.push({
      index: match.index,
      length: match[0].length,
      segment: { type: "variable", reference },
    })
  }
  found.sort((a, b) => a.index - b.index)
  const segments: Segment[] = []
  let last = 0
  for (const { index, length, segment } of found) {
    if (index > last) segments.push({ type: "text", text: template.slice(last, index) })
    segments.push(segment)
    last = index + length
  }
  if (last < template.length) segments.push({ type: "text", text: template.slice(last) })
  return segments
}

export function serializeTemplate(segments: Segment[]): string {
  return segments
    .map((s) => {
      switch (s.type) {
        case "text":
          return s.text
        case "output":
          return `{{ steps.${s.reference.node}.output.${s.reference.output} }}`
        case "variable":
          return `{{ ${s.reference.trim()} }}`
      }
    })
    .join("")
}

/** `steps.<node>.output.<key>` as a reference, or `undefined`. */
function stepReference(reference: string): GraphReference | undefined {
  const match = /^steps\.([a-z][a-z0-9_]*)\.output\.([A-Za-z0-9_]+)$/.exec(reference.trim())
  return match ? { node: match[1] ?? "", output: match[2] ?? "" } : undefined
}

/** Node outputs a mapping uses. */
export function mappingReferences(mapping: GraphMapping): GraphReference[] {
  if (mapping.type === "output") return [mapping.reference]
  if (mapping.type === "variable") {
    const step = stepReference(mapping.reference)
    return step ? [step] : []
  }
  return parseTemplate(mapping.template).flatMap((s) => (s.type === "output" ? [s.reference] : []))
}

/** Shared, system and workflow variables a mapping uses. */
export function mappingVariables(mapping: GraphMapping): string[] {
  if (mapping.type === "output") return []
  if (mapping.type === "variable") {
    return stepReference(mapping.reference) ? [] : [mapping.reference.trim()]
  }
  return parseTemplate(mapping.template).flatMap((s) =>
    s.type === "variable" ? [s.reference] : [],
  )
}

/**
 * Problems of the variables in `mapping` that can be told before the run:
 * syntax, unknown shared variables (when the definitions are known), types
 * that cannot fill the parameter, credential names or resources in text.
 * Mirrors the engine's validation.
 */
export function variableProblems(
  mapping: GraphMapping,
  parameter: ParameterKind | undefined,
  definitions?: readonly VariableDefinition[],
): string[] {
  const problems: string[] = []
  const known = (reference: string) => {
    const syntax = referenceSyntaxProblem(reference)
    if (syntax) {
      problems.push(syntax)
      return undefined
    }
    const type = referenceType(reference, definitions)
    if (!type && definitions && reference.startsWith("shared.")) {
      problems.push(`there is no shared variable “${reference.slice("shared.".length)}”`)
    }
    return type
  }
  if (mapping.type === "variable") {
    if (stepReference(mapping.reference)) return problems
    const type = known(mapping.reference)
    if (type && parameter && !assignable(type, parameter)) {
      problems.push(
        `{{ ${mapping.reference.trim()} }} is a ${type.type} value, the parameter expects ${expects(parameter)}`,
      )
    }
    if (mapping.default && parameter && !assignable(valueType(mapping.default), parameter)) {
      problems.push(
        `the default is a ${mapping.default.type} value, the parameter expects ${expects(parameter)}`,
      )
    }
  } else if (mapping.type === "template") {
    for (const reference of mappingVariables(mapping)) {
      const type = known(reference)
      if (type && !rendersAsText(type)) {
        problems.push(`{{ ${reference} }} is a ${type.type} value and cannot be part of text`)
      }
    }
  }
  return problems
}

function valueType(value: VariableValue) {
  return value.type === "resource" ? { type: "resource" as const } : { type: value.type }
}

export interface OutputOption {
  reference: GraphReference
  /** "node label · output label" */
  label: string
  kind: ValueKind
  /** Kept out of history; may only go to sensitive inputs and outputs. */
  sensitive?: boolean
}

/** Declared outputs of the nodes that run before `id`. */
export function outputOptions(graph: WorkflowGraph, id: string, lookup: Lookup): OutputOption[] {
  // Variables hold the value on the node's path; readable anywhere.
  const variables: OutputOption[] = (lookup.variables ?? []).map((variable) => ({
    reference: { node: VARIABLES_NODE, output: variable.key },
    label: `Variable · ${variable.label || variable.key}`,
    kind: variable.kind,
    sensitive: variable.sensitive,
  }))
  return [...outputsBefore(graph, id, lookup), ...variables]
}

function outputsBefore(graph: WorkflowGraph, id: string, lookup: Lookup): OutputOption[] {
  return upstream(graph, id).flatMap((nodeId) => {
    const node = graph.nodes.find((n) => n.id === nodeId)
    if (!node) return []
    const { action } = actionOf(node, lookup)
    const callee =
      node.type === "call" ? lookup.workflows.find((w) => w.id === node.workflow) : undefined
    const title =
      node.label || action?.name || callee?.name || (node.type === "trigger" ? "Inputs" : nodeId)
    return nodeOutputs(node, lookup).map((output) => ({
      reference: { node: nodeId, output: output.key },
      label: `${title} · ${output.label}`,
      kind: output.kind,
      sensitive: output.sensitive,
    }))
  })
}

/** Why a reference cannot be resolved at `id`, or `null`. */
export function referenceProblem(
  graph: WorkflowGraph,
  id: string,
  reference: GraphReference,
  lookup: Lookup,
): string | null {
  if (reference.node === VARIABLES_NODE && lookup.variables?.length) {
    return lookup.variables.some((v) => v.key === reference.output)
      ? null
      : `there is no variable “${reference.output}”`
  }
  const node = graph.nodes.find((n) => n.id === reference.node)
  if (!node) return `there is no node “${reference.node}”`
  if (!upstream(graph, id).includes(reference.node)) {
    return `“${reference.node}” does not run before this node`
  }
  if (!nodeOutputs(node, lookup).some((o) => o.key === reference.output)) {
    return `“${reference.node}” has no output “${reference.output}”`
  }
  return null
}

/**
 * The parameter kind a value of `kind` is checked like: sub-workflow inputs
 * and workflow outputs are converted to their kind the way parameters are.
 */
export function parameterKindOf(kind: ValueKind): ParameterKind {
  switch (kind) {
    case "integer":
      return { type: "integer" }
    case "boolean":
      return { type: "boolean" }
    case "projectKey":
      return { type: "project" }
    default:
      return { type: "text", multiline: false }
  }
}

export const VALUE_KIND_LABEL: Record<ValueKind, string> = {
  text: "text",
  integer: "number",
  boolean: "yes/no",
  path: "path",
  url: "URL",
  projectKey: "project key",
}

export interface Compatibility {
  level: "ok" | "warning" | "error"
  message?: string
}

/**
 * Whether values of `kind` fit a parameter of `parameter` kind, mirroring
 * the engine's conversion (`mapped_parameters`): text parameters take
 * anything, numbers and yes/no must parse, projects need a project key,
 * resources cannot be mapped.
 */
export function compatibility(
  parameter: ParameterKind,
  kind: ValueKind | "text-template",
): Compatibility {
  switch (parameter.type) {
    case "text":
    case "code":
      return { level: "ok" }
    case "lines":
      return {
        level: "warning",
        message: "Each line of the value becomes one item; nothing is split on spaces.",
      }
    case "pairs":
      return {
        level: "warning",
        message: `Each line must be a name and a value separated by “${parameter.separator}”.`,
      }
    case "credential":
      return {
        level: "warning",
        message: "The value must be the name of a credential stored on this computer.",
      }
    case "integer":
      return kind === "integer"
        ? { level: "ok" }
        : { level: "warning", message: "The value must be a whole number when the node runs." }
    case "boolean":
      return kind === "boolean"
        ? { level: "ok" }
        : { level: "warning", message: "The value must be “true” or “false” when the node runs." }
    case "choice":
      return {
        level: "warning",
        message: `The value must be one of: ${parameter.options.join(", ")}.`,
      }
    case "project":
      return kind === "projectKey"
        ? { level: "ok" }
        : { level: "warning", message: "The value must be a project key of this profile." }
    case "resource":
      return { level: "error", message: "Resources cannot come from earlier nodes." }
  }
}

/**
 * What the mapped value will look like: outputs of the shown run where
 * known, otherwise a placeholder naming the output.
 */
export function previewMapping(
  mapping: GraphMapping,
  values: Record<string, Record<string, string>> = {},
): string {
  const value = (reference: GraphReference) =>
    values[reference.node]?.[reference.output] ?? `‹${reference.node}.${reference.output}›`
  if (mapping.type === "output") {
    const known = values[mapping.reference.node]?.[mapping.reference.output]
    return known ?? mapping.default ?? value(mapping.reference)
  }
  if (mapping.type === "variable") {
    const step = stepReference(mapping.reference)
    // Variable values are previewed by the variable picker, never here.
    return step ? value(step) : `‹${mapping.reference.trim()}›`
  }
  return parseTemplate(mapping.template)
    .map((s) => {
      switch (s.type) {
        case "text":
          return s.text
        case "output":
          return value(s.reference)
        case "variable":
          return `‹${s.reference}›`
      }
    })
    .join("")
}
