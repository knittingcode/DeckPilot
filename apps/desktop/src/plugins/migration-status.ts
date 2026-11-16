// What a module migration means for the user, in words (the migration
// notice, `migration.tsx`). Never a value: the agent's reasons name
// settings by key and credentials by id.

import type { ModuleMigration } from "./types"

export type MigrationTone = "moving" | "waiting" | "builtin" | "moved" | "unavailable"

export interface MigrationStatus {
  tone: MigrationTone
  /** The headline: "Moving to a plugin", "Waiting for your consent", "Staying built-in", … */
  label: string
  /** What it means for the module's actions. */
  text: string
}

/** What a migration means for the user. */
export function migrationStatus(m: ModuleMigration): MigrationStatus {
  const plugin = m.pluginName ? `${m.pluginName} (${m.plugin})` : m.plugin
  switch (m.phase) {
    case "pending":
    case "installed":
    case "settingsCopied":
      return {
        tone: "moving",
        label: "Moving to a plugin",
        text: `${m.moduleName} is moving to the plugin ${plugin}. The built-in keeps running its actions until the move is done.`,
      }
    case "awaitingConsent":
      return {
        tone: "waiting",
        label: "Waiting for your consent",
        text: `${m.moduleName} can move to the plugin ${plugin} once you allow what the plugin needs. Until then the built-in keeps running its actions.`,
      }
    case "cutOver":
      if (m.route === "pluginWithdrawn") {
        return {
          tone: "unavailable",
          label: "Actions unavailable",
          text: `${m.moduleName}'s actions run in ${plugin}, which is not available: ${m.routeReason ?? "it was removed"}.`,
        }
      }
      return {
        tone: "moved",
        label: "Moved to a plugin",
        text: `${m.moduleName}'s actions run in the plugin ${plugin}; your actions keep working unchanged.`,
      }
    case "failed":
      return {
        tone: "builtin",
        label: "Staying built-in",
        text:
          m.attempts >= m.maxAttempts
            ? `The move to ${plugin} failed ${String(m.attempts)} times; DeckPilot tries again only when you press Retry. The built-in keeps running its actions.`
            : `The move to ${plugin} failed; DeckPilot tries again at its next start. The built-in keeps running its actions.`,
      }
    case "notEligible":
      return {
        tone: "builtin",
        label: "Staying built-in",
        text: `${m.moduleName} stays built-in on this computer; its actions keep working.`,
      }
    case "other":
      return {
        tone: "builtin",
        label: "Unknown state",
        text: `The agent reports a state this app does not know; ${m.moduleName}'s actions are unchanged.`,
      }
  }
}

/** Whether the user declined something this migration asked for. */
export function isDeclined(m: ModuleMigration): boolean {
  return (m.consent ?? []).some((item) => item.decision === "declined")
}
