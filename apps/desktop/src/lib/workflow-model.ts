// Pure helpers for the workflow editor.

import type { ActionParameters, Project, StepCondition, Workflow, WorkflowStep } from "./types"

export function newStep(action: string): WorkflowStep {
  return { action, onFailure: "stop", condition: { type: "always" }, overrides: {} }
}

/** Moves the step at `from` to position `to`. */
export function moveStep(workflow: Workflow, from: number, to: number): Workflow {
  if (from === to || from < 0 || from >= workflow.steps.length) return workflow
  const steps = [...workflow.steps]
  const [step] = steps.splice(from, 1)
  if (!step) return workflow
  steps.splice(Math.max(0, Math.min(to, steps.length)), 0, step)
  return { ...workflow, steps }
}

export function updateStep(
  workflow: Workflow,
  index: number,
  change: Partial<WorkflowStep>,
): Workflow {
  return {
    ...workflow,
    steps: workflow.steps.map((step, i) => (i === index ? { ...step, ...change } : step)),
  }
}

export function removeStep(workflow: Workflow, index: number): Workflow {
  return { ...workflow, steps: workflow.steps.filter((_, i) => i !== index) }
}

export function setOverride(
  overrides: ActionParameters | undefined,
  key: string,
  value: ActionParameters[string] | undefined,
): ActionParameters {
  const next = { ...overrides }
  if (value === undefined) {
    // eslint-disable-next-line @typescript-eslint/no-dynamic-delete
    delete next[key]
  } else {
    next[key] = value
  }
  return next
}

/** Conditions that depend on a previous step make no sense for the first one. */
export function conditionAllowed(condition: StepCondition, index: number): boolean {
  return (
    index > 0 || (condition.type !== "previousSucceeded" && condition.type !== "previousFailed")
  )
}

export function describeCondition(condition: StepCondition | undefined, projects: Project[]) {
  switch (condition?.type ?? "always") {
    case "previousSucceeded":
      return "if the previous step succeeded"
    case "previousFailed":
      return "if the previous step failed"
    case "activeProject": {
      const id = condition?.type === "activeProject" ? condition.project : ""
      return `while ${projects.find((p) => p.id === id)?.key ?? "a missing project"} is active`
    }
    default:
      return "always"
  }
}

/** Client-side checks mirroring the backend, for immediate feedback. */
export function validateWorkflow(workflow: Workflow, actionIds: Set<string>): string[] {
  const issues: string[] = []
  if (!workflow.name.trim()) issues.push("Enter a name.")
  workflow.steps.forEach((step, index) => {
    if (!actionIds.has(step.action)) issues.push(`Step ${index + 1}: choose an action.`)
    if (step.condition && !conditionAllowed(step.condition, index)) {
      issues.push(`Step ${index + 1}: there is no previous step.`)
    }
  })
  return issues
}

/**
 * Which workflow the editor shows. After a save, duplicate or delete the
 * editor asks for a workflow that may not be in the profile yet; until the
 * reloaded profile arrives it keeps showing what it showed before instead of
 * falling back to an empty new workflow.
 */
export interface WorkflowSelection {
  /** A workflow id, or a marker for a new, unsaved workflow. */
  id: string
  /** While waiting for a reload: the profile data at the time and what was shown. */
  waiting?: { data: unknown; shown: string }
}

export function shownWorkflow(
  selection: WorkflowSelection,
  workflows: Workflow[],
  data: unknown,
  drafts: string[],
): string {
  const { id, waiting } = selection
  if (drafts.includes(id) || workflows.some((w) => w.id === id)) return id
  // The requested workflow has not arrived yet.
  if (waiting && waiting.data === data) return waiting.shown
  // Reloaded without it (for example deleted elsewhere): the first one, or a new one.
  return workflows[0]?.id ?? drafts[0] ?? id
}
