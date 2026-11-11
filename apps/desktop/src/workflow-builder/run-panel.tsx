import {
  CircleAlert,
  CircleCheck,
  CircleSlash,
  Cpu,
  Loader2,
  MousePointerClick,
  PlugZap,
  Square,
  Trash2,
} from "lucide-react"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Button } from "@/components/ui/button"
import { formatTimestamp } from "@/lib/format"
import { type LiveExecution, executionProgress } from "@/lib/executions"
import type { ExecutionState } from "@/lib/types"
import { cn } from "@/lib/utils"

import { formatMs } from "./describe"
import type { NodeRun } from "./node-view"
import { projectOutcomeText, runSummary } from "./run-model"

export function StateIcon({ state, className }: { state: ExecutionState; className?: string }) {
  const icon = {
    running: <Loader2 className={cn("animate-spin text-primary", className)} />,
    succeeded: <CircleCheck className={cn("text-emerald-600 dark:text-emerald-400", className)} />,
    failed: <CircleAlert className={cn("text-destructive", className)} />,
    cancelled: <CircleSlash className={cn("text-muted-foreground", className)} />,
    interrupted: <PlugZap className={cn("text-amber-600 dark:text-amber-400", className)} />,
  }[state]
  return icon
}

/**
 * Runs of the open workflow, newest first. The selected run is shown on the
 * canvas; a new run is followed automatically.
 */
export function RunHistory({
  runs,
  shown,
  now,
  onShow,
  onCancel,
  onClear,
  projectKey,
}: {
  runs: LiveExecution[]
  shown: string | null
  now: number
  onShow: (id: string | null) => void
  onCancel: (id: string) => void
  onClear: () => void
  /** A project's key by its id. */
  projectKey: (id: string | null | undefined) => string
}) {
  return (
    <section className="grid gap-2 border-t px-4 py-3" aria-label="Runs">
      <div className="flex items-center gap-2">
        <h3 className="flex-1 text-xs font-medium text-muted-foreground uppercase">Runs</h3>
        {shown && (
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-1.5 text-xs"
            onClick={() => {
              onShow(null)
            }}
          >
            Hide on canvas
          </Button>
        )}
        {runs.some((r) => r.state !== "running") && (
          <ConfirmDialog
            trigger={
              <Button variant="ghost" size="icon" className="size-6" aria-label="Clear history">
                <Trash2 className="size-3.5" />
              </Button>
            }
            title="Clear the run history of this workflow?"
            description="Finished runs are deleted. Running workflows are not affected."
            confirmLabel="Clear"
            onConfirm={onClear}
          />
        )}
      </div>
      {runs.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No runs yet. Runs started here, from a Stream Deck or while the app was closed appear
          here.
        </p>
      )}
      <ul className="grid gap-1">
        {runs.map((run) => (
          <li
            key={run.id}
            className={cn(
              "grid gap-1 rounded-md border text-xs",
              shown === run.id && "border-primary bg-accent",
            )}
          >
            <button
              type="button"
              aria-pressed={shown === run.id}
              onClick={() => {
                onShow(run.id)
              }}
              className="grid w-full gap-1 rounded-md px-2.5 py-2 text-left hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            >
              <span className="flex items-center gap-1.5">
                <StateIcon state={run.state} className="size-3.5" />
                <span className="flex-1 font-medium">{runSummary(run, now)}</span>
                {run.trigger === "device" ? (
                  <Cpu
                    className="size-3.5 text-muted-foreground"
                    aria-label="Started from a Stream Deck"
                  />
                ) : (
                  <MousePointerClick
                    className="size-3.5 text-muted-foreground"
                    aria-label="Started in the app"
                  />
                )}
              </span>
              <span className="text-muted-foreground">
                {formatTimestamp(Math.floor(run.startedAt / 1000))}
              </span>
              {projectOutcomeText(run.project, projectKey) && (
                <span className="text-muted-foreground">
                  {projectOutcomeText(run.project, projectKey)}
                </span>
              )}
              {run.message && (run.state === "failed" || run.state === "interrupted") && (
                <span className="line-clamp-2 text-muted-foreground">{run.message}</span>
              )}
            </button>
            {run.state === "running" && (
              <div className="flex items-center gap-2 px-2.5 pb-2">
                <span
                  className="h-1 flex-1 overflow-hidden rounded-full bg-muted"
                  role="progressbar"
                  aria-valuenow={executionProgress(run)}
                  aria-label="Progress"
                >
                  <span
                    className="block h-full bg-primary transition-[width]"
                    style={{ width: `${executionProgress(run)}%` }}
                  />
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 px-1.5 text-xs"
                  onClick={() => {
                    onCancel(run.id)
                  }}
                >
                  <Square className="size-3" /> Cancel
                </Button>
              </div>
            )}
          </li>
        ))}
      </ul>
    </section>
  )
}

/** What a node did in the shown run: state, timing, message, error, outputs. */
export function NodeRunReport({ run }: { run: NodeRun }) {
  if (run.state === "running") {
    return (
      <section className="grid gap-2 border-t px-4 py-3" aria-label="Run">
        <h3 className="text-xs font-medium text-muted-foreground uppercase">Run</h3>
        <p className="flex items-center gap-1.5 text-xs">
          <Loader2 className="size-3.5 animate-spin text-primary" />
          {run.attempt && run.attempt > 1 ? `Running, attempt ${run.attempt}` : "Running"}
          {run.progress?.message ? ` · ${run.progress.message}` : ""}
        </p>
      </section>
    )
  }
  const { report } = run
  const outputs = Object.entries(report.output ?? {})
  return (
    <section className="grid gap-2 border-t px-4 py-3" aria-label="Run">
      <h3 className="text-xs font-medium text-muted-foreground uppercase">Run</h3>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted-foreground">Result</dt>
        <dd className="capitalize">{report.status}</dd>
        {report.attempts > 0 && (
          <>
            <dt className="text-muted-foreground">Attempts</dt>
            <dd>{report.attempts}</dd>
          </>
        )}
        <dt className="text-muted-foreground">Started</dt>
        <dd>after {formatMs(report.startedMs)}</dd>
        <dt className="text-muted-foreground">Duration</dt>
        <dd>{formatMs(report.durationMs)}</dd>
      </dl>
      {report.message && (
        <p className="rounded bg-muted px-2 py-1.5 text-xs break-words">{report.message}</p>
      )}
      {report.error && (
        <div className="grid gap-0.5 rounded border border-destructive/40 bg-destructive/5 px-2 py-1.5 text-xs">
          <span className="font-mono text-destructive">{report.error.code}</span>
          <span className="break-words">{report.error.message}</span>
          {report.error.retryable && (
            <span className="text-muted-foreground">Running it again may help.</span>
          )}
        </div>
      )}
      {outputs.length > 0 && (
        <div className="grid gap-1">
          <h4 className="text-xs font-medium">Outputs</h4>
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
            {outputs.map(([key, value]) => (
              <div key={key} className="contents">
                <dt className="font-mono text-muted-foreground">{key}</dt>
                <dd className="font-mono break-all">{value}</dd>
              </div>
            ))}
          </dl>
          <p className="text-[0.6875rem] text-muted-foreground">
            Only outputs the action declares are recorded.
          </p>
        </div>
      )}
    </section>
  )
}
