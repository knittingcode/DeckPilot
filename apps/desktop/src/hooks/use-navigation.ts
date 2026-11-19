import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import {
  type Location,
  type NavigationState,
  back,
  browserStorage,
  canGoBack,
  canGoForward,
  current,
  forward,
  initialNavigation,
  loadNavigation,
  locationOf,
  push,
  sanitize,
  saveNavigation,
} from "@/lib/navigation"

export const IS_MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent)

/** Keyboard shortcuts of the history, as shown in tooltips. */
export const SHORTCUTS = IS_MAC
  ? { back: "⌘[", forward: "⌘]", settings: "⌘,", sidebar: "⌘B" }
  : { back: "Alt+←", forward: "Alt+→", settings: "Ctrl+,", sidebar: "Ctrl+B" }

/**
 * The navigation of the main window: history, persistence across launches,
 * keyboard shortcuts and the back and forward mouse buttons. `exists` tells
 * which workspace pages are valid; others are removed from the history, for
 * example after a module was disabled.
 *
 * The settings are a window of their own: locations in them (`settings:…`,
 * and pages that moved there such as `profiles`) go to `openSettings`
 * instead of the history.
 */
export function useNavigation(
  exists: ((location: Location) => boolean) | null,
  openSettings: (page?: string) => void,
) {
  const [stored, setStored] = useState<NavigationState>(
    () => loadNavigation(browserStorage()) ?? initialNavigation(),
  )

  // Locations that stopped existing are dropped when the state is used,
  // and before every change.
  const existsRef = useRef(exists)
  const settingsRef = useRef(openSettings)
  useEffect(() => {
    existsRef.current = exists
    settingsRef.current = openSettings
  }, [exists, openSettings])
  const valid = useMemo(() => (exists ? sanitize(stored, exists) : stored), [stored, exists])

  useEffect(() => {
    saveNavigation(browserStorage(), valid)
  }, [valid])

  const actions = useMemo(() => {
    const setState = (change: (state: NavigationState) => NavigationState) => {
      setStored((previous) => {
        const exists = existsRef.current
        return change(exists ? sanitize(previous, exists) : previous)
      })
    }
    const go = (location: Location) => {
      if (location.mode === "settings") {
        settingsRef.current(location.page)
        return
      }
      setState((previous) => push(previous, location))
    }
    return {
      // Old page names (e.g. `models`) lead to where the page moved.
      navigate: (page: string) => {
        go(locationOf(page))
      },
      go,
      openSettings: (page?: string) => {
        settingsRef.current(page)
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
      canGoBack: canGoBack(valid),
      canGoForward: canGoForward(valid),
      ...actions,
    }),
    [valid, actions],
  )
}
