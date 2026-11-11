import type {
  ActionInstance,
  ActionType,
  GraphNode,
  GraphPredicate,
  ModuleInfo,
  ValueKind,
  Workflow,
  WorkflowGraph,
  WorkflowInput,
  WorkflowOutput,
  WorkflowTrigger,
  WorkflowVariable,
} from "@/lib/types"

import { FLOW_TITLE } from "./catalog"
import { VARIABLES_NODE, upstream } from "./graph"
import { triggersSummary } from "./trigger-model"

export interface Lookup {
  actions: ActionInstance[]
  types: ActionType[]
  workflows: Workflow[]
  projects: { key: string; name: string }[]
  /** The module registry, for node documentation. */
  modules?: ModuleInfo[]
  /** The open workflow's declared inputs: outputs of its start node. */
  inputs?: WorkflowInput[]
  /** The open workflow's declared outputs, set by its Output nodes. */
  outputs?: WorkflowOutput[]
  /** The open workflow's variables, readable as `steps.vars.output.<key>`. */
  variables?: WorkflowVariable[]
  /** The open workflow's triggers, shown on its start node. */
  triggers?: WorkflowTrigger[]
  triggersPaused?: boolean
}

/** An output a node provides to later nodes. */
export interface NodeOutput {
  key: string
  label: string
  kind: ValueKind
  sensitive?: boolean
}

/**
 * What a node provides to later nodes, mirroring the engine's validation:
 * an action its type's declared outputs, the start node the workflow's
 * inputs, a sub-workflow the called workflow's declared outputs.
 */
export function nodeOutputs(node: GraphNode, lookup: Lookup): NodeOutput[] {
  const named = (values: { key: string; label?: string; kind: ValueKind; sensitive?: boolean }[]) =>
    values.map((v) => ({ ...v, label: v.label || v.key }))
  switch (node.type) {
    case "action":
      return actionOf(node, lookup).type?.node?.outputs ?? []
    case "trigger":
      return named(lookup.inputs ?? [])
    case "call":
      return named(lookup.workflows.find((w) => w.id === node.workflow)?.outputs ?? [])
    default:
      return []
  }
}

export function actionOf(node: GraphNode, lookup: Lookup) {
  if (node.type !== "action") return { action: undefined, type: undefined }
  const action = lookup.actions.find((a) => a.id === node.action)
  const type = action ? lookup.types.find((t) => t.id === action.actionType) : undefined
  return { action, type }
}

function predicateText(predicate: GraphPredicate) {
  switch (predicate.type) {
    case "equals":
      return `${predicate.reference.node}.${predicate.reference.output} = “${predicate.value}”`
    case "notEmpty":
      return `${predicate.reference.node}.${predicate.reference.output} is set`
    case "succeeded":
      return `${predicate.node} succeeded`
    case "activeProject":
      return `project is ${predicate.key || "…"}`
  }
}

/** Title and one-line summary of a node. */
export function describeNode(node: GraphNode, lookup: Lookup): { title: string; detail: string } {
  const label = node.label?.trim()
  switch (node.type) {
    case "action": {
      const { action, type } = actionOf(node, lookup)
      return {
        title: label || action?.name || "Missing action",
        detail: type?.name ?? (action ? action.actionType : "The action no longer exists"),
      }
    }
    case "condition":
      return { title: label || "Condition", detail: predicateText(node.predicate) }
    case "loop": {
      const workflow = lookup.workflows.find((w) => w.id === node.workflow)
      return {
        title: label || "Loop",
        detail: `${workflow?.name ?? "Missing workflow"} × ${node.times}`,
      }
    }
    case "call": {
      const workflow = lookup.workflows.find((w) => w.id === node.workflow)
      return {
        title: label || workflow?.name || "Sub-workflow",
        detail: workflow ? "Sub-workflow" : "Missing workflow",
      }
    }
    case "output": {
      const keys = Object.keys(node.values ?? {})
      return {
        title: label || "Outputs",
        detail: keys.length ? keys.join(", ") : "Sets no outputs",
      }
    }
    case "setVariable": {
      const keys = Object.keys(node.values ?? {})
      return {
        title: label || "Set variable",
        detail: keys.length ? keys.join(", ") : "Sets no variables",
      }
    }
    case "trigger":
      return {
        title: label || FLOW_TITLE.trigger,
        detail: triggersSummary(lookup.triggers, lookup.triggersPaused),
      }
    case "delay":
      return { title: label || "Delay", detail: formatMs(node.ms) }
    default:
      return { title: label || FLOW_TITLE[node.type], detail: "" }
  }
}

export function formatMs(ms: number) {
  if (ms < 1000) return `${ms} ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${Number.isInteger(seconds) ? seconds : seconds.toFixed(1)} s`
  const minutes = Math.floor(seconds / 60)
  const rest = Math.round(seconds % 60)
  return rest ? `${minutes} min ${rest} s` : `${minutes} min`
}

/** Outputs of the nodes that run before `id`, for mappings and conditions. */
export function availableOutputs(graph: WorkflowGraph, id: string, lookup: Lookup) {
  return [
    ...upstream(graph, id).flatMap((nodeId) => {
      const node = graph.nodes.find((n) => n.id === nodeId)
      if (!node) return []
      return nodeOutputs(node, lookup).map((output) => ({
        reference: { node: nodeId, output: output.key },
        label: `${node.label || nodeId} · ${output.label}`,
      }))
    }),
    ...(lookup.variables ?? []).map((variable) => ({
      reference: { node: VARIABLES_NODE, output: variable.key },
      label: `Variable · ${variable.label || variable.key}`,
    })),
  ]
}
