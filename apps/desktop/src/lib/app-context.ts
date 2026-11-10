import { createContext, useContext } from "react"

import type { ModulePageId } from "@/modules"

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
  navigate: (page: PageId) => void
  /** Registered feature modules and their state. */
  modules: ModuleInfo[]
  setModules: (modules: ModuleInfo[]) => void
}

/** Pages of the core application. */
export type CorePageId =
  | "dashboard"
  | "actions"
  | "workflows"
  | "profiles"
  | "models"
  | "designer"
  | "devices"
  | "settings"

/** A core page or a page contributed by a module (`module/page`). */
export type PageId = CorePageId | ModulePageId

export const AppContext = createContext<AppContextValue | null>(null)

export function useAppContext(): AppContextValue {
  const value = useContext(AppContext)
  if (!value) throw new Error("useAppContext must be used inside AppContext")
  return value
}
