import { getCurrentWindow } from "@tauri-apps/api/window"
import { useEffect, useState } from "react"

import { api } from "@/lib/api"

import { IS_MAC } from "./use-navigation"

/**
 * Platform differences of the main window's chrome, in one place.
 *
 * - macOS: the content extends into the native titlebar (`titleBarStyle:
 *   "Overlay"` in `tauri.conf.json`). The native traffic lights stay; the
 *   app titlebar leaves room for them, unless the window is fullscreen
 *   (where macOS hides them), and moves the window when empty space is
 *   dragged.
 * - Windows and Linux: the native titlebar and window manager decorations
 *   are kept unchanged, so moving, resizing and the window buttons behave
 *   as everywhere else. The app titlebar is an ordinary toolbar below it.
 */
export interface WindowChrome {
  /** The app titlebar occupies the native titlebar area. */
  overlay: boolean
  /** Room to leave for the native window controls, in pixels. */
  controlsInset: number
}

/** Right edge of the traffic lights at `trafficLightPosition.x = 16`. */
const TRAFFIC_LIGHTS_WIDTH = 68

function inTauri() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
}

export function useWindowChrome(): WindowChrome {
  const overlay = IS_MAC && inTauri()
  const [fullscreen, setFullscreen] = useState(false)

  useEffect(() => {
    if (!overlay) return
    const appWindow = getCurrentWindow()
    const update = () => {
      appWindow.isFullscreen().then(setFullscreen, () => undefined)
    }
    update()
    const stop = appWindow.onResized(update)
    return () => {
      void stop.then((unlisten) => {
        unlisten()
      })
    }
  }, [overlay])

  return { overlay, controlsInset: overlay && !fullscreen ? TRAFFIC_LIGHTS_WIDTH : 0 }
}

/** Elements inside the titlebar that keep their own pointer behaviour. */
const INTERACTIVE =
  "button, a, input, select, textarea, label, summary, [role=button], [role=menuitem], [role=switch], [role=tab], [contenteditable=true], [data-no-drag]"

function isEmptySpace(target: EventTarget | null) {
  return target instanceof Element && !target.closest(INTERACTIVE)
}

/**
 * Pointer handlers for the titlebar: dragging empty space moves the window,
 * double-clicking it does what the system setting says. Controls inside
 * the titlebar are never affected.
 */
export function windowDragHandlers(chrome: WindowChrome) {
  if (!chrome.overlay) return {}
  return {
    onMouseDown: (event: React.MouseEvent) => {
      if (event.button !== 0 || event.detail !== 1 || !isEmptySpace(event.target)) return
      void getCurrentWindow()
        .startDragging()
        .catch(() => undefined)
    },
    onDoubleClick: (event: React.MouseEvent) => {
      if (!isEmptySpace(event.target)) return
      api.titlebarDoubleClick().catch(() => undefined)
    },
  }
}
