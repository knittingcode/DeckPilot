import { EXECUTION_STATE_LABEL, type LiveExecution, executionDuration } from "@/lib/executions"
import type { ExecutionSummary } from "@/lib/types"

import { formatMs } from "./describe"
import type { NodeRun } from "./node-view"

/** The run state of every node of `run`, for the canvas. */
export function nodeRuns(run: LiveExecution | undefined): Record<string, NodeRun> | undefined {
  if (!run) return undefined
  const result: Record<string, NodeRun> = {}
  for (const report of run.nodes) result[report.node] = { state: "done", report }
  for (const node of run.runningNodes ?? []) {
    result[node] = { state: "running", attempt: run.attempts?.[node] }
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
