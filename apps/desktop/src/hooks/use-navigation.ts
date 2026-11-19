import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import {
  type Location,
  type NavigationState,
  back,
  browserStorage,
  canGoBack,
  canGoForward,
  closeSettings,
  current,
  forward,
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

/** Keyboard shortcuts of the history, as shown in tooltips. */
export const SHORTCUTS = IS_MAC
  ? { back: "⌘[", forward: "⌘]", settings: "⌘,", sidebar: "⌘B" }
  : { back: "Alt+←", forward: "Alt+→", settings: "Ctrl+,", sidebar: "Ctrl+B" }

/**
 * The navigation of the main window: history, persistence across launches,
 * keyboard shortcuts and the back and forward mouse buttons. `exists` tells
 * which locations are valid; others are removed from the history, for
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
      back: () => {
        setState(back)
      },
      forward: () => {
        setState(forward)
      },
    }
  }, [])

  const onKey = useCallback(
    (event: KeyboardEvent) => {
      const command = IS_MAC ? event.metaKey : event.ctrlKey
      if (command && !event.shiftKey && !event.altKey && event.key === ",") {
        event.preventDefault()
        actions.openSettings()
      } else if (IS_MAC && event.metaKey && !event.shiftKey && !event.altKey) {
        if (event.key === "[") {
          event.preventDefault()
          actions.back()
        } else if (event.key === "]") {
          event.preventDefault()
          actions.forward()
        }
      } else if (!IS_MAC && event.altKey && !event.ctrlKey && !event.metaKey) {
        if (event.key === "ArrowLeft") {
          event.preventDefault()
          actions.back()
        } else if (event.key === "ArrowRight") {
          event.preventDefault()
          actions.forward()
        }
      }
    },
    [actions],
  )

  useEffect(() => {
    // The back and forward buttons of a mouse.
    const onMouse = (event: MouseEvent) => {
      if (event.button === 3) {
        event.preventDefault()
        actions.back()
      } else if (event.button === 4) {
        event.preventDefault()
        actions.forward()
      }
    }
    window.addEventListener("keydown", onKey)
    window.addEventListener("mouseup", onMouse)
    return () => {
      window.removeEventListener("keydown", onKey)
      window.removeEventListener("mouseup", onMouse)
    }
  }, [onKey, actions])

  return useMemo(
    () => ({
      location: current(valid),
      /** The workspace page settings return to (and that stays mounted under them). */
      workspace: valid.workspace,
      canGoBack: canGoBack(valid),
      canGoForward: canGoForward(valid),
      ...actions,
    }),
    [valid, actions],
  )
}
