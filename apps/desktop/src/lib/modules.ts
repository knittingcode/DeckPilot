import { currentLanguage, i18n } from "@/i18n"
import { formatList } from "@/i18n/format"

import type { ActionType, ModuleInfo } from "./types"

/** The module that provides an action type, enabled or not. */
export function moduleOf(modules: ModuleInfo[], typeId: string): ModuleInfo | undefined {
  return modules.find((m) => m.actionTypes.some((t) => t.id === typeId))
}

/**
 * The types offered for new actions and workflow nodes: deprecated types
 * are left out, while saved actions of them keep working and stay editable.
 */
export function creatableTypes(types: ActionType[]): ActionType[] {
  return types.filter((t) => !t.deprecated)
}

/** Whether the user can switch a module on or off now, and why not. */
export interface ModuleToggle {
  allowed: boolean
  /** Explains a disabled switch, in the window's language; `null` when switching is allowed. */
  reason: string | null
}

function names(modules: ModuleInfo[], ids: string[]) {
  return formatList(
    currentLanguage(),
    ids.map((id) => modules.find((m) => m.id === id)?.name ?? id),
  )
}

/** Mirrors the rules the backend enforces when a module is enabled or disabled. */
export function moduleToggle(module: ModuleInfo, modules: ModuleInfo[]): ModuleToggle {
  if (module.enabled) {
    const dependents = modules.filter((m) => m.enabled && m.dependencies.includes(module.id))
    if (dependents.length > 0) {
      return {
        allowed: false,
        reason: i18n.t("settings:modules.toggle.requiredBy", {
          count: dependents.length,
          modules: names(
            modules,
            dependents.map((m) => m.id),
          ),
        }),
      }
    }
    return { allowed: true, reason: null }
  }
  if (!module.supported) {
    return { allowed: false, reason: i18n.t("settings:modules.toggle.unsupported") }
  }
  const missing = module.dependencies.filter((id) => !modules.some((m) => m.id === id))
  if (missing.length > 0) {
    return {
      allowed: false,
      reason: i18n.t("settings:modules.toggle.notInstalled", {
        count: missing.length,
        modules: formatList(currentLanguage(), missing),
      }),
    }
  }
  const disabled = module.dependencies.filter(
    (id) => !modules.some((m) => m.id === id && m.enabled),
  )
  if (disabled.length > 0) {
    return {
      allowed: false,
      reason: i18n.t("settings:modules.toggle.requires", {
        count: disabled.length,
        modules: names(modules, disabled),
      }),
    }
  }
  return { allowed: true, reason: null }
}

export type ModuleFilter = "all" | "enabled" | "disabled" | "unavailable"

export function moduleMatches(module: ModuleInfo, filter: ModuleFilter, query: string): boolean {
  const state =
    filter === "all" ||
    (filter === "enabled" && module.enabled) ||
    (filter === "disabled" && !module.enabled && module.supported) ||
    (filter === "unavailable" && !module.supported)
  if (!state) return false
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const text = [
    module.id,
    module.name,
    module.description,
    ...module.actionTypes.map((t) => t.name),
    ...module.pages.map((p) => p.title),
  ]
    .join(" ")
    .toLowerCase()
  return words.every((word) => text.includes(word))
}
