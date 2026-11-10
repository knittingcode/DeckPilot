import {
  Download,
  MonitorSmartphone,
  RefreshCw,
  Server,
  TriangleAlert,
  UserRound,
} from "lucide-react"
import { useEffect, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { useStatus, useStatusItem } from "@/hooks/use-status"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"
import { useSecurity } from "@/lib/security-context"
import {
  AGENT_LABEL,
  agentState,
  deviceSummary,
  formatDuration,
  summary,
  updateItem,
} from "@/lib/status-model"
import type { AgentConnection, UpdateState } from "@/lib/types"

function Detail({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-2">
      <h2 className="text-xs font-medium text-muted-foreground uppercase">{title}</h2>
      {children}
    </div>
  )
}

function AgentDetail() {
  const { agent, openSettings } = useAppContext()
  const status = agent?.status
  const state = agentState(agent)
  // Read when the popover opens; the uptime does not tick.
  const [now] = useState(() => Date.now() / 1000)
  return (
    <Detail title="Background agent">
      <dl className="grid grid-cols-[7rem_1fr] gap-x-3 gap-y-1 text-sm">
        <dt className="text-muted-foreground">Connection</dt>
        <dd>{AGENT_LABEL[state]}</dd>
        {status && (
          <>
            <dt className="text-muted-foreground">Version</dt>
            <dd>
              {status.version} (protocol {status.protocol})
            </dd>
            <dt className="text-muted-foreground">Running for</dt>
            <dd title={`Started ${formatTimestamp(status.startedAt)}`}>
              {formatDuration(now - status.startedAt)}
            </dd>
          </>
        )}
        {agent?.compatibility && agent.compatibility.type !== "compatible" && (
          <>
            <dt className="text-muted-foreground">App version</dt>
            <dd>{agent.compatibility.app}</dd>
          </>
        )}
      </dl>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            openSettings("agent")
          }}
        >
          Agent settings
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            openSettings("diagnostics")
          }}
        >
          Logs & diagnostics
        </Button>
      </div>
    </Detail>
  )
}

function DevicesDetail() {
  const { agent, openSettings } = useAppContext()
  const devices = agent?.status?.devices ?? []
  return (
    <Detail title="Devices">
      {devices.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {agent?.running ? "No Stream Deck detected." : "The agent is not running."}
        </p>
      ) : (
        <ul className="grid gap-1.5 text-sm">
          {devices.map((device) => (
            <li key={device.id.serial} className="grid">
              <span className="flex items-center gap-1.5">
                <span
                  aria-hidden
                  className={
                    device.state === "connected"
                      ? "size-1.5 rounded-full bg-emerald-500"
                      : "size-1.5 rounded-full bg-muted-foreground/50"
                  }
                />
                <span className="font-medium">{device.modelName}</span>
                {device.source === "mock" && (
                  <span className="rounded border px-1 text-[0.6875rem] text-muted-foreground">
                    Simulated
                  </span>
                )}
              </span>
              <span className="pl-3 text-xs text-muted-foreground">
                {device.id.serial} · {device.state === "connected" ? "Connected" : "Disconnected"}
              </span>
            </li>
          ))}
        </ul>
      )}
      <Button
        size="sm"
        variant="outline"
        className="justify-self-start"
        onClick={() => {
          openSettings("devices")
        }}
      >
        Device settings
      </Button>
    </Detail>
  )
}

/**
 * Core status items and the events that become notifications and
 * background operations. Renders nothing.
 */
export function CoreStatusItems() {
  const { agent, updates, openSettings, navigate } = useAppContext()
  const state = agentState(agent)
  const devices = agent?.status?.devices ?? []

  useStatusItem({
    id: "core.agent",
    owner: "core",
    placement: "left",
    priority: 100,
    icon: state === "incompatible" ? TriangleAlert : Server,
    label: state === "running" ? "Agent" : AGENT_LABEL[state],
    tone:
      state === "running"
        ? "positive"
        : state === "connecting"
          ? "neutral"
          : state === "stopped"
            ? "danger"
            : "warning",
    tooltip: AGENT_LABEL[state],
    progress: state === "connecting" ? true : undefined,
    detail: AgentDetail,
  })
  useStatusItem(
    agent?.running
      ? {
          id: "core.devices",
          owner: "core",
          placement: "left",
          priority: 90,
          icon: MonitorSmartphone,
          label: deviceSummary(devices),
          tooltip: "Connected Stream Deck devices",
          detail: DevicesDetail,
        }
      : null,
  )
  const profile = agent?.status?.activeProfile
  useStatusItem(
    profile
      ? {
          id: "core.profile",
          owner: "core",
          placement: "left",
          priority: 60,
          icon: UserRound,
          label: profile.name,
          tooltip: "Profile active on the device",
          onSelect: () => {
            navigate("profiles")
          },
        }
      : null,
  )
  const update = updateItem(updates)
  useStatusItem(
    update
      ? {
          id: "core.update",
          owner: "core",
          placement: "right",
          priority: 85,
          icon: update.progress === undefined ? Download : RefreshCw,
          tooltip: "Open update settings",
          onSelect: () => {
            openSettings("updates")
          },
          ...update,
        }
      : null,
  )

  return (
    <>
      <AgentEvents agent={agent} />
      <UpdateEvents updates={updates} />
      <SecurityEvents />
    </>
  )
}

