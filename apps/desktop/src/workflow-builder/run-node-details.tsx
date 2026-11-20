import { Lock } from "lucide-react"

import { useTranslation } from "@/i18n"
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
  const { t } = useTranslation()
  const history = retryHistory(run, node)
  if (history.attempts <= 1 && !history.live) return null
  return (
    <div className="grid gap-1" aria-label={t("workflows:run.retries.title")} role="group">
      <h5 className="text-xs font-medium">{t("workflows:run.retries.title")}</h5>
      <p className="text-xs">
        {t("workflows:run.attempts", { count: history.attempts })}
        {history.live &&
          ` · ${t("workflows:run.retries.waitedTotal", { time: formatMs(history.totalWaitMs) })}`}
      </p>
      {history.live ? (
        <ol className="grid gap-0.5 text-xs text-muted-foreground">
          {history.waits.map((wait) => (
            <li key={wait.attempt}>
              {t("workflows:run.retries.attempt", {
                attempt: wait.attempt,
                time: formatMs(wait.waitMs),
              })}
            </li>
          ))}
        </ol>
      ) : (
        <p className="text-[0.6875rem] text-muted-foreground">
          {t("workflows:run.retries.historyOnly")}
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
  const { t } = useTranslation()
  const report = run.nodes.find((n) => n.node === node)
  const previews = outputPreviews(report, declared)
  if (previews.length === 0) {
    return <p className="text-xs text-muted-foreground">{t("workflows:run.outputs.none")}</p>
  }
  return (
    <div className="grid gap-1">
      <h5 className="text-xs font-medium">{t("workflows:run.outputs.title")}</h5>
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
                <span title={preview.truncated ? t("workflows:run.outputs.shortened") : undefined}>
                  {preview.value}
                </span>
              )}
              {preview.state === "sensitive" && (
                <span className="inline-flex items-center gap-1">
                  <Lock className="size-3" aria-hidden /> {t("workflows:run.outputs.sensitive")}
                </span>
              )}
              {preview.state === "missing" && t("workflows:run.outputs.missing")}
            </dd>
          </div>
        ))}
      </dl>
      <p className="text-[0.6875rem] text-muted-foreground">
        {t("workflows:run.outputs.declaredOnly")}
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
  const { t } = useTranslation()
  const report = run.nodes.find((n) => n.node === row.node)
  const error = report?.error
  const failure = error ? describeFailure(error) : undefined
  return (
    <section
      className="grid gap-2 rounded-md border p-2"
      aria-label={t("workflows:run.node.details", { node: row.title })}
    >
      <h4 className="text-xs font-medium">{row.title}</h4>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
        <dt className="text-muted-foreground">{t("workflows:run.node.result")}</dt>
        <dd className="capitalize">{STATUS_LABEL[row.status]}</dd>
        <dt className="text-muted-foreground">{t("workflows:run.node.started")}</dt>
        <dd>
          {row.estimated
            ? t("workflows:run.node.startedAbout", { time: formatMs(row.startMs) })
            : t("workflows:run.node.startedAt", { time: formatMs(row.startMs) })}
        </dd>
        <dt className="text-muted-foreground">{t("workflows:run.node.duration")}</dt>
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
          aria-label={t("workflows:run.node.failure")}
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
