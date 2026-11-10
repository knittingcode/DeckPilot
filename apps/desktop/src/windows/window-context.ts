import { createContext, useContext } from "react"

import type { WindowSpec } from "@/lib/types"

export interface WindowContextValue {
  spec: WindowSpec
  /** Closes this window. Background work it started keeps running. */
  close: () => void
}

export const WindowContext = createContext<WindowContextValue | null>(null)

export function useWindow(): WindowContextValue {
  const value = useContext(WindowContext)
  if (!value) throw new Error("useWindow must be used inside a native window")
  return value
}
