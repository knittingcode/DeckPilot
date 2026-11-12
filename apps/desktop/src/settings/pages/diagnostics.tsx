import { ClipboardCopy, ScrollText } from "lucide-react"
import { toast } from "sonner"

import { AgentStopped } from "@/components/agent"
import { ErrorNotice, Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
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
      <Card>
        <CardHeader>
          <CardTitle>Logs</CardTitle>
          <CardDescription>
            The agent writes a log of device events, action runs and errors. Logs stay on this
            computer.
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-3">
          <Button
            variant="outline"
            className="justify-self-start"
            onClick={() => {
              api.openWindow("app.logs").catch((e: unknown) => {
                toastFailure("The log window did not open", e)
              })
            }}
          >
            <ScrollText /> Show log…
          </Button>
          {status && (
            <p className="font-mono text-xs break-all text-muted-foreground select-text">
              {status.logDir}
            </p>
          )}
        </CardContent>
      </Card>
      <Card>
        <CardHeader className="flex flex-row items-start gap-2">
          <div className="grid flex-1 gap-1.5">
            <CardTitle>Diagnostics</CardTitle>
            <CardDescription>
              Include these details when you report a problem. They contain paths on this computer
              but no configuration content.
            </CardDescription>
          </div>
          {state.status === "ready" && (
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
          )}
        </CardHeader>
        <CardContent className="grid gap-3">
          {agent && !agent.running && <AgentStopped />}
          {state.status === "loading" && <Loading />}
          {state.status === "error" && <ErrorNotice error={state.error} />}
          {state.status === "ready" && (
            <pre className="overflow-x-auto rounded-md bg-muted p-3 text-xs whitespace-pre-wrap select-text">
              {diagnosticsReport(state.data, agent)}
            </pre>
          )}
        </CardContent>
      </Card>
    </>
  )
}
