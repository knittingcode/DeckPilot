import { createContext, useContext } from "react"

import type { SecurityView } from "./types"

export interface SecurityContextValue {
  view: SecurityView
  /** Updates the state, e.g. after locking; the gate reacts to it. */
  setView: (view: SecurityView) => void
  refresh: () => void
}

export const SecurityContext = createContext<SecurityContextValue | null>(null)

export function useSecurity(): SecurityContextValue {
  const value = useContext(SecurityContext)
  if (!value) throw new Error("useSecurity must be used inside SecurityGate")
  return value
}
