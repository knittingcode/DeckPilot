// The shared visual asset library (docs/key-text.md, "Icons and
// backgrounds"): DeckPilot's own icons and gradients (always there, offline,
// whatever plugins are installed) and the installed plugins' icons and
// backgrounds, as the backend lists them. Read once per app run and again
// whenever `reloadKey` changes (a plugin was installed, updated, enabled,
// disabled or removed). Reading it also tells the backend which plugin
// assets the editor's previews draw; `version` changes when they change,
// so the previews are drawn again.

import { useEffect, useState } from "react"

import { api } from "@/lib/api"
import type { VisualLibrary } from "@/lib/types"

/** Nothing listed yet (or no backend): no built-in assets either. */
export const EMPTY_LIBRARY: VisualLibrary = {
  categories: [],
  icons: [],
  gradients: [],
  plugins: [],
  version: 0,
}

let cached: VisualLibrary | null = null

/** The library, read again when `reloadKey` changes. */
export function useVisualLibrary(reloadKey: string): VisualLibrary {
  const [library, setLibrary] = useState<VisualLibrary>(cached ?? EMPTY_LIBRARY)
  useEffect(() => {
    let active = true
    // Not every embedding of the editor has the command (tests stub
    // `api`): the library is then empty.
    const read = (api as Partial<typeof api>).visualLibrary
    if (typeof read !== "function") return
    read().then(
      (loaded: VisualLibrary | undefined) => {
        // A stubbed backend may answer nothing.
        if (!loaded) return
        cached = loaded
        if (active) setLibrary(loaded)
      },
      () => {
        // No backend answer: what was read before stays.
      },
    )
    return () => {
      active = false
    }
  }, [reloadKey])
  return library
}

/** Forgets the cached library (tests). */
export function resetVisualLibrary(): void {
  cached = null
}
