import { createContext, useContext } from "react"

import type { ProfilesView } from "./types"

export interface AppContextValue {
  profiles: ProfilesView
  /** Reloads profile data after a change; pages also reload their own data. */
  refresh: () => void
  /** Incremented on every refresh so dependent views can reload. */
  revision: number
}

export const AppContext = createContext<AppContextValue | null>(null)

export function useAppContext(): AppContextValue {
  const value = useContext(AppContext)
  if (!value) throw new Error("useAppContext must be used inside AppContext")
  return value
}
