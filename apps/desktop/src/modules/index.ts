import { Box, Puzzle, type LucideIcon } from "lucide-react"

import type { ModuleInfo } from "@/lib/types"

import { projectsModule } from "./projects"
import type { FrontendModule } from "./types"

/** Every module with a user interface. Modules without one need no entry. */
const FRONTEND_MODULES: FrontendModule[] = [projectsModule]

/** Page id used in navigation for a module page, e.g. `projects/projects`. */
export type ModulePageId = `${string}/${string}`

export function modulePageId(module: string, page: string): ModulePageId {
  return `${module}/${page}`
}

export interface ModuleNavItem {
  id: ModulePageId
  label: string
  icon: LucideIcon
  module: string
}

function frontend(id: string) {
  return FRONTEND_MODULES.find((m) => m.id === id)
}

/** Navigation entries of enabled modules, in the order their manifests declare. */
export function moduleNavigation(modules: ModuleInfo[]): ModuleNavItem[] {
  return modules
    .filter((m) => m.enabled)
    .flatMap((m) =>
      m.pages.filter((page) => frontend(m.id)?.pages[page.id]).map((page) => ({ module: m, page })),
    )
    .sort((a, b) => a.page.order - b.page.order || a.module.id.localeCompare(b.module.id))
    .map(({ module, page }) => ({
      id: modulePageId(module.id, page.id),
      label: page.title,
      icon: frontend(module.id)?.pages[page.id]?.icon ?? Box,
      module: module.id,
    }))
}

/** Views of every module page, keyed by navigation id. */
export const MODULE_VIEWS: Record<string, FrontendModule["pages"][string]["view"]> =
  Object.fromEntries(
    FRONTEND_MODULES.flatMap((m) =>
      Object.entries(m.pages).map(([page, { view }]) => [modulePageId(m.id, page), view]),
    ),
  )

/** Whether a module page exists and its module is enabled. */
export function isModulePageAvailable(modules: ModuleInfo[], id: string): boolean {
  const [moduleId, pageId] = id.split("/")
  const module = modules.find((m) => m.id === moduleId && m.enabled)
  return Boolean(module?.pages.some((p) => p.id === pageId) && id in MODULE_VIEWS)
}

/** The view of a module window, by identifier. */
export function moduleWindowView(id: string) {
  const module = FRONTEND_MODULES.find((m) => id.startsWith(`${m.id}.`))
  return module?.windows?.[id] ?? null
}

/** Dashboard cards of enabled modules. */
export function dashboardCards(modules: ModuleInfo[]) {
  return modules
    .filter((m) => m.enabled)
    .flatMap((m) =>
      (frontend(m.id)?.dashboardCards ?? []).map((card, i) => ({ key: `${m.id}:${i}`, card })),
    )
}

/** Identifier of a module's settings page: `module:<module id>`. */
export function moduleSettingsId(module: string) {
  return `module:${module}`
}

/** The icon of a module in the settings. */
export function moduleIcon(module: string): LucideIcon {
  return frontend(module)?.icon ?? Puzzle
}

export interface ModuleSettingsPage {
  id: string
  module: ModuleInfo
  icon: LucideIcon
  /** The module's own view; `null`: generated from its manifest settings. */
  view: FrontendModule["settings"] | null
}

/**
 * Settings pages of enabled modules: their own view, or one generated from
 * the settings their manifest declares. Modules without either have none.
 */
export function moduleSettingsPages(modules: ModuleInfo[]): ModuleSettingsPage[] {
  return modules
    .filter((m) => m.enabled && m.supported && (m.config.length > 0 || frontend(m.id)?.settings))
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((module) => ({
      id: moduleSettingsId(module.id),
      module,
      icon: moduleIcon(module.id),
      view: frontend(module.id)?.settings ?? null,
    }))
}
