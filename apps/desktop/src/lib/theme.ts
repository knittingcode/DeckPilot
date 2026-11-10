import { createContext, useContext } from "react"

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
