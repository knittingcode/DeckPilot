import { Store } from "@/lib/status"
import type {
  AgentExecutionEvent,
  ExecutionRecord,
  ExecutionState,
  ExecutionSummary,
} from "@/lib/types"

/** A run as this window knows it; `attempts` counts retries seen live. */
export type LiveExecution = ExecutionRecord & { attempts?: Record<string, number> }

/**
 * Workflow runs as the agent reports them. Runs execute in the agent, so
 * this is only a view: it is filled from `listExecutions` when the window
 * opens or the agent (re)connects, and kept current by execution events.
 */
export class ExecutionStore extends Store<LiveExecution> {
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

  /** Adds runs from a history list, keeping what is known about them. */
  merge(summaries: ExecutionSummary[]) {
    for (const summary of summaries) {
      const current = this.get(summary.id)
      // A live run knows more than a list read a moment ago.
      if (current?.state === "running" && summary.state === "running") continue
      this.put(summary.id, { ...current, ...summary, nodes: current?.nodes ?? [] })
    }
  }

  /** Forgets finished runs, e.g. after the history was cleared. */
  forget(matches: (record: LiveExecution) => boolean) {
    let changed = false
    for (const [id, record] of this.values) {
      if (record.state !== "running" && matches(record)) {
        this.values.delete(id)
        changed = true
      }
    }
    if (changed) this.publish()
  }

  /**
   * Applies an agent event. Returns `false` for progress of a run this
   * store does not know yet; the caller then loads it.
   */
  apply(message: AgentExecutionEvent): boolean {
    switch (message.type) {
      case "executionStarted":
        // Progress may arrive before the answer that started the run.
        if (!this.get(message.execution.id)) {
          this.put(message.execution.id, { ...message.execution, nodes: [] })
        }
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
        let attempts = current.attempts
        if (event.type === "nodeStarted") running.add(event.node)
        if (event.type === "retrying") attempts = { ...attempts, [event.node]: event.attempt }
        if (event.type === "nodeFinished") {
          running.delete(event.report.node)
          nodes = [...nodes.filter((n) => n.node !== event.report.node), event.report]
        }
        this.put(current.id, {
          ...current,
          nodes,
          attempts,
          runningNodes: [...running],
          completedNodes: message.completed,
          totalNodes: message.total,
        })
        return true
      }
    }
  }

  /** Runs of a profile, newest first. */
  static ofProfile<T extends ExecutionSummary>(records: T[], profileId: string | null) {
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

/** Milliseconds a run took, or has been running. */
export function executionDuration(summary: ExecutionSummary, now: number) {
  return (summary.finishedAt ?? now) - summary.startedAt
}
