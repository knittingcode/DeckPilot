// The catalog of the layout editor's library: action types that can be put
// on a control directly, without creating an action in the Action Library
// first. Dropping one creates the action with the type's default settings
// (or reuses an action that already is exactly that) and binds it.

import type { ActionInput } from "@/lib/api"
import { creatableTypes } from "@/lib/modules"
import type {
  ActionInstance,
  ActionParameters,
  ActionType,
  ModuleInfo,
  ParameterValue,
} from "@/lib/types"

export interface CatalogEntry {
  type: ActionType
  /** The module or plugin that provides it. */
  module: string
}

export interface CatalogGroup {
  /** The types' category, e.g. "Navigation". */
  label: string
  entries: CatalogEntry[]
}

/** The id of a catalog entry among the editor's draggables. */
export function catalogDragId(actionType: string): string {
  return `library:type:${actionType}`
}

/**
 * Whether an action of the type works without any input: every required
 * parameter has a default. Types that need a value (a URL, an application)
 * are created in the Action Library, where they are configured.
 */
export function needsNoInput(type: ActionType): boolean {
  return type.parameters.every((p) => !p.required || p.default !== undefined)
}

/**
 * The types the library offers for dropping on a control, by category:
 * those of enabled, supported modules that are not deprecated and need no
 * input. Categories and types keep the order of the modules.
 */
export function catalogGroups(modules: ModuleInfo[]): CatalogGroup[] {
  const groups = new Map<string, CatalogEntry[]>()
  for (const module of modules) {
    // Plugin actions are placed from the Action Library, where their
    // availability and permissions are shown.
    if (module.plugin || !module.enabled || !module.supported) continue
    for (const type of creatableTypes(module.actionTypes)) {
      if (!needsNoInput(type)) continue
      const entries = groups.get(type.category) ?? []
      entries.push({ type, module: module.name })
      groups.set(type.category, entries)
    }
  }
  return [...groups].map(([label, entries]) => ({ label, entries }))
}

/** The settings of a new action of the type: its declared defaults. */
export function defaultParameters(type: ActionType): ActionParameters {
  const parameters: ActionParameters = {}
  for (const spec of type.parameters) {
    if (spec.default !== undefined) parameters[spec.key] = spec.default
  }
  return parameters
}

function sameValue(a: ParameterValue | undefined, b: ParameterValue | undefined): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null)
}

/**
 * An action of the profile that is exactly a new action of the type would
 * be: the same type, the default settings and no label of its own. It is
 * reused, so dropping "Next page" on five keys leaves one action in the
 * Action Library, not five.
 */
export function existingDefault(
  actions: ActionInstance[],
  type: ActionType,
): ActionInstance | undefined {
  const defaults = defaultParameters(type)
  const keys = type.parameters.map((p) => p.key)
  return actions.find(
    (action) =>
      action.actionType === type.id &&
      !action.label &&
      keys.every((key) => sameValue(action.parameters[key], defaults[key])),
  )
}

/** A name no action of the profile has: the type's, then "… 2", "… 3". */
export function freeActionName(actions: ActionInstance[], name: string): string {
  const taken = new Set(actions.map((a) => a.name.trim().toLowerCase()))
  if (!taken.has(name.toLowerCase())) return name
  let n = 2
  while (taken.has(`${name} ${n}`.toLowerCase())) n += 1
  return `${name} ${n}`
}

/** What to create for a type dropped on a control. */
export function newAction(actions: ActionInstance[], type: ActionType): ActionInput {
  return {
    name: freeActionName(actions, type.name),
    actionType: type.id,
    parameters: defaultParameters(type),
    description: "",
    icon: type.node?.icon ?? null,
    label: null,
  }
}

/** Entries whose name, description, category or module contain `query`. */
export function filterCatalog(groups: CatalogGroup[], query: string): CatalogGroup[] {
  const needle = query.trim().toLowerCase()
  if (needle === "") return groups
  return groups
    .map((group) => ({
      ...group,
      entries: group.entries.filter((entry) =>
        `${entry.type.name} ${entry.type.description} ${group.label} ${entry.module}`
          .toLowerCase()
          .includes(needle),
      ),
    }))
    .filter((group) => group.entries.length > 0)
}
