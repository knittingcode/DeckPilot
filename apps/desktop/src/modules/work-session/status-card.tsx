import { CircleAlert, CircleCheck, CircleX } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import type { ElapsedFormat, WorkSessionSnapshot } from "@/lib/types"

import {
  STATE_TEXT,
  describeDuration,
  formatElapsed,
  noticeText,
  showsElapsed,
  timerRunning,
} from "./model"
import { useLiveElapsed } from "./use-work-session"

/**
 * The live state of the agent's work session. It only shows the state: the
 * session is started, checked and finished from its key (D11).
 */
export function StatusCard({
  snapshot,
  format,
  projectName,
  onChange,
}: {
  snapshot: WorkSessionSnapshot
  format: ElapsedFormat
  projectName: (id: string) => string
  onChange: (snapshot: WorkSessionSnapshot) => void
}) {
  const text = STATE_TEXT[snapshot.state]
  const notices = snapshot.notices ?? []
  return (
    <Card>
      <CardHeader>
        <CardTitle>Current session</CardTitle>
        <CardDescription>
          Operated from its key on the device; this page only shows the state.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
        <div className="flex flex-wrap items-end gap-x-6 gap-y-2">
          <div className="grid gap-1">
            <span className="text-xs text-muted-foreground">State</span>
            <p aria-live="polite" className="flex items-center gap-2 text-lg font-medium">
              <Badge variant={timerRunning(snapshot.state) ? "default" : "secondary"}>
                {text.label}
              </Badge>
            </p>
          </div>
          {showsElapsed(snapshot.state) && <Elapsed snapshot={snapshot} format={format} />}
          {snapshot.project && (
            <div className="grid gap-1">
              <span className="text-xs text-muted-foreground">Project</span>
              <span className="text-sm">{projectName(snapshot.project)}</span>
            </div>
          )}
          {snapshot.startedAtMs !== undefined && (
            <div className="grid gap-1">
              <span className="text-xs text-muted-foreground">Started</span>
              <span className="text-sm">{new Date(snapshot.startedAtMs).toLocaleString()}</span>
            </div>
          )}
        </div>
        <p className="text-sm text-muted-foreground">{text.hint}</p>
        {snapshot.verification && <LastCheck snapshot={snapshot} />}
        {notices.length > 0 && (
          <section aria-label="Notices" className="grid gap-2">
            <ul className="grid gap-2">
              {notices.map((notice) => {
                const { title, detail } = noticeText(notice)
                return (
                  <li
                    key={`${notice.problem}:${notice.atMs}`}
                    className="flex gap-2 rounded-md border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-sm"
                  >
                    <CircleAlert className="mt-0.5 size-4 shrink-0 text-amber-600" aria-hidden />
                    <div className="grid gap-0.5">
                      <span className="font-medium">{title}</span>
                      <span className="text-muted-foreground">{detail}</span>
                      <span className="text-xs text-muted-foreground">
                        {new Date(notice.atMs).toLocaleString()}
                      </span>
                    </div>
                  </li>
                )
              })}
            </ul>
            <div>
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  api
                    .clearWorkSessionNotices(snapshot.session ?? null)
                    .then(onChange, (e: unknown) => {
                      toastFailure(
                        "The notices were not cleared",
                        e,
                        "Unlock DeckPilot if it is locked, then try again.",
                      )
                    })
                }}
              >
                Clear notices
              </Button>
            </div>
          </section>
        )}
      </CardContent>
    </Card>
  )
}

function Elapsed({ snapshot, format }: { snapshot: WorkSessionSnapshot; format: ElapsedFormat }) {
  const elapsed = useLiveElapsed(snapshot, format)
  return (
    <div className="grid gap-1">
      <span className="text-xs text-muted-foreground" id="work-session-elapsed">
        Elapsed
      </span>
      <time
        role="timer"
        aria-labelledby="work-session-elapsed"
        title={describeDuration(elapsed)}
        className="font-mono text-3xl font-semibold tabular-nums"
      >
        {formatElapsed(elapsed, format)}
      </time>
    </div>
  )
}

function LastCheck({ snapshot }: { snapshot: WorkSessionSnapshot }) {
  const record = snapshot.verification
  if (!record) return null
  const outcome = record.outcome
  const Icon = outcome === undefined ? CircleAlert : outcome.canFinish ? CircleCheck : CircleX
  const label =
    outcome === undefined ? "No result" : outcome.canFinish ? "Can finish" : "Not ready to finish"
  return (
    <div className="flex gap-2 text-sm">
      <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
      <p>
        <span className="font-medium">Last end check: {label}</span>
        {outcome?.details && <span className="text-muted-foreground"> — {outcome.details}</span>}
        <span className="block text-xs text-muted-foreground">
          {new Date(record.atMs).toLocaleString()}
        </span>
      </p>
    </div>
  )
}
