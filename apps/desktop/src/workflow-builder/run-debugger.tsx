import { Loader2 } from "lucide-react"
import { useState } from "react"

import type { LiveExecution } from "@/lib/executions"

import { formatMs } from "./describe"
import {
  type DeclaredOutput,
  buildTimeline,
  debugState,
  runFailures,
  runMetrics,
} from "./run-debug"
import { RunNodeDetails } from "./run-node-details"
import { RunTimeline } from "./run-timeline"

export interface NodeInfo {
  title: string
  /** Outputs the node's action declares; `undefined` for flow nodes. */
  declared?: DeclaredOutput[]
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="grid gap-1.5" aria-label={title}>
      <h4 className="text-[0.6875rem] font-medium text-muted-foreground">{title}</h4>
      {children}
    </section>
  )
}

/**
 * Debugging view of the shown run: metrics, failures, the execution
 * timeline and the details of a node chosen on it.
 */
export function RunDebugger({
  run,
  now,
  info,
}: {
  run: LiveExecution
  now: number
  /** Title and declared outputs of a node of the workflow. */
  info: (node: string) => NodeInfo
}) {
  const [chosen, setChosen] = useState<string | null>(null)
  const state = debugState(run)
  const title = (node: string) => info(node).title
  return (
    <section className="grid gap-3 border-t px-4 py-3" aria-label="Run details">
      <h3 className="text-xs font-medium text-muted-foreground uppercase">Run details</h3>
      {state.type === "loading" && (
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground" role="status">
          <Loader2 className="size-3.5 animate-spin" /> Loading node reports…
        </p>
      )}
      {state.type === "waiting" && (
        <p className="text-xs text-muted-foreground" role="status">
          Waiting for the first node to start.
        </p>
      )}
      {state.type === "empty" && (
        <p className="text-xs text-muted-foreground">This run did not run any nodes.</p>
      )}
      {state.type === "ready" && (
        <Report
          run={run}
          now={now}
          title={title}
          info={info}
          chosen={chosen}
          onChoose={setChosen}
        />
      )}
    </section>
  )
}

function Report({
  run,
  now,
  title,
  info,
  chosen,
  onChoose,
}: {
  run: LiveExecution
  now: number
  title: (node: string) => string
  info: (node: string) => NodeInfo
  chosen: string | null
  onChoose: (node: string) => void
}) {
  const timeline = buildTimeline(run, now, title)
  const metrics = runMetrics(run, timeline, now)
  const failures = runFailures(run, title)
  const selected =
    timeline.rows.find((r) => r.node === chosen) ??
    timeline.rows.find((r) => r.node === failures[0]?.node)
  return (
    <>
      <Section title="Metrics">
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
          <dt className="text-muted-foreground">{run.state === "running" ? "Elapsed" : "Total"}</dt>
          <dd>{formatMs(metrics.totalMs)}</dd>
          <dt className="text-muted-foreground">Node time</dt>
          <dd>
            {formatMs(metrics.nodeTimeMs)}
            {metrics.maxParallel > 1 && ` · up to ${metrics.maxParallel} in parallel`}
          </dd>
          {metrics.retriedNodes > 0 && (
            <>
              <dt className="text-muted-foreground">Retried</dt>
              <dd>
                {metrics.retriedNodes} {metrics.retriedNodes === 1 ? "node" : "nodes"}
              </dd>
            </>
          )}
        </dl>
        {metrics.slowest.length > 0 && (
          <ol className="grid gap-0.5 text-xs" aria-label="Slowest nodes">
            {metrics.slowest.map((row) => (
              <li key={row.node} className="flex gap-2">
                <button
                  type="button"
                  className="min-w-0 flex-1 truncate text-left hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  onClick={() => {
                    onChoose(row.node)
                  }}
                >
                  {row.title}
                </button>
                <span className="text-muted-foreground tabular-nums">
                  {formatMs(row.durationMs)}
                </span>
              </li>
            ))}
          </ol>
        )}
      </Section>

      {failures.length > 0 && (
        <Section title="Failures">
          <ul className="grid gap-1">
            {failures.map((failure) => (
              <li key={failure.node}>
                <button
                  type="button"
                  onClick={() => {
                    onChoose(failure.node)
                  }}
                  className="grid w-full gap-0.5 rounded border border-destructive/40 bg-destructive/5 px-2 py-1.5 text-left text-xs hover:bg-destructive/10 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  <span className="font-medium">{failure.title}</span>
                  <span className="font-mono text-destructive">
                    {failure.error.code}
                    {failure.error.actionCode && ` · ${failure.error.actionCode}`}
                  </span>
                  <span className="break-words">{failure.error.message}</span>
                  <span className="text-muted-foreground">{failure.hint}</span>
                </button>
              </li>
            ))}
          </ul>
        </Section>
      )}

      <Section title="Timeline">
        <RunTimeline timeline={timeline} selected={selected?.node ?? null} onSelect={onChoose} />
      </Section>

      {selected ? (
        <RunNodeDetails run={run} row={selected} declared={info(selected.node).declared} />
      ) : (
        <p className="text-xs text-muted-foreground">
          Choose a node on the timeline to see its retries, failure and outputs.
        </p>
      )}
    </>
  )
}
