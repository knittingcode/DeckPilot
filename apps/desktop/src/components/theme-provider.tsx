import { useCallback, useEffect, useMemo, useState } from "react"

import { THEME_STORAGE_KEY, ThemeContext, type ThemePreference } from "@/lib/theme"
import { THEME_EVENT, announce, onAnnouncement } from "@/lib/window-sync"

function isPreference(value: unknown): value is ThemePreference {
  return value === "light" || value === "dark" || value === "system"
}

function readPreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY)
    if (isPreference(stored)) return stored
  } catch {
    // Storage can be unavailable; fall back to the system theme.
  }
  return "system"
}

function systemDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches
}

/**
 * The theme of a window. A choice made in one window (Settings ›
 * Appearance in the settings window) applies to every open window: through
 * the shared storage's `storage` event where the webviews share it, and
 * through an app event where they do not.
 */
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

  // Choices made in other windows.
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === THEME_STORAGE_KEY && isPreference(event.newValue)) {
        setPreferenceState(event.newValue)
      }
    }
    window.addEventListener("storage", onStorage)
    const stop = onAnnouncement(THEME_EVENT, ({ preference: next }) => {
      if (isPreference(next)) setPreferenceState(next)
    })
    return () => {
      window.removeEventListener("storage", onStorage)
      stop()
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
      localStorage.setItem(THEME_STORAGE_KEY, next)
    } catch {
      // Not persisted; the choice still applies for this session.
    }
    announce(THEME_EVENT, { preference: next })
  }, [])

  const value = useMemo(
    () => ({ preference, resolved, setPreference }),
    [preference, resolved, setPreference],
  )
  return <ThemeContext value={value}>{children}</ThemeContext>
}
