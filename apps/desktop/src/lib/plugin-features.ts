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
// have plugins at all, so the features keep their pre-plugin behaviour
// there, exactly like the agent (`Agent::project_context_active`): the
// Workflows screen is always there, Projects follows its legacy module.

import { useMemo } from "react"

import { type AppContextValue, useAppContext } from "./app-context"
import { pluginsEnabled } from "./plugin-actions"
import type { ModuleInfo } from "./types"

export const WORKFLOWS_PLUGIN = "deckpilot.workflows"
export const PROJECTS_PLUGIN = "deckpilot.projects"

/** The built-in module Projects had before its plugin. */
export const LEGACY_PROJECTS_MODULE = "projects"

/**
 * Built-in modules whose user interface moved to a first-party plugin, by
 * module id. In builds with plugins their settings pages and module cards
 * are not shown: the plugin's own (Settings › Plugins) replace them.
 */
export const EXTRACTED_MODULES: Readonly<Record<string, string>> = {
  git: "deckpilot.git",
  docker: "deckpilot.docker",
  workspace: "deckpilot.workspace",
  github: "deckpilot.github",
  gitlab: "deckpilot.gitlab",
  jira: "deckpilot.jira",
  projects: PROJECTS_PLUGIN,
}

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
   * Whether the plugin is installed and enabled. `deckpilot.projects` is
   * also active while its legacy module provides the project context (not
   * migrated yet), and in builds without plugins `deckpilot.workflows` and
   * `deckpilot.projects` answer as the core did before them.
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

/** The legacy Projects module provides the project context. */
function legacyProjects(modules: ModuleInfo[], plugins: ModuleInfo[], withPlugins: boolean) {
  const builtin = modules.find((m) => m.id === LEGACY_PROJECTS_MODULE)
  if (!builtin?.enabled || !builtin.supported) return false
  if (!withPlugins) return true
  // Only while the module is not migrated: once the plugin took over (or
  // the user withdrew it), the plugin alone decides.
  const route =
    plugins.find((p) => p.id === PROJECTS_PLUGIN)?.plugin?.route ??
    plugins.flatMap((p) => p.plugin?.aliases ?? []).find((a) => a.module === LEGACY_PROJECTS_MODULE)
      ?.route
  return route === undefined || route === "builtin"
}

/**
 * The features of `modules` (the built-in modules) and `plugins` (one
 * module per installed plugin, `usePluginModules`).
 */
export function pluginFeatures(
  modules: ModuleInfo[],
  plugins: ModuleInfo[],
  withPlugins: boolean = pluginsEnabled(),
): PluginFeatures {
  const enabled = new Set(
    withPlugins ? plugins.filter((p) => p.plugin?.state === "enabled").map((p) => p.id) : [],
  )
  const projectsLegacy = legacyProjects(modules, plugins, withPlugins)
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
  // Without plugins (or before Projects is migrated) the core shows what it
  // showed before the plugins.
  if (!withPlugins) hostViews.add("workflows")
  if (projectsLegacy) {
    hostViews.add("projects")
    statusItems.add("activeProject")
    dashboardCards.add("activeProject")
  }
  const isPluginActive = (id: string) => {
    if (!withPlugins) return id === WORKFLOWS_PLUGIN || (id === PROJECTS_PLUGIN && projectsLegacy)
    return enabled.has(id) || (id === PROJECTS_PLUGIN && projectsLegacy)
  }
  return { isPluginActive, hostViews, statusItems, dashboardCards }
}

/**
 * The features as the main window sees them now, following every plugin
 * change. Outside the main window (no app context) every feature is on:
 * those windows only show what the main window opened.
 */
export function usePluginFeatures(): PluginFeatures {
  const app = useOptionalAppContext()
  const modules = app?.modules
  const plugins = app?.pluginModules
  return useMemo(
    () => (modules ? pluginFeatures(modules, plugins ?? []) : ALL_ACTIVE),
    [modules, plugins],
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

/** Whether a built-in module's UI moved to a plugin (and so is not shown here). */
export function isExtractedModule(id: string, withPlugins: boolean = pluginsEnabled()): boolean {
  return withPlugins && Object.hasOwn(EXTRACTED_MODULES, id)
}
