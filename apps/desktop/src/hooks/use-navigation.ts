import { useEffect, useMemo, useRef, useState } from "react"

import {
  type Location,
  type NavigationState,
  browserStorage,
  closeSettings,
  current,
  initialNavigation,
  loadNavigation,
  locationOf,
  openSettings,
  push,
  sanitize,
  saveNavigation,
} from "@/lib/navigation"
import { rememberSettingsPage, rememberedSettingsPage } from "@/settings/remembered-page"

export const IS_MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent)

/** Keyboard shortcuts of the main window, as shown in tooltips. */
export const SHORTCUTS = IS_MAC ? { settings: "⌘," } : { settings: "Ctrl+," }

/**
 * The navigation of the main window: the current location, kept across
 * launches, and the settings shortcut. There is no back and forward: the
 * rail, the selectors and the toolbar's commands lead everywhere. `exists`
 * tells which locations are valid; others are dropped, for
 * example after a module was disabled.
 *
 * The settings are a view of the main window: their pages are locations in
 * the same history as the workspace pages. Opening the settings without a
 * page returns to the settings page shown last, also across launches.
 */
export function useNavigation(exists: ((location: Location) => boolean) | null) {
  const [stored, setStored] = useState<NavigationState>(
    () => loadNavigation(browserStorage()) ?? initialNavigation(),
  )

  // Locations that stopped existing are dropped when the state is used,
  // and before every change.
  const existsRef = useRef(exists)
  useEffect(() => {
    existsRef.current = exists
  }, [exists])
  const valid = useMemo(() => (exists ? sanitize(stored, exists) : stored), [stored, exists])

  useEffect(() => {
    saveNavigation(browserStorage(), valid)
    const location = current(valid)
    if (location.mode === "settings") rememberSettingsPage(browserStorage(), location.page)
  }, [valid])

  const actions = useMemo(() => {
    const setState = (change: (state: NavigationState) => NavigationState) => {
      setStored((previous) => {
        const exists = existsRef.current
        return change(exists ? sanitize(previous, exists) : previous)
      })
    }
    const go = (location: Location) => {
      setState((previous) => push(previous, location))
    }
    return {
      // Old page names (e.g. `models`) lead to where the page moved.
      navigate: (page: string) => {
        go(locationOf(page))
      },
      go,
      openSettings: (page?: string) => {
        const remembered = rememberedSettingsPage(browserStorage())
        setState((previous) => openSettings(previous, page, remembered))
      },
      closeSettings: () => {
        setState(closeSettings)
      },
    }
  }, [])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const command = IS_MAC ? event.metaKey : event.ctrlKey
      if (command && !event.shiftKey && !event.altKey && event.key === ",") {
        event.preventDefault()
        actions.openSettings()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => {
      window.removeEventListener("keydown", onKey)
    }
  }, [actions])

  return useMemo(
    () => ({
      location: current(valid),
      /** The workspace page settings return to (and that stays mounted under them). */
      workspace: valid.workspace,
      ...actions,
    }),
    [valid, actions],
  )
}
