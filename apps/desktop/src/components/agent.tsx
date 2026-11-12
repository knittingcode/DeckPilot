import { Play, RefreshCw, RotateCw, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { describeLayout } from "@/lib/format"
import type { AgentCapability, DeviceInfo } from "@/lib/types"

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
      toastFailure("The agent did not start", e, "Check its log in Settings › Diagnostics")
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

/** Features the running agent's protocol does not have, as the user knows them. */
const MISSING_FEATURES: Partial<Record<AgentCapability, string>> = {
  encryptedDatabase: "the encrypted configuration",
  backgroundExecutions: "background workflow runs and their history",
  notifications: "workflow notifications",
}

/** Shown when the running agent is not the one shipped with this app. */
export function AgentCompatibilityNotice() {
  const { agent, setAgent } = useAppContext()
  const [busy, setBusy] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const compatibility = agent?.compatibility
  if (!agent?.running || !compatibility || compatibility.type === "compatible") return null
  const running = agent.status?.runningExecutions ?? 0
  const restart = async (interruptRunning: boolean) => {
    setBusy(true)
    try {
      setAgent(await api.restartAgent(interruptRunning))
      toast.success("The agent was restarted")
    } catch (e) {
      toastFailure("The agent was not restarted", e, "Check its log in Settings › Diagnostics")
    } finally {
      setBusy(false)
    }
  }
  const missing =
    agent.capabilities === undefined
      ? []
      : Object.entries(MISSING_FEATURES)
          .filter(([capability]) => !agent.capabilities?.includes(capability as AgentCapability))
          .map(([, label]) => label)
  const required = compatibility.type === "updateRequired"
  return (
    <Alert variant={required ? "destructive" : "default"}>
      <TriangleAlert />
      <AlertTitle>
        {required
          ? "The running agent is too old for this app"
          : "The running agent does not match this app"}
      </AlertTitle>
      <AlertDescription>
        <p>
          Agent {compatibility.agent}, app {compatibility.app}.{" "}
          {required
            ? "Nothing can be configured until it is replaced by the agent of this app."
            : "Restart the agent so both run the same version."}
        </p>
        {missing.length > 0 && <p>Until then these are unavailable: {missing.join(", ")}.</p>}
        {running > 0 && (
          <p>
            {running === 1 ? "A workflow is" : `${running} workflows are`} running in the agent.
            Restarting now interrupts {running === 1 ? "it" : "them"}; the history records that.
          </p>
        )}
        <div className="mt-2 flex gap-2">
          {running > 0 && !confirming ? (
            <Button
              size="sm"
              disabled={busy}
              onClick={() => {
                setConfirming(true)
              }}
            >
              <RotateCw /> Restart agent…
            </Button>
          ) : (
            <Button
              size="sm"
              variant={running > 0 ? "destructive" : "default"}
              disabled={busy}
              onClick={() => void restart(running > 0)}
            >
              <RotateCw /> {running > 0 ? "Interrupt workflows and restart" : "Restart agent"}
            </Button>
          )}
          {confirming && (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => {
                setConfirming(false)
              }}
            >
              Wait for them
            </Button>
          )}
        </div>
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
          (e: unknown) => {
            toastFailure("The devices were not refreshed", e, "Check that the agent is running")
          },
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
