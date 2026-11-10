import { Play, RefreshCw } from "lucide-react"
import { useState } from "react"

import { ErrorNotice } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { describeLayout } from "@/lib/format"
import type { DeviceInfo } from "@/lib/types"

export function AgentBadge() {
  const { agent } = useAppContext()
  if (!agent) return <Badge variant="outline">Checking…</Badge>
  return agent.running ? (
    <Badge className="bg-emerald-600 text-white">Running</Badge>
  ) : (
    <Badge variant="destructive">Stopped</Badge>
  )
}

/** Start button and explanation shown while the agent is not running. */
export function AgentStopped() {
  const { agent, setAgent } = useAppContext()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const start = async () => {
    setBusy(true)
    setError(null)
    try {
      const connection = await api.startAgent()
      setAgent(connection)
      if (!connection.running) setError("The agent did not start. Check its log for details.")
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="grid gap-3">
      <p className="text-sm text-muted-foreground">
        The background agent handles devices and runs actions. It keeps running when this window is
        closed.
      </p>
      {error && <ErrorNotice error={error} />}
      <div>
        <Button size="sm" disabled={busy || !agent?.canStart} onClick={() => void start()}>
          <Play /> Start agent
        </Button>
        {agent && !agent.canStart && (
          <p className="mt-2 text-xs text-muted-foreground">
            The agent executable was not found next to this application. Start
            <code className="mx-1">deckpilot-agent</code>manually.
          </p>
        )}
      </div>
    </div>
  )
}

export function RefreshDevicesButton() {
  const [error, setError] = useState<string | null>(null)
  return (
    <>
      <Button
        variant="outline"
        size="sm"
        onClick={() => {
          setError(null)
          api.refreshDevices().catch((e: unknown) => {
            setError(errorMessage(e))
          })
        }}
      >
        <RefreshCw /> Refresh devices
      </Button>
      {error && <ErrorNotice error={error} />}
    </>
  )
}

export function DeviceRow({
  device,
  onGenerateLayout,
}: {
  device: DeviceInfo
  onGenerateLayout?: (device: DeviceInfo) => void
}) {
  const connected = device.state === "connected"
  const layout = device.layout
  const needsLayout = layout.type === "missing" || layout.type === "invalid"
  return (
    <div className="grid gap-1 rounded-lg border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{device.modelName}</span>
        <span className="font-mono text-xs text-muted-foreground select-text">
          {device.id.serial}
        </span>
        <Badge variant={connected ? "default" : "outline"}>
          {connected ? "Connected" : "Disconnected"}
        </Badge>
        <div className="flex-1" />
        {onGenerateLayout && layout.type !== "noProfile" && layout.type !== "unknownModel" && (
          <Button
            variant={needsLayout ? "default" : "outline"}
            size="sm"
            onClick={() => {
              onGenerateLayout(device)
            }}
          >
            {needsLayout ? "Create layout from actions" : "Regenerate layout"}
          </Button>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        {describeLayout(layout)}
        {connected && device.page && ` · showing page “${device.page}”`}
      </p>
      {layout.type === "invalid" && (
        <ul className="list-disc pl-5 text-xs text-destructive">
          {layout.issues.map((issue) => (
            <li key={issue}>{issue}</li>
          ))}
        </ul>
      )}
    </div>
  )
}
