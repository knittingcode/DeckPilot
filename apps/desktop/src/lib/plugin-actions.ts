// Plugin actions in the action editors (experimental).
//
// With `VITE_DECKPILOT_PLUGINS=1` every installed plugin the agent lists
// becomes a module of the editors: its id is the plugin id
// (`<publisher>.<name>`), its action types are the definitions the agent's
// engine registers (`PluginActionInfo.definition`), and its per-action
// availability is the agent's. Nothing about plugins is decided here; the
// only grammar is the owner rule of `ModuleId::owner_of` in the core: an
// action type belongs to everything before its last `.`. Without the flag
// none of this runs and the editors see the built-in modules only.

import type { KnownPlugin, PluginSummary } from "@/plugins/types"

import type { ActionType, ModuleInfo } from "./types"

/** Builds with the experimental plugin marketplace (`VITE_DECKPILOT_PLUGINS=1`). */
export function pluginsEnabled(): boolean {
  return import.meta.env.VITE_DECKPILOT_PLUGINS === "1"
}

/**
 * The module an action type belongs to: everything before its **last**
 * `.` (`projects.next` → `projects`, `acme.hello-weather.greet` →
 * `acme.hello-weather`), as `ModuleId::owner_of` decides it.
 */
export function ownerOf(typeId: string): string | null {
  const split = typeId.lastIndexOf(".")
  return split > 0 ? typeId.slice(0, split) : null
}

/** A plugin id (`<publisher>.<name>`); built-in module ids never have a `.`. */
export function isPluginId(moduleId: string): boolean {
  return moduleId.includes(".")
}

/** Whether an action type is owned by a plugin. */
export function isPluginType(typeId: string): boolean {
  const owner = ownerOf(typeId)
  return owner !== null && isPluginId(owner)
}

/** `PluginModuleState.state` of a plugin that `plugin_known` has but is not installed. */
export const NOT_INSTALLED = "notInstalled"

/**
 * One module per installed plugin, from a `listPlugins` answer, and one
 * without action types per plugin that was installed here before
 * (`listKnownPlugins`), so that actions still using it name it.
 */
export function pluginModules(plugins: PluginSummary[], known: KnownPlugin[] = []): ModuleInfo[] {
  const installed = new Set(plugins.map((p) => p.id))
  const gone: ModuleInfo[] = known
    .filter((k) => !k.installed && !installed.has(k.id) && isPluginId(k.id))
    .map((k) => ({
      id: k.id,
      version: k.lastVersion,
      name: k.name || k.id,
      description: k.publisher ? `Plugin by ${k.publisher}` : "Plugin",
      platforms: [],
      pages: [],
      capabilities: [],
      config: [],
      dependencies: [],
      enabledByDefault: false,
      enabled: false,
      supported: true,
      configValues: {},
      actionTypes: [],
      plugin: { state: NOT_INSTALLED, unavailable: {} },
    }))
  return plugins
    .filter((plugin) => isPluginId(plugin.id))
    .map((plugin): ModuleInfo => {
      const actionTypes: ActionType[] = []
      const unavailable: Record<string, string> = {}
      for (const action of plugin.actions) {
        const definition = action.definition
        // Only definitions inside the plugin's own namespace.
        if (!definition || definition.id !== action.actionType) continue
        if (ownerOf(definition.id) !== plugin.id) continue
        actionTypes.push(definition)
        if (!action.available) {
          unavailable[definition.id] =
            action.unavailableReason ?? plugin.stateReason ?? `the plugin is ${plugin.state}`
        }
      }
      return {
        id: plugin.id,
        version: plugin.version,
        name: plugin.name || plugin.id,
        description: plugin.publisher ? `Plugin by ${plugin.publisher}` : "Plugin",
        platforms: [],
        pages: [],
        capabilities: [],
        config: [],
        dependencies: [],
        enabledByDefault: false,
        enabled: actionTypes.some((t) => !(t.id in unavailable)),
        supported: true,
        configValues: {},
        actionTypes,
        plugin: { state: plugin.state, unavailable },
      }
    })
    .concat(gone)
}

