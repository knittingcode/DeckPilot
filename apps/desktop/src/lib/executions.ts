import { Store } from "@/lib/status"
import type {
  AgentExecutionEvent,
  ExecutionRecord,
  ExecutionState,
  ExecutionSummary,
  NodeProgress,
} from "@/lib/types"

/** A retry seen live: the attempt about to start and the wait before it. */
export interface RetryWait {
  attempt: number
  waitMs: number
}

/** A run as this window knows it; `attempts` counts retries seen live. */
export type LiveExecution = ExecutionRecord & {
  attempts?: Record<string, number>
  /** The latest progress of running nodes. */
  progress?: Record<string, NodeProgress>
  /** Retries seen live per node; history records keep only the final attempt. */
  retries?: Record<string, RetryWait[]>
  /** When running nodes were seen to start, in ms after the run started. */
  nodeStarts?: Record<string, number>
}

/** What only this window saw live, kept when the agent's record replaces a run. */
function liveDetails(run: LiveExecution | undefined) {
  return run ? { retries: run.retries, nodeStarts: run.nodeStarts } : {}
}

/**
 * Workflow runs as the agent reports them. Runs execute in the agent, so
 * this is only a view: it is filled from `listExecutions` when the window
 * opens or the agent (re)connects, and kept current by execution events.
 */
export class ExecutionStore extends Store<LiveExecution> {
  /** The current time; replaceable in tests. */
  clock: () => number = () => Date.now()

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
    this.put(record.id, { ...record, ...liveDetails(this.get(record.id)) })
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
        this.put(message.execution.id, {
          ...message.execution,
          nodes: current?.nodes ?? [],
          ...liveDetails(current),
        })
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
        let retries = current.retries
        let nodeStarts = current.nodeStarts
        if (event.type === "nodeStarted") {
          running.add(event.node)
          nodeStarts = {
            ...nodeStarts,
            [event.node]: Math.max(0, this.clock() - current.startedAt),
          }
        }
        if (event.type === "retrying") {
          attempts = { ...attempts, [event.node]: event.attempt }
          const seen = retries?.[event.node] ?? []
          retries = {
            ...retries,
            [event.node]: [...seen, { attempt: event.attempt, waitMs: event.waitMs }],
          }
        }
        let progress = current.progress
        if (event.type === "nodeProgress") {
          progress = { ...progress, [event.node]: event.progress }
        }
        if (event.type === "nodeFinished") {
          running.delete(event.report.node)
          nodes = [...nodes.filter((n) => n.node !== event.report.node), event.report]
        }
        this.put(current.id, {
          ...current,
          nodes,
          attempts,
          retries,
          nodeStarts,
          progress,
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
