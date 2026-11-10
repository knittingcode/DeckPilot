import { createContext, useContext } from "react"

import type { AgentConnection, ProfilesView, UpdateState } from "./types"

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
}

export type PageId =
  | "dashboard"
  | "actions"
  | "workflows"
  | "profiles"
  | "projects"
  | "models"
  | "designer"
  | "devices"
  | "settings"

export const AppContext = createContext<AppContextValue | null>(null)

export function useAppContext(): AppContextValue {
  const value = useContext(AppContext)
  if (!value) throw new Error("useAppContext must be used inside AppContext")
  return value
}
