import { ChevronDown, ChevronRight, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import type { WorkSessionReview, WorkSessionSummary } from "@/lib/types"

import {
  RECORD_TEXT,
  REVIEW_KIND_TEXT,
  describeDuration,
  formatElapsed,
  noticeText,
  openReviews,
  outcomeKey,
} from "./model"

/** Sessions loaded at a time. */
export const HISTORY_PAGE = 20

/**
 * Past and current sessions with their intervals, transitions and flagged
 * gaps. A flagged gap is counted until the user excludes it (D8).
 */
export function HistoryCard({
  reloadKey,
  projectName,
}: {
  /** Changes when the agent's session changes, so the history follows it. */
  reloadKey: string
  projectName: (id: string) => string
}) {
  const { t } = useTranslation()
  const [limit, setLimit] = useState(HISTORY_PAGE)
  const history = useAsync(() => api.listWorkSessions(limit), `${reloadKey}:${limit}`)
  // Entries changed here since the last load (a gap excluded or kept).
  const [updated, setUpdated] = useState<Record<string, WorkSessionSummary>>({})
  const [loadedKey, setLoadedKey] = useState(reloadKey)
  if (loadedKey !== reloadKey) {
    setLoadedKey(reloadKey)
    setUpdated({})
  }

  return (
    <SettingsSection
      title={t("workspace:workSession.history.title")}
      description={t("workspace:workSession.history.description")}
    >
      <div className="grid gap-3">
        {history.state.status === "loading" ? (
          <Loading />
        ) : history.state.status === "error" ? (
          <ErrorNotice
            error={failureMessage(
              t("workspace:workSession.history.loadFailed"),
              history.state.error,
              t("workspace:workSession.checkAgent"),
            )}
          />
        ) : history.state.data.length === 0 ? (
          <EmptyState title={t("workspace:workSession.history.empty")}>
            {t("workspace:workSession.history.emptyHint")}
          </EmptyState>
        ) : (
          <div className="grid gap-3">
            <ul
              aria-label={t("workspace:workSession.history.sessions")}
              className="grid divide-y border-y"
            >
              {history.state.data.map((entry) => {
                const summary = updated[entry.session] ?? entry
                return (
                  <SessionRow
                    key={entry.session}
                    summary={summary}
                    projectName={projectName}
                    onChange={(next) => {
                      setUpdated((current) => ({ ...current, [next.session]: next }))
                    }}
                  />
                )
              })}
            </ul>
            {history.state.data.length >= limit && (
              <div>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setLimit(limit + HISTORY_PAGE)
                  }}
                >
                  {t("workspace:workSession.history.showOlder")}
                </Button>
              </div>
            )}
          </div>
        )}
      </div>
    </SettingsSection>
  )
}

