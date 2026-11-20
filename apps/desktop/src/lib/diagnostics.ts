import { i18n } from "@/i18n"

import { OS_LABELS } from "./format"
import type { AgentConnection, Overview } from "./types"

/**
 * Technical details for a bug report. Contains paths, never configuration content.
 *
 * Always in English, whatever language the window shows: the report is
 * pasted into issues and support requests, read by whoever helps, and
 * searched for by its labels. The page around it is translated.
 */
export function diagnosticsReport(overview: Overview, agent: AgentConnection | null) {
  const status = agent?.status
  const osLabel = (os: Overview["os"]) => i18n.t(OS_LABELS[os], { lng: "en" })
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
