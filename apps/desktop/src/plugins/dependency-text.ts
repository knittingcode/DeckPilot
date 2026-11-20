// Wording and decisions of the dependency screens (dependencies.tsx), kept
// apart from the components.

import { i18n } from "@/i18n"

import type { PluginDependencyInfo, PluginRemovalPlan } from "./types"

/** How a dependency stands here, in a few words. */
export function dependencyStateText(d: PluginDependencyInfo): string {
  switch (d.state) {
    case "satisfied":
      return d.installedVersion
        ? i18n.t("plugins:dependencies.state.installed", { version: d.installedVersion })
        : i18n.t("plugins:dependencies.state.available")
    case "disabled":
      return d.installedVersion
        ? i18n.t("plugins:dependencies.state.installedOff", { version: d.installedVersion })
        : i18n.t("plugins:dependencies.state.off")
    case "wrongVersion":
      return d.installedVersion
        ? i18n.t("plugins:dependencies.state.wrongVersion", {
            version: d.installedVersion,
            needed: d.version,
          })
        : i18n.t("plugins:dependencies.state.otherVersion", { needed: d.version })
    case "missing":
      return d.builtin
        ? i18n.t("plugins:dependencies.state.notInDeckPilot")
        : i18n.t("plugins:dependencies.state.notInstalled")
    case "unusable":
      return i18n.t("plugins:dependencies.state.unusable")
  }
}

/** Whether a removal plan needs the user to see it before it happens. */
export function removalNeedsReview(plan: PluginRemovalPlan): boolean {
  return (
    (plan.disables ?? []).length > 0 ||
    (plan.usedBy ?? []).some((d) => d.enabled) ||
    (plan.orphans ?? []).length > 0
  )
}
