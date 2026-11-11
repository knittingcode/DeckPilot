/**
 * Structured data mappings. The engine's format stays the same
 * (`{{ steps.<node>.output.<key> }}` inside text, or one output reference);
 * the editor works on its parts. Nothing is evaluated here: the engine
 * resolves mappings when a node runs.
 */
import type {
  GraphMapping,
  GraphReference,
  ParameterKind,
  ValueKind,
  WorkflowGraph,
} from "@/lib/types"

import { type Lookup, actionOf } from "./describe"
import { upstream } from "./graph"

export type Segment = { type: "text"; text: string } | { type: "output"; reference: GraphReference }

const REFERENCE = /\{\{\s*steps\.([a-z][a-z0-9_]*)\.output\.([A-Za-z0-9_.-]+)\s*\}\}/g

/** The parts of a template: plain text and output references. */
export function parseTemplate(template: string): Segment[] {
  const segments: Segment[] = []
  let last = 0
  for (const match of template.matchAll(REFERENCE)) {
    const index = match.index
    if (index > last) segments.push({ type: "text", text: template.slice(last, index) })
    segments.push({
      type: "output",
      reference: { node: match[1] ?? "", output: match[2] ?? "" },
    })
    last = index + match[0].length
  }
  if (last < template.length) segments.push({ type: "text", text: template.slice(last) })
  return segments
}

export function serializeTemplate(segments: Segment[]): string {
  return segments
    .map((s) =>
      s.type === "text" ? s.text : `{{ steps.${s.reference.node}.output.${s.reference.output} }}`,
    )
    .join("")
}

/** References a mapping uses. */
export function mappingReferences(mapping: GraphMapping): GraphReference[] {
  if (mapping.type === "output") return [mapping.reference]
  return parseTemplate(mapping.template).flatMap((s) => (s.type === "output" ? [s.reference] : []))
}

export interface OutputOption {
  reference: GraphReference
  /** "node label · output label" */
  label: string
  kind: ValueKind
}

/** Declared outputs of the nodes that run before `id`. */
export function outputOptions(graph: WorkflowGraph, id: string, lookup: Lookup): OutputOption[] {
  return upstream(graph, id).flatMap((nodeId) => {
    const node = graph.nodes.find((n) => n.id === nodeId)
    if (!node) return []
    const { action, type } = actionOf(node, lookup)
    return (type?.node?.outputs ?? []).map((output) => ({
      reference: { node: nodeId, output: output.key },
      label: `${node.label || action?.name || nodeId} · ${output.label}`,
      kind: output.kind,
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
  const node = graph.nodes.find((n) => n.id === reference.node)
  if (!node) return `there is no node “${reference.node}”`
  if (!upstream(graph, id).includes(reference.node)) {
    return `“${reference.node}” does not run before this node`
  }
  const { type } = actionOf(node, lookup)
  if (!type?.node?.outputs.some((o) => o.key === reference.output)) {
    return `“${reference.node}” has no output “${reference.output}”`
  }
  return null
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
  return parseTemplate(mapping.template)
    .map((s) => (s.type === "text" ? s.text : value(s.reference)))
    .join("")
}
