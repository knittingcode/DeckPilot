// Which optional features the plugins currently provide
// (docs/marketplace/module-migration.md §11, D1 and D6).
//
// Workflows management and Projects are first-party plugins. Plugins ship
// no UI code: their screens, the active project's status item and its
// dashboard card stay in this app and are drawn only while the plugin that
// activates them (`contributes.views` of kind `workflows`/`projects`,
// `statusItems`, `dashboardCards`, permission `hostViews`) is enabled. The
// answer follows the live plugin list (`usePluginModules`, read again on
// every plugin change), so enabling or disabling a plugin shows or hides
// its UI at once. Nothing is ever deleted: with a plugin off, its data
// stays in the profile and keys that use it keep running.
//
// Builds without the plugin runtime (no `VITE_DECKPILOT_PLUGINS=1`) cannot
// have plugins at all: the Workflows screen keeps its pre-plugin behaviour
// there and is always shown. Projects have no provider without their
// plugin (the built-in `projects` module is gone, WP-C6), exactly like the
// agent (`Agent::project_context_active`).

import { useMemo } from "react"

import { type AppContextValue, useAppContext } from "./app-context"
import { pluginsEnabled } from "./plugin-actions"
import type { ModuleInfo } from "./types"

export const WORKFLOWS_PLUGIN = "deckpilot.workflows"
export const PROJECTS_PLUGIN = "deckpilot.projects"

/** The app's own screens a plugin can activate. */
export type HostViewKind = "workflows" | "projects"
/** Status-bar items the app draws for a plugin. */
export type HostStatusItem = "activeProject"
/** Dashboard cards the app draws for a plugin. */
export type HostDashboardCard = "activeProject"

/**
 * What each first-party plugin may activate. Anything else a plugin
 * lists is ignored, whoever sends it.
 */
const HOST_CONTRIBUTIONS: Record<
  string,
  { views: HostViewKind[]; statusItems: HostStatusItem[]; dashboardCards: HostDashboardCard[] }
> = {
  [WORKFLOWS_PLUGIN]: { views: ["workflows"], statusItems: [], dashboardCards: [] },
  [PROJECTS_PLUGIN]: {
    views: ["projects"],
    statusItems: ["activeProject"],
    dashboardCards: ["activeProject"],
  },
}

export interface PluginFeatures {
  /**
   * Whether the plugin is installed and enabled. In builds without plugins
   * `deckpilot.workflows` answers as the core did before it.
   */
  isPluginActive: (id: string) => boolean
  /** The app's own screens to show. */
  hostViews: ReadonlySet<HostViewKind>
  statusItems: ReadonlySet<HostStatusItem>
  dashboardCards: ReadonlySet<HostDashboardCard>
}

/** Every feature on: outside the main window, which has no plugin list. */
const ALL_ACTIVE: PluginFeatures = {
  isPluginActive: () => true,
  hostViews: new Set<HostViewKind>(["workflows", "projects"]),
  statusItems: new Set<HostStatusItem>(["activeProject"]),
  dashboardCards: new Set<HostDashboardCard>(["activeProject"]),
}

/**
 * The features of `plugins` (one module per installed plugin,
 * `usePluginModules`).
 */
export function pluginFeatures(
  plugins: ModuleInfo[],
  withPlugins: boolean = pluginsEnabled(),
): PluginFeatures {
  const enabled = new Set(
    withPlugins ? plugins.filter((p) => p.plugin?.state === "enabled").map((p) => p.id) : [],
  )
  const hostViews = new Set<HostViewKind>()
  const statusItems = new Set<HostStatusItem>()
  const dashboardCards = new Set<HostDashboardCard>()
  for (const plugin of plugins) {
    const allowed = HOST_CONTRIBUTIONS[plugin.id]
    if (!allowed || !enabled.has(plugin.id) || !plugin.plugin) continue
    const listed = plugin.plugin
    for (const view of allowed.views) if (listed.hostViews?.includes(view)) hostViews.add(view)
    for (const item of allowed.statusItems)
      if (listed.statusItems?.includes(item)) statusItems.add(item)
    for (const card of allowed.dashboardCards)
      if (listed.dashboardCards?.includes(card)) dashboardCards.add(card)
  }
  // Without plugins the core shows the Workflows screen it showed before.
  if (!withPlugins) hostViews.add("workflows")
  const isPluginActive = (id: string) => (withPlugins ? enabled.has(id) : id === WORKFLOWS_PLUGIN)
  return { isPluginActive, hostViews, statusItems, dashboardCards }
}

/**
 * The features as the main window sees them now, following every plugin
 * change. Outside the main window (no app context) every feature is on:
 * those windows only show what the main window opened.
 */
export function usePluginFeatures(): PluginFeatures {
  const app = useOptionalAppContext()
  const inMainWindow = app !== null
  const plugins = app?.pluginModules
  return useMemo(
    () => (inMainWindow ? pluginFeatures(plugins ?? []) : ALL_ACTIVE),
    [inMainWindow, plugins],
  )
}

/** The app context, or `null` outside the main window. */
function useOptionalAppContext(): AppContextValue | null {
  try {
    return useAppContext()
  } catch {
    return null
  }
}

/** Whether the plugin `id` is active now (see `PluginFeatures.isPluginActive`). */
export function useIsPluginActive(id: string): boolean {
  return usePluginFeatures().isPluginActive(id)
}
