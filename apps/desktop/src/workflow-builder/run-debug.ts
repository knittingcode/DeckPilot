// Pure computations for debugging a workflow run: the execution timeline,
// duration metrics, retry history, failure reasons and output previews.
// Only data the app already receives is used: node reports of the run and
// the live events this window saw.

import { type LiveExecution, type RetryWait, executionDuration } from "@/lib/executions"
import type { NodeReport, NodeStatus, StepError } from "@/lib/types"

export type RowStatus = NodeStatus | "running"

export const STATUS_LABEL: Record<RowStatus, string> = {
  running: "running",
  succeeded: "succeeded",
  failed: "failed",
  skipped: "skipped",
  cancelled: "cancelled",
}

export interface TimelineRow {
  node: string
  title: string
  status: RowStatus
  /** Milliseconds after the start of the run. */
  startMs: number
  durationMs: number
  /** Row of the chart; nodes that overlap in time get different lanes. */
  lane: number
  /** Ran at the same time as another node (a parallel branch). */
  parallel: boolean
  /** The start was not reported; estimated from what finished before. */
  estimated: boolean
  attempts: number
}

export interface Timeline {
  /** Milliseconds the chart spans: the run, or the latest node end. */
  spanMs: number
  rows: TimelineRow[]
  lanes: number
}

const end = (row: Pick<TimelineRow, "startMs" | "durationMs">) => row.startMs + row.durationMs

function overlaps(a: TimelineRow, b: TimelineRow) {
  return a.durationMs > 0 && b.durationMs > 0 && a.startMs < end(b) && b.startMs < end(a)
}

/** Start and duration bars of every node of a run, finished and running. */
export function buildTimeline(
  run: LiveExecution,
  now: number,
  title: (node: string) => string,
): Timeline {
  const elapsed = Math.max(0, executionDuration(run, now))
  const rows: TimelineRow[] = run.nodes.map((report) => ({
    node: report.node,
    title: title(report.node),
    status: report.status,
    startMs: report.startedMs,
    durationMs: report.durationMs,
    lane: 0,
    parallel: false,
    estimated: false,
    attempts: report.attempts,
  }))
  const finishedBy = rows.reduce((latest, row) => Math.max(latest, end(row)), 0)
  for (const node of run.runningNodes ?? []) {
    if (rows.some((r) => r.node === node)) continue
    const seen = run.nodeStarts?.[node]
    const startMs = seen ?? Math.min(finishedBy, elapsed)
    rows.push({
      node,
      title: title(node),
      status: "running",
      startMs,
      durationMs: Math.max(0, elapsed - startMs),
      lane: 0,
      parallel: false,
      estimated: seen === undefined,
      attempts: run.attempts?.[node] ?? 1,
    })
  }
  rows.sort((a, b) => a.startMs - b.startMs || a.node.localeCompare(b.node))

  // Greedy interval partitioning: a node takes the first lane free at its start.
  const laneEnds: number[] = []
  for (const row of rows) {
    let lane = laneEnds.findIndex((e) => e <= row.startMs || row.durationMs === 0)
    if (lane < 0) lane = laneEnds.length
    row.lane = lane
    laneEnds[lane] = Math.max(laneEnds[lane] ?? 0, end(row))
  }
  for (const row of rows) row.parallel = rows.some((other) => other !== row && overlaps(row, other))

  const spanMs = Math.max(elapsed, ...rows.map(end), 1)
  return { spanMs, rows, lanes: Math.max(1, laneEnds.length) }
}

/** Position of a bar in percent of the chart. */
export function barGeometry(row: TimelineRow, spanMs: number) {
  const left = Math.min(100, (row.startMs / spanMs) * 100)
  const width = Math.max(0.75, Math.min(100 - left, (row.durationMs / spanMs) * 100))
  return { left, width }
}

export interface RunMetrics {
  totalMs: number
  /** Time spent in nodes, added up; above `totalMs` when branches ran in parallel. */
  nodeTimeMs: number
  /** Up to three nodes that took longest. */
  slowest: TimelineRow[]
  counts: Partial<Record<RowStatus, number>>
  retriedNodes: number
  /** Most nodes seen running at the same moment. */
  maxParallel: number
}

