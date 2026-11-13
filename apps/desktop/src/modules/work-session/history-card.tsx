import { ChevronDown, ChevronRight, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
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
} from "./model"

/** Sessions loaded at a time. */
export const HISTORY_PAGE = 20

const time = (ms: number) => new Date(ms).toLocaleString()

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
    <Card>
      <CardHeader>
        <CardTitle>History</CardTitle>
        <CardDescription>
          Sessions kept on this computer, newest first. Gaps longer than the review threshold are
          counted and flagged; exclude one if you did not work then.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {history.state.status === "loading" ? (
          <Loading />
        ) : history.state.status === "error" ? (
          <ErrorNotice
            error={failureMessage(
              "The session history could not be loaded",
              history.state.error,
              "Check that the agent is running in Settings › Agent.",
            )}
          />
        ) : history.state.data.length === 0 ? (
          <EmptyState title="No sessions yet">
            Sessions appear here once the Work Session key has started one.
          </EmptyState>
        ) : (
          <div className="grid gap-3">
            <ul aria-label="Work sessions" className="grid gap-2">
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
                  Show older sessions
                </Button>
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
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
    <li className="rounded-md border">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-3 py-2 text-sm">
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
          aria-label={`Duration ${describeDuration(elapsed)}`}
        >
          {summary.state === "aborted" ? "–" : formatElapsed(elapsed)}
        </span>
        <Badge variant={summary.state === "aborted" ? "destructive" : "secondary"}>
          {RECORD_TEXT[summary.state]}
        </Badge>
        {flagged > 0 && (
          <Badge variant="outline" className="border-amber-500/60 text-amber-700">
            <TriangleAlert aria-hidden />
            {flagged === 1 ? "1 gap to review" : `${flagged} gaps to review`}
          </Badge>
        )}
      </div>
      {open && (
        <div id={detailsId} className="grid gap-4 border-t px-3 py-3 text-sm">
          {summary.notice && (
            <section className="grid gap-1">
              <h4 className="font-medium">Notice</h4>
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
              <h4 className="font-medium">Flagged gaps</h4>
              <ul aria-label="Flagged gaps" className="grid gap-2">
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
            <h4 className="font-medium">Intervals</h4>
            {summary.intervals.length === 0 ? (
              <p className="text-muted-foreground">
                None: the timer never started (the start workflow did not succeed).
              </p>
            ) : (
              <ol aria-label="Intervals" className="grid gap-1">
                {summary.intervals.map((interval) => (
                  <li key={interval.seq} className="flex flex-wrap gap-x-3">
                    <span>
                      {time(interval.startedAtMs)} –{" "}
                      {interval.endedAtMs === undefined ? "now" : time(interval.endedAtMs)}
                    </span>
                    <span className="font-mono text-muted-foreground tabular-nums">
                      {formatElapsed(
                        Math.max(0, (interval.endedAtMs ?? now) - interval.startedAtMs),
                      )}
                    </span>
                    {interval.endReason === "gapExcluded" && (
                      <span className="text-muted-foreground">(a gap after it is excluded)</span>
                    )}
                  </li>
                ))}
              </ol>
            )}
          </section>
          {summary.verification && (
            <section className="grid gap-1">
              <h4 className="font-medium">Last end check</h4>
              <p className="text-muted-foreground">
                {summary.verification.outcome === undefined
                  ? "No result"
                  : summary.verification.outcome.canFinish
                    ? "Can finish"
                    : "Not ready to finish"}
                {summary.verification.outcome?.details
                  ? ` — ${summary.verification.outcome.details}`
                  : ""}{" "}
                ({time(summary.verification.atMs)})
              </p>
            </section>
          )}
          {summary.transitions.length > 0 && (
            <section className="grid gap-1">
              <h4 className="font-medium">Transitions</h4>
              <ol aria-label="Transitions" className="grid gap-0.5 text-muted-foreground">
                {summary.transitions.map((t) => (
                  <li key={t.seq}>
                    {time(t.atMs)}: {t.from} → {t.to} ({t.event})
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
  const [busy, setBusy] = useState(false)
  const resolve = (count: boolean) => {
    setBusy(true)
    api.resolveWorkSessionReview(session, review.id, count).then(
      (next) => {
        setBusy(false)
        onChange(next)
        toast.success(count ? "The gap is counted" : "The gap is excluded")
      },
      (e: unknown) => {
        setBusy(false)
        toastFailure(
          count ? "The gap was not counted" : "The gap was not excluded",
          e,
          "Unlock DeckPilot if it is locked, then try again.",
        )
      },
    )
  }
  const span = `${time(review.fromMs)} – ${time(review.toMs)}`
  const status =
    review.resolution === "excluded"
      ? "Excluded"
      : review.resolution === "counted"
        ? "Counted"
        : "Counted, not reviewed"
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <TriangleAlert className="size-4 shrink-0 text-amber-600" aria-hidden />
      <span>
        {REVIEW_KIND_TEXT[review.kind]}: {span} (
        {formatElapsed(Math.max(0, review.toMs - review.fromMs))})
      </span>
      <Badge variant="outline">{status}</Badge>
      <span className="flex gap-1">
        {review.resolution === "excluded" ? (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            aria-label={`Include the gap ${span}`}
            onClick={() => {
              resolve(true)
            }}
          >
            Include
          </Button>
        ) : (
          <>
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={`Exclude the gap ${span}`}
              onClick={() => {
                resolve(false)
              }}
            >
              Exclude
            </Button>
            {review.resolution === undefined && (
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={`Keep the gap ${span} counted`}
                onClick={() => {
                  resolve(true)
                }}
              >
                Keep counted
              </Button>
            )}
          </>
        )}
      </span>
    </li>
  )
}
