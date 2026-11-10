import { osLabel } from "./format"
import type { AgentConnection, Overview } from "./types"

/** Technical details for a bug report. Contains paths, never configuration content. */
export function diagnosticsReport(overview: Overview, agent: AgentConnection | null) {
  const status = agent?.status
  const lines = [
    `DeckPilot ${overview.appVersion}${overview.development ? " (development session)" : ""}`,
    `Operating system: ${osLabel(overview.os)}`,
    `Database: ${overview.databasePath}`,
    status
      ? `Agent: ${status.version}, protocol ${status.protocol}, backend ${status.backend}`
      : "Agent: not running",
    ...(status
      ? [
          `IPC endpoint: ${status.endpoint}`,
          `Agent data: ${status.dataDir}`,
          `Logs: ${status.logDir}`,
          `Devices: ${status.devices.length}`,
        ]
      : []),
    `Compatibility: ${agent?.compatibility?.type ?? "unknown"}`,
  ]
  return lines.join("\n")
}
