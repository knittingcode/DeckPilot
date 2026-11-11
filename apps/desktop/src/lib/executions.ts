import { Store } from "@/lib/status"
import type {
  AgentExecutionEvent,
  ExecutionRecord,
  ExecutionState,
  ExecutionSummary,
} from "@/lib/types"

/**
 * Workflow runs as the agent reports them. Runs execute in the agent, so
 * this is only a view: it is filled from `listExecutions` when the window
 * opens or the agent (re)connects, and kept current by execution events.
 */
export class ExecutionStore extends Store<ExecutionRecord> {
  /**
   * Replaces the known runs with the agent's list, keeping node details
   * already loaded for runs that are still listed.
   */
  reset(summaries: ExecutionSummary[]) {
    const previous = new Map(this.values)
    this.values.clear()
    for (const summary of summaries) {
      this.values.set(summary.id, { ...summary, nodes: previous.get(summary.id)?.nodes ?? [] })
    }
    this.publish()
  }

  /** A run loaded with its node reports. */
  load(record: ExecutionRecord) {
    this.put(record.id, record)
  }

  /**
   * Applies an agent event. Returns `false` for progress of a run this
   * store does not know yet; the caller then loads it.
   */
  apply(message: AgentExecutionEvent): boolean {
    switch (message.type) {
      case "executionStarted":
        this.put(message.execution.id, { ...message.execution, nodes: [] })
        return true
      case "executionFinished": {
        const current = this.get(message.execution.id)
        this.put(message.execution.id, { ...message.execution, nodes: current?.nodes ?? [] })
        return true
      }
      case "executionProgress": {
        const current = this.get(message.execution)
        if (!current) return false
        if (current.state !== "running") return true
        const running = new Set(current.runningNodes ?? [])
        let nodes = current.nodes
        const event = message.event
        if (event.type === "nodeStarted") running.add(event.node)
        if (event.type === "nodeFinished") {
          running.delete(event.report.node)
          nodes = [...nodes.filter((n) => n.node !== event.report.node), event.report]
        }
        this.put(current.id, {
          ...current,
          nodes,
          runningNodes: [...running],
          completedNodes: message.completed,
          totalNodes: message.total,
        })
        return true
      }
    }
  }

  /** Runs of a profile, newest first. */
  static ofProfile(records: ExecutionRecord[], profileId: string | null) {
    return records
      .filter((r) => r.profile === profileId)
      .sort((a, b) => b.startedAt - a.startedAt || a.id.localeCompare(b.id))
  }
}

export const EXECUTION_STATE_LABEL: Record<ExecutionState, string> = {
  running: "Running",
  succeeded: "Succeeded",
  failed: "Failed",
  cancelled: "Cancelled",
  interrupted: "Interrupted",
}

/** Percent of resolved nodes; real progress, not an estimate of time. */
export function executionProgress(summary: ExecutionSummary) {
  if (summary.totalNodes === 0) return summary.state === "running" ? 0 : 100
  return Math.round((summary.completedNodes / summary.totalNodes) * 100)
}
