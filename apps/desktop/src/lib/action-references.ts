import type { FocusTarget } from "./focus"
import type { ProfileView, Usage, Workflow } from "./types"

/** Whether two usages name the same place. */
export function sameUsage(a: Usage, b: Usage): boolean {
  return usageKey(a) === usageKey(b)
}

/** A stable identity of the place a usage names. */
export function usageKey(usage: Usage): string {
  switch (usage.type) {
    case "workflowStep":
      return `step:${usage.workflow}:${usage.position}`
    case "workflowNode":
      return `node:${usage.workflow}:${usage.node}`
    case "layoutBinding":
      return `key:${usage.layout}:${usage.page}:${JSON.stringify(usage.control)}`
  }
}

/**
 * Every place that uses an action: the usages the backend reports (workflow
 * steps, key bindings) and the nodes of workflow graphs that run it.
 */
export function actionReferences(view: ProfileView, actionId: string): Usage[] {
  const usages = [...(view.actionUsages[actionId] ?? [])]
  for (const workflow of view.profile.workflows) {
    for (const node of workflow.graph?.nodes ?? []) {
      if (node.type !== "action" || node.action !== actionId) continue
      const usage: Usage = {
        type: "workflowNode",
        workflow: workflow.id,
        workflowName: workflow.name,
        node: node.id,
        ...(node.label ? { nodeLabel: node.label } : {}),
      }
      if (!usages.some((u) => sameUsage(u, usage))) usages.push(usage)
    }
  }
  return usages
}

/** The workflow a usage is part of, if any. */
export function usageWorkflow(workflows: Workflow[], usage: Usage): Workflow | undefined {
  return usage.type === "layoutBinding" ? undefined : workflows.find((w) => w.id === usage.workflow)
}

/** The page that shows a usage and what it should open. */
export function usageTarget(usage: Usage): { page: "workflows" | "designer"; focus: FocusTarget } {
  return usage.type === "layoutBinding"
    ? { page: "designer", focus: { type: "layout", layout: usage.layout, page: usage.page } }
    : { page: "workflows", focus: { type: "workflow", workflow: usage.workflow } }
}
