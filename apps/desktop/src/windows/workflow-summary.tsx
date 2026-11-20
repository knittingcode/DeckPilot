import { CircleCheck, CircleSlash, CircleX, ListChecks } from "lucide-react"

import { ModalShell } from "@/components/modal-shell"
import { Badge } from "@/components/ui/badge"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import type { ActionStatus, StepReport } from "@/lib/types"
import { type TranslationKey, useTranslation } from "@/i18n"

import { useWindow } from "./window-context"

const STATUS: Record<
  ActionStatus,
  { label: TranslationKey; icon: typeof CircleCheck; className: string }
> = {
  succeeded: {
    label: "settings:workflowSummary.status.succeeded",
    icon: CircleCheck,
    className: "text-emerald-600",
  },
  failed: {
    label: "settings:workflowSummary.status.failed",
    icon: CircleX,
    className: "text-destructive",
  },
  skipped: {
    label: "settings:workflowSummary.status.skipped",
    icon: CircleSlash,
    className: "text-muted-foreground",
  },
}

/**
 * The result of the last workflow the agent ran: the context it ran in,
 * failed steps first, and the details every step reported. Sections for
 * integrations (Git, VPN, Docker) appear here as soon as their action types
 * report structured results; nothing is shown that was not reported.
 */
export function WorkflowSummaryWindow() {
  const { t } = useTranslation()
  const { close } = useWindow()
  const { state } = useAsync(async () => {
    const [report, connection] = await Promise.all([api.lastWorkflowReport(), api.agentStatus()])
    const profileId = connection.status?.activeProfile?.id
    const profile = profileId ? (await api.getProfile(profileId)).profile : null
    return { report, status: connection.status, profile }
  }, "summary")

  const data = state.status === "ready" ? state.data : null
  const report = data?.report ?? null
  const workflow = report && data?.profile?.workflows.find((w) => w.id === report.workflow)
  const actionName = (id: string) =>
    data?.profile?.actions.find((a) => a.id === id)?.name ??
    t("settings:workflowSummary.removedAction")
  const failed = report?.steps.filter((s) => s.result.status === "failed") ?? []

  return (
    <ModalShell
      size="medium"
      icon={ListChecks}
      title={workflow ? workflow.name : t("settings:workflowSummary.title")}
      description={
        report
          ? `${
              report.status === "failed"
                ? t("settings:workflowSummary.finishedWithProblems")
                : t("settings:workflowSummary.finished")
            } · ${t("settings:workflowSummary.stepCount", { count: report.steps.length })}`
          : undefined
      }
      loading={state.status === "loading"}
      error={state.status === "error" ? state.error : null}
      onClose={close}
      primary={{ label: t("settings:shared.close"), onClick: close }}
    >
      {data && !report && (
        <p className="text-sm text-muted-foreground">{t("settings:workflowSummary.none")}</p>
      )}
      {data && report && (
        <>
          <dl className="grid grid-cols-[8rem_1fr] gap-x-4 gap-y-1 text-sm">
            <dt className="text-muted-foreground">{t("settings:page.profile")}</dt>
            <dd>{data.status?.activeProfile?.name ?? "—"}</dd>
            <dt className="text-muted-foreground">{t("settings:workflowSummary.activeProject")}</dt>
            <dd>
              {data.status?.activeProject
                ? `${data.status.activeProject.key} · ${data.status.activeProject.name}`
                : "—"}
            </dd>
          </dl>
          {failed.length > 0 && (
            <section className="grid gap-2" aria-label={t("settings:workflowSummary.failedSteps")}>
              <h2 className="text-sm font-medium text-destructive">
                {t("settings:workflowSummary.failedSteps")}
              </h2>
              {failed.map((step) => (
                <Step key={`failed-${step.action}`} step={step} name={actionName(step.action)} />
              ))}
            </section>
          )}
          <section className="grid gap-2" aria-label={t("settings:workflowSummary.allSteps")}>
            <h2 className="text-sm font-medium">{t("settings:workflowSummary.steps")}</h2>
            <ol className="grid gap-2">
              {report.steps.map((step, index) => (
                <li key={`${step.action}-${index}`}>
                  <Step step={step} name={`${index + 1}. ${actionName(step.action)}`} />
                </li>
              ))}
            </ol>
          </section>
        </>
      )}
    </ModalShell>
  )
}

function Step({ step, name }: { step: StepReport; name: string }) {
  const { t } = useTranslation()
  const status = STATUS[step.result.status]
  const details = Object.entries(step.result.data ?? {})
  return (
    <div className="grid gap-1 rounded-md border p-3 text-sm">
      <div className="flex items-center gap-2">
        <status.icon className={`size-4 shrink-0 ${status.className}`} aria-hidden />
        <span className="flex-1 font-medium">{name}</span>
        <Badge variant="outline">{t(status.label)}</Badge>
      </div>
      {step.result.message && (
        <p className="break-words text-muted-foreground">{step.result.message}</p>
      )}
      {details.length > 0 && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 font-mono text-xs">
          {details.map(([key, value]) => (
            <div key={key} className="contents">
              <dt className="text-muted-foreground">{key}</dt>
              <dd className="break-all">{value}</dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  )
}
