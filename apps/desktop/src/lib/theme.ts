import { createContext, useContext } from "react"

/** Where the preference is kept; shared by every window of the app. */
export const THEME_STORAGE_KEY = "deckpilot.theme"

export type ThemePreference = "system" | "light" | "dark"

export interface ThemeContextValue {
  preference: ThemePreference
  resolved: "light" | "dark"
  setPreference: (preference: ThemePreference) => void
}

export const ThemeContext = createContext<ThemeContextValue | null>(null)

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext)
  if (!value) throw new Error("useTheme must be used inside ThemeProvider")
  return value
}
