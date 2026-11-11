import { Lock } from "lucide-react"

import type { LiveExecution } from "@/lib/executions"
import { cn } from "@/lib/utils"

import { formatMs } from "./describe"
import {
  type DeclaredOutput,
  STATUS_LABEL,
  type TimelineRow,
  describeFailure,
  outputPreviews,
  retryHistory,
} from "./run-debug"

function Retries({ run, node }: { run: LiveExecution; node: string }) {
  const history = retryHistory(run, node)
  if (history.attempts <= 1 && !history.live) return null
  return (
    <div className="grid gap-1" aria-label="Retries" role="group">
      <h5 className="text-xs font-medium">Retries</h5>
      <p className="text-xs">
        {history.attempts} attempts
        {history.live && ` · waited ${formatMs(history.totalWaitMs)} in total`}
      </p>
      {history.live ? (
        <ol className="grid gap-0.5 text-xs text-muted-foreground">
          {history.waits.map((wait) => (
            <li key={wait.attempt}>
              Attempt {wait.attempt} after waiting {formatMs(wait.waitMs)}
            </li>
          ))}
        </ol>
      ) : (
        <p className="text-[0.6875rem] text-muted-foreground">
          Run history keeps only the final attempt; earlier attempts and their waits are shown only
          while a run is watched live.
        </p>
      )}
    </div>
  )
}

function Outputs({
  run,
  node,
  declared,
}: {
  run: LiveExecution
  node: string
  declared: DeclaredOutput[]
}) {
  const report = run.nodes.find((n) => n.node === node)
  const previews = outputPreviews(report, declared)
  if (previews.length === 0) {
    return <p className="text-xs text-muted-foreground">The action declares no outputs.</p>
  }
  return (
    <div className="grid gap-1">
      <h5 className="text-xs font-medium">Outputs</h5>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        {previews.map((preview) => (
          <div key={preview.key} className="contents">
            <dt className="font-mono text-muted-foreground" title={preview.label}>
              {preview.key}
            </dt>
            <dd
              className={cn(
                "break-all",
                preview.state === "value" ? "font-mono" : "text-muted-foreground",
              )}
            >
              {preview.state === "value" && (
                <span title={preview.truncated ? "Shortened preview" : undefined}>
                  {preview.value}
                </span>
              )}
              {preview.state === "sensitive" && (
                <span className="inline-flex items-center gap-1">
                  <Lock className="size-3" aria-hidden /> not kept (sensitive)
                </span>
              )}
              {preview.state === "missing" && "not produced"}
            </dd>
          </div>
        ))}
      </dl>
      <p className="text-[0.6875rem] text-muted-foreground">
        Only declared outputs are shown; sensitive outputs reach later nodes but are never kept.
      </p>
    </div>
  )
}

/** Everything known about one node of the shown run. */
export function RunNodeDetails({
  run,
  row,
  declared,
}: {
  run: LiveExecution
  row: TimelineRow
  /** Outputs the node's action declares; `undefined` for flow nodes. */
  declared: DeclaredOutput[] | undefined
}) {
  const report = run.nodes.find((n) => n.node === row.node)
  const error = report?.error
  const failure = error ? describeFailure(error) : undefined
  return (
    <section className="grid gap-2 rounded-md border p-2" aria-label={`${row.title} details`}>
      <h4 className="text-xs font-medium">{row.title}</h4>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
        <dt className="text-muted-foreground">Result</dt>
        <dd className="capitalize">{STATUS_LABEL[row.status]}</dd>
        <dt className="text-muted-foreground">Started</dt>
        <dd>
          {row.estimated ? "about " : ""}
          {formatMs(row.startMs)} into the run
        </dd>
        <dt className="text-muted-foreground">Duration</dt>
        <dd>{formatMs(row.durationMs)}</dd>
      </dl>
      {report?.message && (
        <p className="rounded bg-muted px-2 py-1.5 text-xs break-words">{report.message}</p>
      )}
      {error && failure && (
        <div
          className={cn(
            "grid gap-0.5 rounded border px-2 py-1.5 text-xs",
            error.code === "cancelled" ? "bg-muted" : "border-destructive/40 bg-destructive/5",
          )}
          role="group"
          aria-label="Failure"
        >
          <span className="font-mono">
            {error.code}
            {error.actionCode && ` · ${error.actionCode}`}
          </span>
          <span className="break-words">{error.message}</span>
          <span className="text-muted-foreground">{failure.explanation}</span>
          {error.code !== "cancelled" && (
            <span className="text-muted-foreground">{failure.hint}</span>
          )}
        </div>
      )}
      <Retries run={run} node={row.node} />
      {declared && row.status !== "running" && (
        <Outputs run={run} node={row.node} declared={declared} />
      )}
    </section>
  )
}
