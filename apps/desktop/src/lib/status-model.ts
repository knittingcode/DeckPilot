import { i18n } from "@/i18n"

import type { AgentConnection, DeviceInfo, UpdateState } from "./types"

/**
 * Pure helpers of the status bar's core items. Their texts are in the
 * window's language; the components that show them re-render when it
 * changes.
 */

/** Short, single-line text for notifications; never whole backend output. */
export function summary(text: string, limit = 160) {
  const line = text.split("\n")[0]?.trim() ?? ""
  return line.length > limit ? `${line.slice(0, limit - 1)}…` : line
}

export function formatDuration(seconds: number) {
  const s = Math.max(0, Math.floor(seconds))
  const days = Math.floor(s / 86400)
  const hours = Math.floor((s % 86400) / 3600)
  const minutes = Math.floor((s % 3600) / 60)
  if (days > 0) return i18n.t("common:duration.daysHours", { days, hours })
  if (hours > 0) return i18n.t("common:duration.hoursMinutes", { hours, minutes })
  if (minutes > 0) return i18n.t("common:duration.minutes", { minutes })
  return i18n.t("common:duration.lessThanMinute")
}

export type AgentState = "connecting" | "running" | "stopped" | "incompatible"

const AGENT_LABEL = {
  connecting: "devices:agent.state.connecting",
  running: "devices:agent.state.running",
  stopped: "devices:agent.state.stopped",
  incompatible: "devices:agent.state.incompatible",
} as const satisfies Record<AgentState, string>

export function agentLabel(state: AgentState): string {
  return i18n.t(AGENT_LABEL[state])
}

export function agentState(agent: AgentConnection | null): AgentState {
  if (!agent) return "connecting"
  if (!agent.running) return "stopped"
  const type = agent.compatibility?.type
  return type && type !== "compatible" ? "incompatible" : "running"
}

export function deviceSummary(devices: DeviceInfo[]) {
  const connected = devices.filter((d) => d.state === "connected")
  const physical = connected.filter((d) => d.source !== "mock").length
  const simulated = connected.length - physical
  if (connected.length === 0) return i18n.t("devices:summary.none")
  if (simulated === 0) return i18n.t("devices:summary.physical", { count: physical })
  if (physical === 0) return i18n.t("devices:summary.simulated", { count: simulated })
  return i18n.t("devices:summary.both", {
    physical: i18n.t("devices:summary.physical", { count: physical }),
    simulated: i18n.t("devices:summary.simulated", { count: simulated }),
  })
}

export function updateItem(updates: UpdateState | null) {
  if (!updates?.configured) return null
  const version = updates.available?.version
  const { phase, progress } = updates
  if (phase === "checking") {
    return { label: i18n.t("notifications:update.checking"), progress: true as const }
  }
  if (phase === "downloading") {
    const percent = progress?.total
      ? Math.round((progress.downloaded / progress.total) * 100)
      : true
    return {
      label: version
        ? i18n.t("notifications:update.downloadingVersion", { version })
        : i18n.t("notifications:update.downloading"),
      progress: percent,
    }
  }
  if (phase === "ready") {
    return {
      label: version
        ? i18n.t("notifications:update.restartVersion", { version })
        : i18n.t("notifications:update.restart"),
    }
  }
  if (phase === "installing") {
    return { label: i18n.t("notifications:update.installing"), progress: true as const }
  }
  if (updates.error)
    return { label: i18n.t("notifications:update.failed"), tone: "danger" as const }
  if (version) return { label: i18n.t("notifications:update.available", { version }) }
  return null
}