/** The built-in modules followed by the plugin modules; `modules` itself without plugins. */
export function withPluginModules(modules: ModuleInfo[], plugins: ModuleInfo[]): ModuleInfo[] {
  return plugins.length === 0 ? modules : [...modules, ...plugins]
}

/** Why a plugin module's action type cannot be placed or run; `null` when it can. */
export function pluginTypeUnavailable(module: ModuleInfo, typeId: string): string | null {
  return module.plugin?.unavailable[typeId] ?? null
}

/**
 * The action types a workflow can use: the backend's list with the plugin
 * types replaced by the ones available now. Without plugin modules (the
 * flag is off, or nothing is installed) the list is returned unchanged.
 */
export function withPluginTypes(types: ActionType[], modules: ModuleInfo[]): ActionType[] {
  if (!pluginsEnabled()) return types
  const available = modules
    .filter((m) => m.plugin)
    .flatMap((m) => m.actionTypes.filter((t) => !pluginTypeUnavailable(m, t.id)))
  if (available.length === 0 && !types.some((t) => isPluginType(t.id))) return types
  return [...types.filter((t) => !isPluginType(t.id)), ...available]
}

/**
 * Why an action of `typeId` cannot be newly placed (on a key, a gesture, a
 * workflow step) because of its plugin: disabled, broken, quarantined, a
 * permission not granted, or not installed at all. `null` for built-in
 * types, in builds without plugins, and for plugin actions that can run.
 * Places that already use such an action keep it and show this reason.
 */
export function pluginActionUnavailable(modules: ModuleInfo[], typeId: string): string | null {
  if (!pluginsEnabled() || !isPluginType(typeId)) return null
  const availability = typeAvailability(modules, typeId)
  return availability.available
    ? null
    : (availability.reason ?? `The plugin ${ownerOf(typeId) ?? ""} is not available`)
}

/** Whether an action of `typeId` can be edited and run now, and why not. */
export interface TypeAvailability {
  available: boolean
  /** The module (or plugin module) that provides the type. */
  owner: ModuleInfo | undefined
  /**
   * What the user can read about it: why a plugin action is unavailable,
   * or that its plugin is not installed. `null` for built-in modules, whose
   * messages the editors word themselves.
   */
  reason: string | null
}

/**
 * The availability of an action type for the editors. For built-in types
 * it is what it always was: available while its module is enabled. A
 * plugin type is available when the agent says so; one whose plugin is not
 * installed (any more) keeps its instances, which say so.
 */
export function typeAvailability(modules: ModuleInfo[], typeId: string): TypeAvailability {
  const owner =
    modules.find((m) => m.actionTypes.some((t) => t.id === typeId)) ??
    (pluginsEnabled() && isPluginType(typeId)
      ? modules.find((m) => m.plugin && m.id === ownerOf(typeId))
      : undefined)
  if (owner?.plugin?.state === NOT_INSTALLED) {
    // The agent's words for it: `Hello Weather (acme.hello-weather): the plugin is not installed`.
    return {
      available: false,
      owner,
      reason: `${owner.name} (${owner.id}): the plugin is not installed`,
    }
  }
  if (owner?.plugin) {
    const known = owner.actionTypes.some((t) => t.id === typeId)
    const why = known
      ? pluginTypeUnavailable(owner, typeId)
      : `${owner.name} (${owner.id}) no longer provides this action`
    return {
      available: why === null,
      owner,
      reason: why === null ? null : known ? `${owner.name} (${owner.id}): ${why}` : why,
    }
  }
  if (!owner && pluginsEnabled() && isPluginType(typeId)) {
    return {
      available: false,
      owner: undefined,
      reason: `The plugin ${ownerOf(typeId) ?? ""} is not installed`,
    }
  }
  return { available: Boolean(owner?.enabled), owner, reason: null }
}
