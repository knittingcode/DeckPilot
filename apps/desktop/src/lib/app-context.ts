import { createContext, useContext, useMemo } from "react"

import type { ModulePageId } from "@/modules"

import type { SettingsPageId } from "./navigation"

import { withPluginModules } from "./plugin-actions"
import type { AgentConnection, ModuleInfo, ProfilesView, UpdateState } from "./types"

export interface AppContextValue {
  profiles: ProfilesView
  /** Profile shown in the editors; independent of the profile on the device. */
  editingProfileId: string | null
  setEditingProfileId: (id: string) => void
  /** Reloads data after a change; pages reload when `revision` changes. */
  refresh: () => void
  revision: number
  /** Latest known agent state; `null` until the first status arrives. */
  agent: AgentConnection | null
  setAgent: (agent: AgentConnection) => void
  updates: UpdateState | null
  setUpdates: (state: UpdateState) => void
  /** Opens a workspace page; settings pages open in the settings window. */
  navigate: (page: PageId) => void
  /** Opens (or focuses) the settings window, at `page` or where it was left. */
  openSettings: (page?: SettingsPageId) => void
  /** Leaves the settings for the workspace page that was open before. */
  closeSettings: () => void
  /** Browser-style history of the main window. */
  history: {
    canGoBack: boolean
    canGoForward: boolean
    back: () => void
    forward: () => void
  }
  /** Registered feature modules and their state. */
  modules: ModuleInfo[]
  setModules: (modules: ModuleInfo[]) => void
  /**
   * One module per installed plugin, as the agent lists them; only in
   * builds with `VITE_DECKPILOT_PLUGINS=1` (see `lib/plugin-actions.ts`).
   * Kept apart from `modules`: plugins have no settings, pages or switch.
   */
  pluginModules?: ModuleInfo[]
}

/** Pages of the core application. */
/** Pages of the core application; `profiles` opens Settings › Profiles. */
export type CorePageId =
  "dashboard" | "actions" | "workflows" | "profiles" | "designer" | "marketplace"

/** A core page or a page contributed by a module (`module/page`). */
export type PageId = CorePageId | ModulePageId

export const AppContext = createContext<AppContextValue | null>(null)

export function useAppContext(): AppContextValue {
  const value = useContext(AppContext)
  if (!value) throw new Error("useAppContext must be used inside AppContext")
  return value
}

/**
 * The modules that provide action types to the action and workflow
 * editors: the built-in ones, then (in builds with plugins) the installed
 * plugins. Exactly `modules` when there are no plugin modules.
 */
export function useActionModules(): ModuleInfo[] {
  const { modules, pluginModules } = useAppContext()
  return useMemo(() => withPluginModules(modules, pluginModules ?? []), [modules, pluginModules])
}
