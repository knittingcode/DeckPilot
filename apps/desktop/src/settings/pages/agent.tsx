import { RefreshCw, Square } from "lucide-react"
import { useEffect, useState } from "react"

import { AgentBadge, AgentCompatibilityNotice, AgentStopped } from "@/components/agent"
import { ErrorNotice } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useTitlebar } from "@/hooks/use-titlebar"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"

export function AgentSettings() {
  const { agent, setAgent } = useAppContext()
  const [loadedAutostart, setAutostart] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)

  // The agent manages its own login item.
  const running = agent?.running ?? false
  useEffect(() => {
    if (!running) return
    api.getAutostart().then(setAutostart, () => {
      setAutostart(null)
    })
  }, [running])

  const run = (operation: () => Promise<unknown>) => {
    setError(null)
    operation().catch((e: unknown) => {
      setError(errorMessage(e))
    })
  }
  const status = agent?.status
  const [refreshing, setRefreshing] = useState(false)
  useTitlebar({
    actions: [
      {
        id: "agent.refresh",
        label: "Refresh status",
        icon: RefreshCw,
        placement: "primary",
        busy: refreshing,
        disabled: refreshing,
        onSelect: () => {
          setRefreshing(true)
          setError(null)
          api
            .agentStatus()
            .then(setAgent, (e: unknown) => {
              setError(errorMessage(e))
            })
            .finally(() => {
              setRefreshing(false)
            })
        },
      },
    ],
  })
  const autostart = running ? loadedAutostart : null

  return (
    <>
      <AgentCompatibilityNotice />
      {error && <ErrorNotice error={error} />}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            Status <AgentBadge />
          </CardTitle>
          <CardDescription>
            The agent owns device communication and action execution and keeps running when this
            window is closed.
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4">
          {agent && !agent.running && <AgentStopped />}
          {status && (
            <>
              <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[10rem_1fr]">
                <dt className="text-muted-foreground">Version</dt>
                <dd>
                  {status.version} (protocol {status.protocol})
                </dd>
                <dt className="text-muted-foreground">Process</dt>
                <dd>
                  PID {status.pid}, started {formatTimestamp(status.startedAt)}
                </dd>
                <dt className="text-muted-foreground">Device backend</dt>
                <dd>{status.backend}</dd>
              </dl>
              <div>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    run(async () => {
                      await api.stopAgent()
                      setAgent(await api.agentStatus())
                    })
                  }}
                >
                  <Square /> Stop agent
                </Button>
                <p className="mt-2 text-xs text-muted-foreground">
                  Stopping the agent blanks connected devices until it is started again.
                </p>
              </div>
            </>
          )}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>Startup</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-3">
            <Switch
              id="autostart"
              disabled={autostart === null}
              checked={autostart ?? false}
              onCheckedChange={(enabled) => {
                run(async () => {
                  setAutostart(await api.setAutostart(enabled))
                })
              }}
            />
            <Label htmlFor="autostart">Start the agent when I log in</Label>
            {autostart === null && (
              <span className="text-xs text-muted-foreground">
                {running ? "(not available)" : "(start the agent to change this)"}
              </span>
            )}
          </div>
        </CardContent>
      </Card>
    </>
  )
}
