import { useSyncExternalStore } from "react"

const QUERY = "(prefers-reduced-motion: reduce)"

function query(): MediaQueryList | null {
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(QUERY)
    : null
}

function subscribe(listener: () => void) {
  const media = query()
  media?.addEventListener("change", listener)
  return () => {
    media?.removeEventListener("change", listener)
  }
}

function reduced(): boolean {
  return query()?.matches ?? false
}

/** Whether the system asks for reduced motion; follows changes. */
export function usePrefersReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, reduced, () => false)
}
