import type { AgentConnection, DeviceInfo, UpdateState } from "./types"

/** Pure helpers of the status bar's core items. */

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
  if (days > 0) return `${days} d ${hours} h`
  if (hours > 0) return `${hours} h ${minutes} min`
  if (minutes > 0) return `${minutes} min`
  return "less than a minute"
}

export type AgentState = "connecting" | "running" | "stopped" | "incompatible"

export const AGENT_LABEL: Record<AgentState, string> = {
  connecting: "Connecting to agent…",
  running: "Agent running",
  stopped: "Agent stopped",
  incompatible: "Agent version mismatch",
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
  if (connected.length === 0) return "No devices"
  const parts = []
  if (physical > 0) parts.push(`${physical} device${physical === 1 ? "" : "s"}`)
  if (simulated > 0) parts.push(`${simulated} simulated`)
  return parts.join(" · ")
}

export function updateItem(updates: UpdateState | null) {
  if (!updates?.configured) return null
  const version = updates.available?.version
  const { phase, progress } = updates
  if (phase === "checking") return { label: "Checking for updates…", progress: true as const }
  if (phase === "downloading") {
    const percent = progress?.total
      ? Math.round((progress.downloaded / progress.total) * 100)
      : true
    return { label: `Downloading ${version ?? "update"}`, progress: percent }
  }
  if (phase === "ready") return { label: `Restart to update to ${version ?? "the new version"}` }
  if (phase === "installing") return { label: "Installing update…", progress: true as const }
  if (updates.error) return { label: "Update failed", tone: "danger" as const }
  if (version) return { label: `Update ${version}` }
  return null
}
