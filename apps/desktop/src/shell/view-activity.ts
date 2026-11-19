import { createContext, useContext } from "react"

/**
 * Whether the view around a component is the one on screen. The main window
 * keeps the workspace page mounted, hidden, while the settings are open, so
 * that the device editor comes back exactly as it was (page, key,
 * inspector). Window-wide handlers of a hidden view, such as editing
 * shortcuts, must stay quiet meanwhile.
 */
export const ViewActiveContext = createContext(true)

export function useViewActive() {
  return useContext(ViewActiveContext)
}
