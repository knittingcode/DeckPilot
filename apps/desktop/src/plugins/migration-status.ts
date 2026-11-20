// What a module migration means for the user, in words (the migration
// notice, `migration.tsx`). Never a value: the agent's reasons name
// settings by key and credentials by id.

import { i18n } from "@/i18n"

import type { ModuleMigration } from "./types"

export type MigrationTone = "moving" | "waiting" | "builtin" | "moved" | "unavailable"

export interface MigrationStatus {
  tone: MigrationTone
  /** The headline: "Moving to a plugin", "Waiting for your consent", "Staying built-in", … */
  label: string
  /** What it means for the module's actions. */
  text: string
}

/** What a migration means for the user, in the window's language. */
export function migrationStatus(m: ModuleMigration): MigrationStatus {
  const plugin = m.pluginName ? `${m.pluginName} (${m.plugin})` : m.plugin
  const module = m.moduleName
  switch (m.phase) {
    case "pending":
    case "installed":
    case "settingsCopied":
      return {
        tone: "moving",
        label: i18n.t("plugins:migration.status.moving.label"),
        text: i18n.t("plugins:migration.status.moving.text", { module, plugin }),
      }
    case "awaitingConsent":
      return {
        tone: "waiting",
        label: i18n.t("plugins:migration.status.waiting.label"),
        text: i18n.t("plugins:migration.status.waiting.text", { module, plugin }),
      }
    case "cutOver":
      if (m.route === "pluginWithdrawn") {
        return {
          tone: "unavailable",
          label: i18n.t("plugins:migration.status.unavailable.label"),
          text: i18n.t("plugins:migration.status.unavailable.text", {
            module,
            plugin,
            reason: m.routeReason ?? i18n.t("plugins:migration.status.unavailable.removed"),
          }),
        }
      }
      return {
        tone: "moved",
        label: i18n.t("plugins:migration.status.moved.label"),
        text: i18n.t("plugins:migration.status.moved.text", { module, plugin }),
      }
    case "failed":
      return {
        tone: "builtin",
        label: i18n.t("plugins:migration.status.builtin.label"),
        text:
          m.attempts >= m.maxAttempts
            ? i18n.t("plugins:migration.status.failed.textFinal", { plugin, count: m.attempts })
            : i18n.t("plugins:migration.status.failed.text", { plugin }),
      }
    case "notEligible":
      return {
        tone: "builtin",
        label: i18n.t("plugins:migration.status.builtin.label"),
        text: i18n.t("plugins:migration.status.notEligible.text", { module }),
      }
    case "other":
      return {
        tone: "builtin",
        label: i18n.t("plugins:migration.status.other.label"),
        text: i18n.t("plugins:migration.status.other.text", { module }),
      }
  }
}

/** Whether the user declined something this migration asked for. */
export function isDeclined(m: ModuleMigration): boolean {
  return (m.consent ?? []).some((item) => item.decision === "declined")
}
