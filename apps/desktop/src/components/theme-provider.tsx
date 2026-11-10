import { useCallback, useEffect, useMemo, useState } from "react"

import { ThemeContext, type ThemePreference } from "@/lib/theme"

const STORAGE_KEY = "deckpilot.theme"

function readPreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored === "light" || stored === "dark" || stored === "system") return stored
  } catch {
    // Storage can be unavailable; fall back to the system theme.
  }
  return "system"
}

function systemDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>(readPreference)
  const [dark, setDark] = useState(systemDark)

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)")
    const listener = () => {
      setDark(media.matches)
    }
    media.addEventListener("change", listener)
    return () => {
      media.removeEventListener("change", listener)
    }
  }, [])

  const resolved = preference === "system" ? (dark ? "dark" : "light") : preference

  useEffect(() => {
    document.documentElement.classList.toggle("dark", resolved === "dark")
    document.documentElement.style.colorScheme = resolved
  }, [resolved])

  const setPreference = useCallback((next: ThemePreference) => {
    setPreferenceState(next)
    try {
      localStorage.setItem(STORAGE_KEY, next)
    } catch {
      // Not persisted; the choice still applies for this session.
    }
  }, [])

  const value = useMemo(
    () => ({ preference, resolved, setPreference }),
    [preference, resolved, setPreference],
  )
  return <ThemeContext value={value}>{children}</ThemeContext>
}
