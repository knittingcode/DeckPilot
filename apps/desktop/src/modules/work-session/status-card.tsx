import { CircleAlert, CircleCheck, CircleX } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import type { ElapsedFormat, WorkSessionSnapshot } from "@/lib/types"

import {
  STATE_TEXT,
  describeDuration,
  formatElapsed,
  noticeText,
  outcomeKey,
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
  const { t } = useTranslation()
  const { dateTime } = useFormat()
  const text = STATE_TEXT[snapshot.state]
  const notices = snapshot.notices ?? []
  return (
    <SettingsSection
      title={t("workspace:workSession.status.title")}
      description={t("workspace:workSession.status.description")}
    >
      <div className="grid gap-4">
        <div className="flex flex-wrap items-end gap-x-6 gap-y-2">
          <div className="grid gap-1">
            <span className="text-xs text-muted-foreground">
              {t("workspace:workSession.status.state")}
            </span>
            <p aria-live="polite" className="flex items-center gap-2 text-lg font-medium">
              <Badge variant={timerRunning(snapshot.state) ? "default" : "secondary"}>
                {t(text.label)}
              </Badge>
            </p>
          </div>
          {showsElapsed(snapshot.state) && <Elapsed snapshot={snapshot} format={format} />}
          {snapshot.project && (
            <div className="grid gap-1">
              <span className="text-xs text-muted-foreground">
                {t("workspace:workSession.status.project")}
              </span>
              <span className="text-sm">{projectName(snapshot.project)}</span>
            </div>
          )}
          {snapshot.startedAtMs !== undefined && (
            <div className="grid gap-1">
              <span className="text-xs text-muted-foreground">
                {t("workspace:workSession.status.started")}
              </span>
              <span className="text-sm">{dateTime(snapshot.startedAtMs)}</span>
            </div>
          )}
        </div>
        <p className="text-sm text-muted-foreground">{t(text.hint)}</p>
        {snapshot.verification && <LastCheck snapshot={snapshot} />}
        {notices.length > 0 && (
          <section aria-label={t("workspace:workSession.status.notices")} className="grid gap-2">
            <ul className="grid gap-2">
              {notices.map((notice) => {
                const { title, detail } = noticeText(notice)
                return (
                  <li
                    key={`${notice.problem}:${notice.atMs}`}
                    className="flex gap-2 border-l-2 border-warning py-1 pl-3 text-sm"
                  >
                    <CircleAlert className="mt-0.5 size-4 shrink-0 text-amber-600" aria-hidden />
                    <div className="grid gap-0.5">
                      <span className="font-medium">{title}</span>
                      <span className="text-muted-foreground">{detail}</span>
                      <span className="text-xs text-muted-foreground">{dateTime(notice.atMs)}</span>
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
                        t("workspace:workSession.status.clearFailed"),
                        e,
                        t("workspace:workSession.unlockNext"),
                      )
                    })
                }}
              >
                {t("workspace:workSession.status.clearNotices")}
              </Button>
            </div>
          </section>
        )}
      </div>
    </SettingsSection>
  )
}

function Elapsed({ snapshot, format }: { snapshot: WorkSessionSnapshot; format: ElapsedFormat }) {
  const { t } = useTranslation()
  const elapsed = useLiveElapsed(snapshot, format)
  return (
    <div className="grid gap-1">
      <span className="text-xs text-muted-foreground" id="work-session-elapsed">
        {t("workspace:workSession.status.elapsed")}
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
  const { t } = useTranslation()
  const { dateTime } = useFormat()
  const record = snapshot.verification
  if (!record) return null
  const outcome = record.outcome
  const Icon = outcome === undefined ? CircleAlert : outcome.canFinish ? CircleCheck : CircleX
  return (
    <div className="flex gap-2 text-sm">
      <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
      <p>
        <span className="font-medium">
          {t("workspace:workSession.check.last", { outcome: t(outcomeKey(outcome)) })}
        </span>
        {outcome?.details && (
          <span className="text-muted-foreground">
            {t("workspace:workSession.check.details", { details: outcome.details })}
          </span>
        )}
        <span className="block text-xs text-muted-foreground">{dateTime(record.atMs)}</span>
      </p>
    </div>
  )
}
