import { EXECUTION_STATE_LABEL, type LiveExecution, executionDuration } from "@/lib/executions"
import type { ExecutionSummary, ProjectOutcome } from "@/lib/types"

import { formatMs } from "./describe"
import type { NodeRun } from "./node-view"

export { triggerText } from "./trigger-model"

/** The run state of every node of `run`, for the canvas. */
export function nodeRuns(run: LiveExecution | undefined): Record<string, NodeRun> | undefined {
  if (!run) return undefined
  const result: Record<string, NodeRun> = {}
  for (const report of run.nodes) result[report.node] = { state: "done", report }
  for (const node of run.runningNodes ?? []) {
    result[node] = {
      state: "running",
      attempt: run.attempts?.[node],
      progress: run.progress?.[node],
    }
  }
  return result
}

/** One line about a run: state, progress or duration. */
export function runSummary(run: ExecutionSummary, now: number) {
  if (run.state === "running") {
    return `Running · ${run.completedNodes} of ${run.totalNodes} nodes`
  }
  return `${EXECUTION_STATE_LABEL[run.state]} · ${formatMs(executionDuration(run, now))}`
}

/**
 * What a run did to DeckPilot's active project, in words; `null` when it
 * did nothing worth saying. `key` names a project by its id.
 */
export function projectOutcomeText(
  outcome: ProjectOutcome | undefined,
  key: (id: string | null | undefined) => string,
): string | null {
  switch (outcome?.type) {
    case "applied":
      return `Switched DeckPilot to ${key(outcome.project)}`
    case "keptInRun":
      return `Worked on ${key(outcome.project)} within the run`
    case "notApplied":
      return outcome.project
        ? `Did not switch to ${key(outcome.project)}: ${outcome.reason}`
        : `Did not switch the project: ${outcome.reason}`
    default:
      return null
  }
}
