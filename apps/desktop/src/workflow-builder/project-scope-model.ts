import type { ActionInstance, ActionType, ProjectScope, Workflow, WorkflowGraph } from "@/lib/types"

/** Whether any action node selects a project (declared by its action type). */
export function selectsProjects(
  graph: WorkflowGraph,
  actions: ActionInstance[],
  types: ActionType[],
): boolean {
  return graph.nodes.some((node) => {
    if (node.type !== "action") return false
    const action = actions.find((a) => a.id === node.action)
    const type = action && types.find((t) => t.id === action.actionType)
    return Boolean(type?.node?.changesContext)
  })
}

/**
 * The scope that applies, as the engine decides it
 * (`Workflow::effective_project_scope`): a workflow saved before the setting
 * existed keeps changing the active project when it selects projects.
 */
export function effectiveProjectScope(
  workflow: Workflow,
  graph: WorkflowGraph,
  actions: ActionInstance[],
  types: ActionType[],
): ProjectScope {
  if (workflow.projectScope) return workflow.projectScope
  const steps = workflow.steps.some((step) => {
    const action = actions.find((a) => a.id === step.action)
    const type = action && types.find((t) => t.id === action.actionType)
    return Boolean(type?.node?.changesContext)
  })
  return selectsProjects(graph, actions, types) || steps ? "application" : "execution"
}
