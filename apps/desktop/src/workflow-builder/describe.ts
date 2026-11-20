import type { TFunction } from "i18next"

import { currentLanguage, i18n } from "@/i18n"
import { formatNumber } from "@/i18n/format"
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

/**
 * Translates with the window's instance by default; components pass the `t`
 * of `useTranslation` so memoized texts follow a language change.
 */
function defaultT(): TFunction {
  return i18n.getFixedT(currentLanguage())
}

function predicateText(predicate: GraphPredicate, t: TFunction) {
  switch (predicate.type) {
    case "equals":
      return t("workflows:node.predicate.equals", {
        output: `${predicate.reference.node}.${predicate.reference.output}`,
        value: predicate.value,
      })
    case "notEmpty":
      return t("workflows:node.predicate.notEmpty", {
        output: `${predicate.reference.node}.${predicate.reference.output}`,
      })
    case "succeeded":
      return t("workflows:node.predicate.succeeded", { node: predicate.node })
    case "activeProject":
      return t("workflows:node.predicate.activeProject", { key: predicate.key || "…" })
  }
}

/** Title and one-line summary of a node. */
export function describeNode(
  node: GraphNode,
  lookup: Lookup,
  t: TFunction = defaultT(),
): { title: string; detail: string } {
  const label = node.label?.trim()
  switch (node.type) {
    case "action": {
      const { action, type } = actionOf(node, lookup)
      return {
        title: label || action?.name || t("workflows:node.missingAction"),
        detail: type?.name ?? (action ? action.actionType : t("workflows:node.actionGone")),
      }
    }
    case "condition":
      return {
        title: label || t(FLOW_TITLE.condition),
        detail: predicateText(node.predicate, t),
      }
    case "loop": {
      const workflow = lookup.workflows.find((w) => w.id === node.workflow)
      return {
        title: label || t(FLOW_TITLE.loop),
        detail: t("workflows:node.loopDetail", {
          workflow: workflow?.name ?? t("workflows:node.missingWorkflow"),
          times: node.times,
        }),
      }
    }
    case "call": {
      const workflow = lookup.workflows.find((w) => w.id === node.workflow)
      return {
        title: label || workflow?.name || t(FLOW_TITLE.call),
        detail: workflow ? t(FLOW_TITLE.call) : t("workflows:node.missingWorkflow"),
      }
    }
    case "output": {
      const keys = Object.keys(node.values ?? {})
      return {
        title: label || t(FLOW_TITLE.output),
        detail: keys.length ? keys.join(", ") : t("workflows:node.noOutputs"),
      }
    }
    case "setVariable": {
      const keys = Object.keys(node.values ?? {})
      return {
        title: label || t(FLOW_TITLE.setVariable),
        detail: keys.length ? keys.join(", ") : t("workflows:node.noVariables"),
      }
    }
    case "trigger":
      return {
        title: label || t(FLOW_TITLE.trigger),
        detail: triggersSummary(lookup.triggers, lookup.triggersPaused),
      }
    case "delay":
      return { title: label || t(FLOW_TITLE.delay), detail: formatMs(node.ms, t) }
    default:
      return { title: label || t(FLOW_TITLE[node.type]), detail: "" }
  }
}

/** A duration for people: "250 ms", "1.5 s", "2 min 5 s" (numbers in the window's language). */
export function formatMs(ms: number, t: TFunction = defaultT()) {
  const number = (value: number) =>
    formatNumber(currentLanguage(), value, { maximumFractionDigits: 1, useGrouping: false })
  if (ms < 1000) return t("workflows:node.duration.ms", { value: number(ms) })
  const seconds = ms / 1000
  if (seconds < 60) return t("workflows:node.duration.seconds", { value: number(seconds) })
  const minutes = Math.floor(seconds / 60)
  const rest = Math.round(seconds % 60)
  return rest
    ? t("workflows:node.duration.minutesSeconds", {
        minutes: number(minutes),
        seconds: number(rest),
      })
    : t("workflows:node.duration.minutes", { minutes: number(minutes) })
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
      label: i18n.t("workflows:node.variableOutput", { label: variable.label || variable.key }),
    })),
  ]
}