function SessionRow({
  summary,
  projectName,
  onChange,
}: {
  summary: WorkSessionSummary
  projectName: (id: string) => string
  onChange: (summary: WorkSessionSummary) => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const time = (ms: number) => format.dateTime(ms)
  const [open, setOpen] = useState(false)
  // An open interval is shown up to when the list was drawn.
  const [now] = useState(() => Date.now())
  // Counted time as of the agent's answer (an active session's keeps growing).
  const elapsed = summary.elapsedMs
  const flagged = openReviews(summary.reviews).length
  const when = summary.startedAtMs ?? summary.requestedAtMs
  const detailsId = `session-${summary.session}`
  const project = summary.project ? projectName(summary.project) : null
  return (
    <li>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 py-2 text-sm">
        <Button
          variant="ghost"
          size="sm"
          className="-ml-2"
          aria-expanded={open}
          aria-controls={detailsId}
          onClick={() => {
            setOpen(!open)
          }}
        >
          {open ? <ChevronDown aria-hidden /> : <ChevronRight aria-hidden />}
          {time(when)}
        </Button>
        {project && <span className="text-muted-foreground">{project}</span>}
        <span
          className="font-mono tabular-nums"
          aria-label={t("workspace:workSession.history.duration", {
            duration: describeDuration(elapsed),
          })}
        >
          {summary.state === "aborted" ? "–" : formatElapsed(elapsed)}
        </span>
        <Badge variant={summary.state === "aborted" ? "destructive" : "secondary"}>
          {t(RECORD_TEXT[summary.state])}
        </Badge>
        {flagged > 0 && (
          <Badge variant="outline" className="border-amber-500/60 text-amber-700">
            <TriangleAlert aria-hidden />
            {t("workspace:workSession.history.gapsToReview", { count: flagged })}
          </Badge>
        )}
      </div>
      {open && (
        <div id={detailsId} className="grid gap-4 border-t px-3 py-3 text-sm">
          {summary.notice && (
            <section className="grid gap-1">
              <h4 className="font-medium">{t("workspace:workSession.history.notice")}</h4>
              <p>
                <span className="font-medium">{noticeText(summary.notice).title}</span>
                <span className="text-muted-foreground">
                  {" "}
                  — {noticeText(summary.notice).detail} ({time(summary.notice.atMs)})
                </span>
              </p>
            </section>
          )}
          {summary.reviews.length > 0 && (
            <section className="grid gap-2">
              <h4 className="font-medium">{t("workspace:workSession.history.flaggedGaps")}</h4>
              <ul
                aria-label={t("workspace:workSession.history.flaggedGaps")}
                className="grid gap-2"
              >
                {summary.reviews.map((review) => (
                  <GapRow
                    key={review.id}
                    session={summary.session}
                    review={review}
                    onChange={onChange}
                  />
                ))}
              </ul>
            </section>
          )}
          <section className="grid gap-1">
            <h4 className="font-medium">{t("workspace:workSession.history.intervals")}</h4>
            {summary.intervals.length === 0 ? (
              <p className="text-muted-foreground">
                {t("workspace:workSession.history.noIntervals")}
              </p>
            ) : (
              <ol aria-label={t("workspace:workSession.history.intervals")} className="grid gap-1">
                {summary.intervals.map((interval) => (
                  <li key={interval.seq} className="flex flex-wrap gap-x-3">
                    <span>
                      {interval.endedAtMs === undefined
                        ? t("workspace:workSession.history.intervalOpen", {
                            from: time(interval.startedAtMs),
                          })
                        : t("workspace:workSession.history.interval", {
                            from: time(interval.startedAtMs),
                            to: time(interval.endedAtMs),
                          })}
                    </span>
                    <span className="font-mono text-muted-foreground tabular-nums">
                      {formatElapsed(
                        Math.max(0, (interval.endedAtMs ?? now) - interval.startedAtMs),
                      )}
                    </span>
                    {interval.endReason === "gapExcluded" && (
                      <span className="text-muted-foreground">
                        {t("workspace:workSession.history.gapExcluded")}
                      </span>
                    )}
                  </li>
                ))}
              </ol>
            )}
          </section>
          {summary.verification && (
            <section className="grid gap-1">
              <h4 className="font-medium">{t("workspace:workSession.history.lastCheck")}</h4>
              <p className="text-muted-foreground">
                {t("workspace:workSession.check.withTime", {
                  result: summary.verification.outcome?.details
                    ? t("workspace:workSession.check.withDetails", {
                        outcome: t(outcomeKey(summary.verification.outcome)),
                        details: summary.verification.outcome.details,
                      })
                    : t(outcomeKey(summary.verification.outcome)),
                  time: time(summary.verification.atMs),
                })}
              </p>
            </section>
          )}
          {summary.transitions.length > 0 && (
            <section className="grid gap-1">
              <h4 className="font-medium">{t("workspace:workSession.history.transitions")}</h4>
              <ol
                aria-label={t("workspace:workSession.history.transitions")}
                className="grid gap-0.5 text-muted-foreground"
              >
                {summary.transitions.map((step) => (
                  <li key={step.seq}>
                    {time(step.atMs)}: {step.from} → {step.to} ({step.event})
                  </li>
                ))}
              </ol>
            </section>
          )}
        </div>
      )}
    </li>
  )
}

function GapRow({
  session,
  review,
  onChange,
}: {
  session: string
  review: WorkSessionReview
  onChange: (summary: WorkSessionSummary) => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const time = (ms: number) => format.dateTime(ms)
  const [busy, setBusy] = useState(false)
  const resolve = (count: boolean) => {
    setBusy(true)
    api.resolveWorkSessionReview(session, review.id, count).then(
      (next) => {
        setBusy(false)
        onChange(next)
        toast.success(
          count
            ? t("workspace:workSession.history.gap.counted")
            : t("workspace:workSession.history.gap.excluded"),
        )
      },
      (e: unknown) => {
        setBusy(false)
        toastFailure(
          count
            ? t("workspace:workSession.history.gap.countFailed")
            : t("workspace:workSession.history.gap.excludeFailed"),
          e,
          t("workspace:workSession.unlockNext"),
        )
      },
    )
  }
  const span = t("workspace:workSession.history.interval", {
    from: time(review.fromMs),
    to: time(review.toMs),
  })
  const status =
    review.resolution === "excluded"
      ? t("workspace:workSession.history.gap.statusExcluded")
      : review.resolution === "counted"
        ? t("workspace:workSession.history.gap.statusCounted")
        : t("workspace:workSession.history.gap.statusUnreviewed")
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <TriangleAlert className="size-4 shrink-0 text-amber-600" aria-hidden />
      <span>
        {t("workspace:workSession.history.gap.summary", {
          kind: t(REVIEW_KIND_TEXT[review.kind]),
          span,
          duration: formatElapsed(Math.max(0, review.toMs - review.fromMs)),
        })}
      </span>
      <Badge variant="outline">{status}</Badge>
      <span className="flex gap-1">
        {review.resolution === "excluded" ? (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            aria-label={t("workspace:workSession.history.gap.includeLabel", { span })}
            onClick={() => {
              resolve(true)
            }}
          >
            {t("workspace:workSession.history.gap.include")}
          </Button>
        ) : (
          <>
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={t("workspace:workSession.history.gap.excludeLabel", { span })}
              onClick={() => {
                resolve(false)
              }}
            >
              {t("workspace:workSession.history.gap.exclude")}
            </Button>
            {review.resolution === undefined && (
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={t("workspace:workSession.history.gap.keepLabel", { span })}
                onClick={() => {
                  resolve(true)
                }}
              >
                {t("workspace:workSession.history.gap.keep")}
              </Button>
            )}
          </>
        )}
      </span>
    </li>
  )
}
