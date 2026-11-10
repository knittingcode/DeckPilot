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