export function runMetrics(run: LiveExecution, timeline: Timeline, now: number): RunMetrics {
  const counts: Partial<Record<RowStatus, number>> = {}
  for (const row of timeline.rows) counts[row.status] = (counts[row.status] ?? 0) + 1
  const slowest = timeline.rows
    .filter((r) => r.durationMs > 0)
    .sort((a, b) => b.durationMs - a.durationMs || a.startMs - b.startMs)
    .slice(0, 3)
  // Sweep over start and end points to find the peak of concurrent nodes.
  const points = timeline.rows
    .filter((r) => r.durationMs > 0)
    .flatMap((r) => [
      { at: r.startMs, delta: 1 },
      { at: end(r), delta: -1 },
    ])
    .sort((a, b) => a.at - b.at || a.delta - b.delta)
  let current = 0
  let maxParallel = 0
  for (const p of points) {
    current += p.delta
    maxParallel = Math.max(maxParallel, current)
  }
  return {
    totalMs: Math.max(0, executionDuration(run, now)),
    nodeTimeMs: timeline.rows.reduce((sum, r) => sum + r.durationMs, 0),
    slowest,
    counts,
    retriedNodes: timeline.rows.filter(
      (r) => r.attempts > 1 || (run.retries?.[r.node]?.length ?? 0) > 0,
    ).length,
    maxParallel,
  }
}

export interface RetryHistory {
  /** Attempts made; for history records this is the final count only. */
  attempts: number
  /** Retry waits seen live, in order. */
  waits: RetryWait[]
  totalWaitMs: number
  /** Waits were seen live; otherwise only the final attempt is known. */
  live: boolean
}

export function retryHistory(run: LiveExecution, node: string): RetryHistory {
  const report = run.nodes.find((n) => n.node === node)
  const waits = run.retries?.[node] ?? []
  const lastLive = waits.at(-1)?.attempt ?? run.attempts?.[node] ?? 0
  return {
    attempts: Math.max(report?.attempts ?? 0, lastLive),
    waits,
    totalWaitMs: waits.reduce((sum, w) => sum + w.waitMs, 0),
    live: waits.length > 0,
  }
}

const ERROR_TEXT: Record<string, string> = {
  "action.failed": "The action reported a failure.",
  timeout: "The node took longer than its timeout.",
  cancelled: "The run was cancelled while the node ran.",
  mapping: "An input could not be resolved from earlier outputs.",
  "parallel-context": "A context-changing action cannot run in a parallel branch.",
  "loop.failed": "A run of the looped workflow failed.",
  unavailable: "The action or its module is not available.",
}

export interface Failure {
  node: string
  title: string
  error: StepError
  /** What the error code means. */
  explanation: string
  /** Whether running again may help, in words. */
  hint: string
}

export function describeFailure(error: StepError): { explanation: string; hint: string } {
  return {
    explanation: ERROR_TEXT[error.code] ?? "The node failed.",
    hint: error.retryable
      ? "Transient: running it again, or a retry policy, may help."
      : "Not transient: running it again will not help without a change.",
  }
}

/** Failed nodes in the order they started; cancellations are not failures. */
export function runFailures(run: LiveExecution, title: (node: string) => string): Failure[] {
  return run.nodes
    .filter(
      (n): n is NodeReport & { error: StepError } =>
        n.error !== undefined && n.status === "failed" && n.error.code !== "cancelled",
    )
    .sort((a, b) => a.startedMs - b.startedMs || a.node.localeCompare(b.node))
    .map((n) => ({
      node: n.node,
      title: title(n.node),
      error: n.error,
      ...describeFailure(n.error),
    }))
}

export interface DeclaredOutput {
  key: string
  label: string
  sensitive?: boolean
}

export type OutputPreview =
  | { key: string; label: string; state: "value"; value: string; truncated: boolean }
  | { key: string; label: string; state: "sensitive" }
  | { key: string; label: string; state: "missing" }

export const PREVIEW_LENGTH = 120

/**
 * Declared outputs of a node with their recorded values. Sensitive outputs
 * are never kept in records, and values that are not declared are not shown.
 */
export function outputPreviews(
  report: NodeReport | undefined,
  declared: DeclaredOutput[],
): OutputPreview[] {
  return declared.map(({ key, label, sensitive }): OutputPreview => {
    if (sensitive) return { key, label, state: "sensitive" }
    const value = report?.output?.[key]
    if (value === undefined) return { key, label, state: "missing" }
    const truncated = value.length > PREVIEW_LENGTH
    return {
      key,
      label,
      state: "value",
      value: truncated ? `${value.slice(0, PREVIEW_LENGTH)}…` : value,
      truncated,
    }
  })
}

export type DebugState =
  { type: "loading" } | { type: "waiting" } | { type: "empty" } | { type: "ready" }

/** Whether there is something to show for a run yet. */
export function debugState(run: LiveExecution): DebugState {
  const started = run.nodes.length > 0 || (run.runningNodes?.length ?? 0) > 0
  if (started) return { type: "ready" }
  if (run.state === "running") return { type: "waiting" }
  // Finished runs that were only listed load their node reports on demand.
  return run.totalNodes > 0 || run.completedNodes > 0 ? { type: "loading" } : { type: "empty" }
}
