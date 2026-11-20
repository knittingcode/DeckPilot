import { ClipboardCopy, ScrollText } from "lucide-react"
import { toast } from "sonner"

import { AgentStopped } from "@/components/agent"
import { ErrorNotice, Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { diagnosticsReport } from "@/lib/diagnostics"
import { useTranslation } from "@/i18n"

export function DiagnosticsSettings() {
  const { t, i18n } = useTranslation()
  const { agent, revision } = useAppContext()
  const { state } = useAsync(() => api.getOverview(), String(revision))
  const status = agent?.status
  return (
    <>
      <SettingsSection
        title={t("settings:diagnostics.logsTitle")}
        description={t("settings:diagnostics.logsDescription")}
      >
        <SettingsGroup>
          <SettingsRow
            label={t("settings:diagnostics.agentLog")}
            description={
              status && <span className="font-mono break-all select-text">{status.logDir}</span>
            }
          >
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                api.openWindow("app.logs").catch((e: unknown) => {
                  toastFailure(t("settings:diagnostics.logWindowFailed"), e)
                })
              }}
            >
              <ScrollText /> {t("settings:diagnostics.showLog")}
            </Button>
          </SettingsRow>
        </SettingsGroup>
      </SettingsSection>
      <SettingsSection
        title={t("settings:diagnostics.title")}
        description={t("settings:diagnostics.description")}
        actions={
          state.status === "ready" && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigator.clipboard.writeText(diagnosticsReport(state.data, agent)).then(
                  () => toast.success(t("settings:diagnostics.copied")),
                  (e: unknown) => {
                    toastFailure(
                      t("settings:diagnostics.notCopied"),
                      e,
                      t("settings:diagnostics.copyByHand"),
                    )
                  },
                )
              }}
            >
              <ClipboardCopy /> {t("settings:shared.copy")}
            </Button>
          )
        }
      >
        {agent && !agent.running && <AgentStopped />}
        {state.status === "loading" && <Loading />}
        {state.status === "error" && <ErrorNotice error={state.error} />}
        {state.status === "ready" && (
          <pre className="relative overflow-x-auto rounded-md bg-muted px-3 py-2 text-xs leading-5 whitespace-pre-wrap select-text">
            {diagnosticsReport(state.data, agent)}
          </pre>
        )}
        {state.status === "ready" && (i18n.resolvedLanguage ?? i18n.language) !== "en" && (
          <p className="text-caption text-muted-foreground">
            {t("settings:diagnostics.reportInEnglish")}
          </p>
        )}
      </SettingsSection>
    </>
  )
}
