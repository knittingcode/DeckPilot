import { createContext, useContext } from "react"

import type { ModulePageId } from "@/modules"

import type { SettingsPageId } from "./navigation"

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
  /** Opens a workspace page; leaves the settings if they are open. */
  navigate: (page: PageId) => void
  /** Opens the settings, at `page` or where they were left. */
  openSettings: (page?: SettingsPageId) => void
  /** Returns from the settings to the workspace page open before. */
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
}

/** Pages of the core application. */
export type CorePageId = "dashboard" | "actions" | "workflows" | "profiles" | "designer"

/** A core page or a page contributed by a module (`module/page`). */
export type PageId = CorePageId | ModulePageId

export const AppContext = createContext<AppContextValue | null>(null)

export function useAppContext(): AppContextValue {
  const value = useContext(AppContext)
  if (!value) throw new Error("useAppContext must be used inside AppContext")
  return value
}
