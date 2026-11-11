import type {
  ActionInstance,
  ActionType,
  GraphNode,
  GraphPredicate,
  Workflow,
  WorkflowGraph,
} from "@/lib/types"

import { FLOW_TITLE } from "./catalog"
import { upstream } from "./graph"

export interface Lookup {
  actions: ActionInstance[]
  types: ActionType[]
  workflows: Workflow[]
  projects: { key: string; name: string }[]
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
  return upstream(graph, id).flatMap((nodeId) => {
    const node = graph.nodes.find((n) => n.id === nodeId)
    if (!node) return []
    const { type } = actionOf(node, lookup)
    return (type?.node?.outputs ?? []).map((output) => ({
      reference: { node: nodeId, output: output.key },
      label: `${node.label || nodeId} · ${output.label}`,
    }))
  })
}
