// A Multi Action is a step workflow (a workflow without a graph) put on a
// key: the agent's workflow engine runs it, with its progress, cancellation
// and history. These helpers create one and its delay rows. A delay row is
// a step that uses a `general.delay` action with the step's own
// `milliseconds`, so the stored workflow is an ordinary step list.

import type { ActionInstance, ParameterValue, Workflow, WorkflowStep } from "./types"
import { newStep } from "./workflow-model"

/** The action type of a delay row. */
export const DELAY_TYPE = "general.delay"
export const DEFAULT_DELAY_MS = 500
export const MIN_DELAY_MS = 10
export const MAX_DELAY_MS = 300_000

/** A name no workflow of the profile has: "Multi Action", then "Multi Action 2", … */
export function freeWorkflowName(workflows: Pick<Workflow, "name">[], name: string): string {
  const taken = new Set(workflows.map((w) => w.name.trim().toLowerCase()))
  if (!taken.has(name.toLowerCase())) return name
  let n = 2
  while (taken.has(`${name} ${n}`.toLowerCase())) n += 1
  return `${name} ${n}`
}

/** A new, empty Multi Action; like every new workflow, its project changes stay in its runs. */
export function newMultiAction(workflows: Pick<Workflow, "name">[]): Workflow {
  return {
    id: crypto.randomUUID(),
    name: freeWorkflowName(workflows, "Multi Action"),
    description: "",
    steps: [],
    projectScope: "execution",
  }
}

/** Whether the workflow is edited as a step list (it has no graph). */
export function isStepWorkflow(workflow: Workflow | undefined): workflow is Workflow {
  return workflow !== undefined && !workflow.graph
}

/** The step's action if it is a delay. */
export function delayAction(
  step: WorkflowStep,
  actions: Pick<ActionInstance, "id" | "actionType" | "parameters">[],
): Pick<ActionInstance, "id" | "actionType" | "parameters"> | undefined {
  const action = actions.find((a) => a.id === step.action)
  return action?.actionType === DELAY_TYPE ? action : undefined
}

function integer(value: ParameterValue | undefined): number | undefined {
  return value?.type === "integer" ? value.value : undefined
}

/** How long a delay step waits: its own value, else its action's, else the default. */
export function delayMilliseconds(
  step: WorkflowStep,
  action: Pick<ActionInstance, "parameters"> | undefined,
): number {
  return (
    integer(step.overrides?.milliseconds) ??
    integer(action?.parameters.milliseconds) ??
    DEFAULT_DELAY_MS
  )
}

/** A delay row that waits `ms` with the delay action `action`. */
export function delayStep(action: string, ms: number = DEFAULT_DELAY_MS): WorkflowStep {
  return { ...newStep(action), overrides: { milliseconds: { type: "integer", value: ms } } }
}

/** The step with a new duration, kept inside the range the agent accepts. */
export function withDelay(step: WorkflowStep, ms: number): WorkflowStep {
  const value = Math.round(Math.min(MAX_DELAY_MS, Math.max(MIN_DELAY_MS, ms)))
  return { ...step, overrides: { ...step.overrides, milliseconds: { type: "integer", value } } }
}
