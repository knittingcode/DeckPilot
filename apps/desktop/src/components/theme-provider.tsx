import { useEffect } from "react"

import { LEGACY_THEME_STORAGE_KEY } from "@/lib/theme"

/**
 * Applies DeckPilot's permanent dark appearance to the document: the
 * `dark` class (the tokens in `index.css`) and `color-scheme: dark` (native
 * form controls, scrollbars). `index.html` sets both already, so this only
 * keeps them in place; the operating system's appearance is never read.
 * An obsolete light/dark/system preference of an earlier version is
 * removed (it is only a display preference, no configuration data).
 */
export function ThemeProvider({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    const root = document.documentElement
    root.classList.add("dark")
    root.style.colorScheme = "dark"
    try {
      localStorage.removeItem(LEGACY_THEME_STORAGE_KEY)
    } catch {
      // Storage unavailable: nothing to clean up.
    }
  }, [])
  return children
}