/** Agent stopped, version mismatch, device disconnected. */
function AgentEvents({ agent }: { agent: AgentConnection | null }) {
  const stores = useStatus()
  const previous = useRef<AgentConnection | null>(null)
  useEffect(() => {
    const before = previous.current
    previous.current = agent
    if (!stores || !agent) return
    const { notifications } = stores
    if (before?.running && !agent.running) {
      notifications.notify({
        key: "agent.stopped",
        severity: "warning",
        title: "The background agent stopped",
        message: "Devices are blank until it is started again.",
      })
    }
    const compatibility = agent.compatibility
    if (
      compatibility &&
      compatibility.type !== "compatible" &&
      before?.compatibility?.type !== compatibility.type
    ) {
      notifications.notify({
        key: "agent.compatibility",
        severity: "warning",
        title: "Agent and app versions differ",
        message: `Agent ${compatibility.agent}, app ${compatibility.app}.`,
      })
    }
    const now = new Map((agent.status?.devices ?? []).map((d) => [d.id.serial, d]))
    for (const device of before?.status?.devices ?? []) {
      const current = now.get(device.id.serial)
      if (device.state === "connected" && agent.running && current?.state !== "connected") {
        notifications.notify({
          key: `device.${device.id.serial}.disconnected`,
          severity: "warning",
          title: `${device.modelName} disconnected`,
          message: device.source === "mock" ? "Simulated device" : `Serial ${device.id.serial}`,
        })
      }
    }
  }, [agent, stores])
  return null
}

/** Update available or failed, and download/installation as operations. */
function UpdateEvents({ updates }: { updates: UpdateState | null }) {
  const stores = useStatus()
  const previous = useRef<UpdateState | null>(null)
  useEffect(() => {
    const before = previous.current
    previous.current = updates
    if (!stores || !updates) return
    const { notifications, operations } = stores
    const version = updates.available?.version
    if (version && before?.available?.version !== version) {
      notifications.notify({
        key: `update.available.${version}`,
        severity: "info",
        title: `DeckPilot ${version} is available`,
      })
    }
    if (updates.error && updates.error !== before?.error) {
      notifications.notify({
        key: "update.failed",
        severity: "error",
        title: "Update failed",
        message: summary(updates.error),
      })
    }
    // Download and installation are operations while the backend reports them.
    if (updates.phase === "downloading") {
      const percent = updates.progress?.total
        ? Math.round((updates.progress.downloaded / updates.progress.total) * 100)
        : undefined
      operations.start({
        id: "update.download",
        kind: "update",
        title: `Downloading DeckPilot ${version ?? "update"}`,
      })
      operations.update("update.download", { progress: percent })
    } else if (before?.phase === "downloading") {
      operations.finish("update.download", updates.error ? { error: "Download failed" } : {})
    }
    if (updates.phase === "installing") {
      operations.start({
        id: "update.install",
        kind: "update",
        title: "Installing the update",
      })
    } else if (before?.phase === "installing") {
      operations.finish("update.install", updates.error ? { error: "Installation failed" } : {})
    }
  }, [updates, stores])
  return null
}

/** Problems with the configuration's protection that need the user. */
function SecurityEvents() {
  const stores = useStatus()
  const { view } = useSecurity()
  const problem = view.problem
  const legacy = view.legacyCopies.length
  useEffect(() => {
    if (!stores) return
    if (problem) {
      stores.notifications.notify({
        key: "security.problem",
        severity: "warning",
        title: "Security needs your attention",
        message: summary(problem),
      })
    }
    if (legacy > 0) {
      stores.notifications.notify({
        key: "security.unencrypted-copy",
        severity: "warning",
        title: "An unencrypted copy of the configuration remains",
        message: "Remove it under Settings → Security.",
      })
    }
  }, [problem, legacy, stores])
  return null
}
