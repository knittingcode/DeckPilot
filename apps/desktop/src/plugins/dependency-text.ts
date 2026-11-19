// Wording and decisions of the dependency screens (dependencies.tsx), kept
// apart from the components.

import type { PluginDependencyInfo, PluginRemovalPlan } from "./types"

/** How a dependency stands here, in a few words. */
export function dependencyStateText(d: PluginDependencyInfo): string {
  switch (d.state) {
    case "satisfied":
      return d.installedVersion ? `Installed ${d.installedVersion}` : "Available"
    case "disabled":
      return d.installedVersion ? `Installed ${d.installedVersion}, turned off` : "Turned off"
    case "wrongVersion":
      return `Installed ${d.installedVersion ?? "in another version"}; needs ${d.version}`
    case "missing":
      return d.builtin ? "Not available in this DeckPilot" : "Not installed"
    case "unusable":
      return "Installed, but cannot be used"
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
