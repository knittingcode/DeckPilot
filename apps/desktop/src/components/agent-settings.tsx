import { ScrollText, Square } from "lucide-react"
import { useEffect, useState } from "react"

import { AgentBadge, AgentStopped } from "@/components/agent"
import { ErrorNotice } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"
import type { AgentStatus } from "@/lib/types"

export function AgentSettings() {
  const { agent } = useAppContext()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          Background agent <AgentBadge />
        </CardTitle>
        <CardDescription>
          The agent owns device communication and action execution and runs independently of this
          window.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-6">
        {agent && !agent.running && <AgentStopped />}
        {agent?.status && <RunningAgent status={agent.status} />}
        <Button
          variant="outline"
          className="justify-self-start"
          onClick={() => {
            api.openWindow("app.logs").catch(() => undefined)
          }}
        >
          <ScrollText /> Show log…
        </Button>
      </CardContent>
    </Card>
  )
}

function RunningAgent({ status }: { status: AgentStatus }) {
  const { setAgent } = useAppContext()
  const [brightness, setBrightness] = useState(status.brightness)
  const [autostart, setAutostart] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api.getAutostart().then(setAutostart, () => {
      setAutostart(null)
    })
  }, [])

  const run = (operation: () => Promise<unknown>) => {
    setError(null)
    operation().catch((e: unknown) => {
      setError(errorMessage(e))
    })
  }

  const rows: [string, string][] = [
    ["Version", `${status.version} (protocol ${status.protocol})`],
    ["Process", `PID ${status.pid}, started ${formatTimestamp(status.startedAt)}`],
    ["Device backend", status.backend],
    ["IPC endpoint", status.endpoint],
    ["Data directory", status.dataDir],
    ["Logs", status.logDir],
  ]

  return (
    <>
      {error && <ErrorNotice error={error} />}
      <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[10rem_1fr]">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="font-mono text-xs break-all select-text">{value}</dd>
          </div>
        ))}
      </dl>

      <div className="grid max-w-md gap-2">
        <Label htmlFor="brightness">Device brightness: {brightness}%</Label>
        <div className="flex items-center gap-3">
          <input
            id="brightness"
            type="range"
            min={0}
            max={100}
            step={5}
            value={brightness}
            className="flex-1"
            onChange={(e) => {
              setBrightness(Number(e.target.value))
            }}
          />
          <Button
            size="sm"
            variant="outline"
            disabled={brightness === status.brightness}
            onClick={() => {
              run(() => api.setBrightness(brightness))
            }}
          >
            Apply
          </Button>
        </div>
      </div>

      <div className="flex items-center gap-2">
        <input
          id="autostart"
          type="checkbox"
          className="size-4"
          disabled={autostart === null}
          checked={autostart ?? false}
          onChange={(e) => {
            const enabled = e.target.checked
            run(async () => {
              setAutostart(await api.setAutostart(enabled))
            })
          }}
        />
        <Label htmlFor="autostart">Start the agent when I log in</Label>
        {autostart === null && (
          <span className="text-xs text-muted-foreground">(not available)</span>
        )}
      </div>

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
  )
}
