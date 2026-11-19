import { useSyncExternalStore } from "react"

/** The display's pixel ratio; 1 where the browser does not say. */
export function devicePixelRatio(): number {
  const ratio = typeof window === "undefined" ? 1 : window.devicePixelRatio
  return Number.isFinite(ratio) && ratio > 0 ? ratio : 1
}

function subscribe(listener: () => void) {
  if (typeof window.matchMedia !== "function") return () => undefined
  // A query for the current ratio stops matching when it changes (the
  // window moved to another display, or the page was zoomed): listen
  // again for the new one each time.
  let media: MediaQueryList | null = null
  const watch = () => {
    media?.removeEventListener("change", changed)
    media = window.matchMedia(`(resolution: ${devicePixelRatio()}dppx)`)
    media.addEventListener("change", changed)
  }
  const changed = () => {
    watch()
    listener()
  }
  watch()
  return () => {
    media?.removeEventListener("change", changed)
  }
}

/**
 * The display's pixel ratio (`window.devicePixelRatio`), updated when the
 * window moves to a display with another ratio or the page is zoomed.
 */
export function useDevicePixelRatio(): number {
  return useSyncExternalStore(subscribe, devicePixelRatio, () => 1)
}

/**
 * How many device pixels wide an image shown `cssPixels` wide must be to
 * be drawn 1:1 on this display, rather than scaled up by the browser.
 */
export function displayPixels(cssPixels: number, ratio: number): number {
  if (!Number.isFinite(cssPixels) || cssPixels <= 0) return 0
  return Math.round(cssPixels * (Number.isFinite(ratio) && ratio > 0 ? ratio : 1))
}
