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

export function DiagnosticsSettings() {
  const { agent, revision } = useAppContext()
  const { state } = useAsync(() => api.getOverview(), String(revision))
  const status = agent?.status
  return (
    <>
      <SettingsSection
        title="Logs"
        description="The agent writes a log of device events, action runs and errors. Logs stay on this computer."
      >
        <SettingsGroup>
          <SettingsRow
            label="Agent log"
            description={
              status && <span className="font-mono break-all select-text">{status.logDir}</span>
            }
          >
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                api.openWindow("app.logs").catch((e: unknown) => {
                  toastFailure("The log window did not open", e)
                })
              }}
            >
              <ScrollText /> Show log…
            </Button>
          </SettingsRow>
        </SettingsGroup>
      </SettingsSection>
      <SettingsSection
        title="Diagnostics"
        description="Include these details when you report a problem. They contain paths on this computer but no configuration content."
        actions={
          state.status === "ready" && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigator.clipboard.writeText(diagnosticsReport(state.data, agent)).then(
                  () => toast.success("Diagnostics copied"),
                  (e: unknown) => {
                    toastFailure(
                      "The diagnostics were not copied",
                      e,
                      "Select the report and copy it by hand",
                    )
                  },
                )
              }}
            >
              <ClipboardCopy /> Copy
            </Button>
          )
        }
      >
        {agent && !agent.running && <AgentStopped />}
        {state.status === "loading" && <Loading />}
        {state.status === "error" && <ErrorNotice error={state.error} />}
        {state.status === "ready" && (
          <pre className="overflow-x-auto rounded-md bg-muted px-3 py-2 text-xs leading-5 whitespace-pre-wrap select-text">
            {diagnosticsReport(state.data, agent)}
          </pre>
        )}
      </SettingsSection>
    </>
  )
}
