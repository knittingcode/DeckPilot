import { Play, RefreshCw, RotateCw, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
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

  const start = async () => {
    setBusy(true)
    try {
      const connection = await api.startAgent()
      setAgent(connection)
      if (!connection.running) toast.error("The agent did not start. Check its log for details.")
    } catch (e) {
      toast.error(errorMessage(e))
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

/** Shown when the running agent is not the one shipped with this app. */
export function AgentCompatibilityNotice() {
  const { agent, setAgent } = useAppContext()
  const [busy, setBusy] = useState(false)
  const compatibility = agent?.compatibility
  if (!agent?.running || !compatibility || compatibility.type === "compatible") return null
  const restart = async () => {
    setBusy(true)
    try {
      setAgent(await api.restartAgent())
      toast.success("The agent was restarted")
    } catch (e) {
      toast.error(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Alert>
      <TriangleAlert />
      <AlertTitle>The running agent does not match this app</AlertTitle>
      <AlertDescription>
        <p>
          {compatibility.type === "agentOutdated"
            ? `The agent (${compatibility.agent}) is older than this app (${compatibility.app}).`
            : compatibility.type === "agentNewer"
              ? `The agent (${compatibility.agent}) is newer than this app (${compatibility.app}).`
              : `The agent's version (${compatibility.agent}) could not be compared with this app (${compatibility.app}).`}{" "}
          Restart the agent so both run the same version.
        </p>
        <Button className="mt-2" size="sm" disabled={busy} onClick={() => void restart()}>
          <RotateCw /> Restart agent
        </Button>
      </AlertDescription>
    </Alert>
  )
}

export function RefreshDevicesButton() {
  return (
    <Button
      variant="outline"
      size="sm"
      onClick={() => {
        api.refreshDevices().then(
          () => toast.success("Devices refreshed"),
          (e: unknown) => toast.error(errorMessage(e)),
        )
      }}
    >
      <RefreshCw /> Refresh devices
    </Button>
  )
}

export function DeviceRow({ device, action }: { device: DeviceInfo; action?: React.ReactNode }) {
  const connected = device.state === "connected"
  const layout = device.layout
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
        <Badge variant="secondary">{device.source === "mock" ? "Simulated" : "USB"}</Badge>
        <div className="flex-1" />
        {action}
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

/** Compact connection state for the titlebar; opens the agent settings. */
export function AgentIndicator({ onOpen }: { onOpen: () => void }) {
  const { agent } = useAppContext()
  const running = agent?.running ?? false
  const label = !agent ? "Checking the agent" : running ? "Agent running" : "Agent stopped"
  return (
    <button
      type="button"
      aria-label={`${label}, open agent settings`}
      title={label}
      className="flex size-(--titlebar-control) shrink-0 items-center justify-center rounded-md hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      onClick={onOpen}
    >
      <span
        className={
          !agent
            ? "size-2 rounded-full bg-muted-foreground/40"
            : running
              ? "size-2 rounded-full bg-emerald-500"
              : "size-2 rounded-full bg-destructive"
        }
        aria-hidden
      />
    </button>
  )
}
