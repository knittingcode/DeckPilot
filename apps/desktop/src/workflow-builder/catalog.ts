/**
 * The node catalog: built-in flow nodes and the nodes modules provide.
 *
 * Module nodes come from the module registry (`ModuleInfo.actionTypes`) and
 * the profile's actions of those types; the editor knows no module.
 */
import { creatableTypes } from "@/lib/modules"
import { pluginTypeUnavailable } from "@/lib/plugin-actions"
import type { ActionInstance, ActionType, GraphNodeKind, ModuleInfo } from "@/lib/types"

import { DEFAULT_POLICY, type NodeType } from "./graph"

export interface CatalogEntry {
  key: string
  title: string
  description: string
  /** Icon name of the action set, or a flow node type. */
  icon: string | null
  /** Name for the node id. */
  name: string
  kind: GraphNodeKind
  /** Why the entry cannot be placed now (a plugin action); absent when it can. */
  unavailable?: string
}

export interface CatalogGroup {
  id: string
  title: string
  entries: CatalogEntry[]
  /** Action types of the group that can get a new action (module groups). */
  types: ActionType[]
  /** Plugin action types that cannot get a new action now, with the reason. */
  unavailableTypes?: { type: ActionType; reason: string }[]
}

export const FLOW_TITLE: Record<NodeType, string> = {
  trigger: "Start",
  action: "Action",
  condition: "Condition",
  parallel: "Parallel",
  join: "Join",
  loop: "Loop",
  call: "Sub-workflow",
  output: "Outputs",
  setVariable: "Set variable",
  delay: "Delay",
  stop: "Stop",
}

export const FLOW_DESCRIPTION: Record<NodeType, string> = {
  trigger: "Where the workflow starts.",
  action: "Runs an action of this profile.",
  condition: "Continues with Yes or No.",
  parallel: "Runs the connected branches at the same time.",
  join: "Waits for every incoming branch, then continues.",
  loop: "Runs another workflow a fixed number of times.",
  call: "Runs another workflow once with inputs and uses its outputs.",
  output: "Sets this workflow's outputs for the node that calls it.",
  setVariable: "Assigns variables of this run for the nodes after it.",
  delay: "Waits before continuing.",
  stop: "Ends the workflow successfully.",
}

/** Built-in nodes. A loop or sub-workflow needs another workflow to run. */
export function flowEntries(otherWorkflow: string | undefined): CatalogEntry[] {
  const kinds: GraphNodeKind[] = [
    { type: "condition", predicate: { type: "succeeded", node: "start" } },
    { type: "parallel" },
    { type: "join" },
    ...(otherWorkflow
      ? [
          { type: "loop" as const, workflow: otherWorkflow, times: 2 },
          { type: "call" as const, workflow: otherWorkflow, policy: DEFAULT_POLICY },
        ]
      : []),
    { type: "output", values: {} },
    { type: "setVariable", values: {} },
    { type: "delay", ms: 1000 },
    { type: "stop" },
  ]
  return kinds.map((kind) => ({
    key: `flow:${kind.type}`,
    title: FLOW_TITLE[kind.type],
    description: FLOW_DESCRIPTION[kind.type],
    icon: kind.type,
    name: FLOW_TITLE[kind.type],
    kind,
  }))
}

export function actionEntry(action: ActionInstance, type: ActionType | undefined): CatalogEntry {
  return {
    key: `action:${action.id}`,
    title: action.name,
    description: type?.name ?? action.actionType,
    icon: action.icon ?? type?.node?.icon ?? null,
    name: action.name,
    kind: { type: "action", action: action.id, policy: DEFAULT_POLICY },
  }
}

/**
 * One group per enabled module that provides action types, and one per
 * installed plugin (builds with plugins only): its actions that cannot run
 * now are listed with the reason and cannot be placed.
 */
export function moduleGroups(modules: ModuleInfo[], actions: ActionInstance[]): CatalogGroup[] {
  return modules
    .filter((module) => (module.enabled || module.plugin) && module.actionTypes.length > 0)
    .map((module) => {
      const types = new Map(module.actionTypes.map((t) => [t.id, t]))
      const group: CatalogGroup = {
        id: module.id,
        title: module.name,
        entries: actions
          .filter((a) => types.has(a.actionType))
          .map((a) => {
            const entry = actionEntry(a, types.get(a.actionType))
            const unavailable = pluginTypeUnavailable(module, a.actionType)
            return unavailable ? { ...entry, unavailable } : entry
          }),
        // Saved actions of deprecated types stay listed above; new nodes
        // are made from the types that replace them.
        types: creatableTypes(module.actionTypes).filter(
          (t) => !pluginTypeUnavailable(module, t.id),
        ),
      }
      if (!module.plugin) return group
      const unavailableTypes = creatableTypes(module.actionTypes).flatMap((type) => {
        const reason = pluginTypeUnavailable(module, type.id)
        return reason ? [{ type, reason }] : []
      })
      return unavailableTypes.length > 0 ? { ...group, unavailableTypes } : group
    })
}

function matches(text: string, query: string) {
  return text.toLowerCase().includes(query)
}

/** Groups with the entries and types that match `query`. */
export function searchCatalog(groups: CatalogGroup[], query: string): CatalogGroup[] {
  const q = query.trim().toLowerCase()
  if (!q) return groups
  return groups
    .map((group) => {
      const groupMatches = matches(group.title, q)
      return {
        ...group,
        entries: group.entries.filter(
          (e) => groupMatches || matches(e.title, q) || matches(e.description, q),
        ),
        types: group.types.filter(
          (t) => groupMatches || matches(t.name, q) || matches(t.description, q),
        ),
        ...(group.unavailableTypes
          ? {
              unavailableTypes: group.unavailableTypes.filter(
                ({ type: t }) => groupMatches || matches(t.name, q) || matches(t.description, q),
              ),
            }
          : {}),
      }
    })
    .filter(
      (group) =>
        group.entries.length > 0 ||
        group.types.length > 0 ||
        (group.unavailableTypes?.length ?? 0) > 0,
    )
}

/** Drag-and-drop payload type for catalog entries. */
export const DRAG_TYPE = "application/x-deckpilot-node"
