// The non-component parts of the key header, its title appearance popover
// and its icon menu: limits and choices of docs/key-text.md, the title fonts,
// and reading an image from the clipboard.

import { useEffect, useState } from "react"

import { type TranslationKey, i18n } from "@/i18n"
import { api } from "@/lib/api"
import type { ActionTitle, ControlId, FontSize, TextMode, TitleFont } from "@/lib/types"

import type { KeyLook } from "./key-look-context"

// The limits of docs/key-text.md (deckpilot-core `TextAppearance`).
export const TEXT_LIMITS = {
  size: [6, 32],
  maxLines: [1, 6],
  speed: [5, 240],
  pause: [0, 10_000],
} as const

export const TEXT_MODES: { value: TextMode; label: TranslationKey }[] = [
  { value: "wrap", label: "editor:text.mode.wrap" },
  { value: "singleLine", label: "editor:text.mode.singleLine" },
  { value: "autoFit", label: "editor:text.mode.autoFit" },
  { value: "marquee", label: "editor:text.mode.marquee" },
]

/** The preset sizes in points (`deckpilot-render` `title_size` on a 72 px key). */
export const PRESET_POINTS: Record<FontSize, number> = {
  small: 9.4,
  medium: 11.2,
  large: 13.7,
}

/** The font of titles that set none (`deckpilot_core::DEFAULT_TITLE_FONT`). */
export const DEFAULT_TITLE_FONT = "inter"

let fontsRequest: Promise<TitleFont[]> | null = null

/**
 * The fonts the renderer can draw titles with (`title_fonts`), asked once.
 * Empty until they arrive, or when the backend cannot say.
 */
export function useTitleFonts(): TitleFont[] {
  const [fonts, setFonts] = useState<TitleFont[]>([])
  useEffect(() => {
    let cancelled = false
    if (!fontsRequest) {
      const request = typeof api.titleFonts === "function" ? api.titleFonts() : null
      fontsRequest = (request ?? Promise.resolve([])).catch(() => {
        // Asked again next time.
        fontsRequest = null
        return []
      })
    }
    void fontsRequest.then((list) => {
      if (!cancelled) setFonts(list)
    })
    return () => {
      cancelled = true
    }
  }, [])
  return fonts
}

const PASTE_TYPES = ["image/png", "image/jpeg"]

/** Why nothing was pasted, for the message shown. */
export type PasteProblem = "empty" | "format" | "denied" | "unsupported"

/**
 * The first PNG or JPEG image on the clipboard as a file, through the
 * WebView's asynchronous clipboard (no clipboard permission of the app's
 * own: the system asks when it wants to), or why there is none.
 */
export async function clipboardImage(): Promise<File | PasteProblem> {
  const clipboard = typeof navigator === "undefined" ? undefined : navigator.clipboard
  if (typeof clipboard?.read !== "function") return "unsupported"
  let items: ClipboardItems
  try {
    items = await clipboard.read()
  } catch {
    return "denied"
  }
  let image = false
  for (const item of items) {
    const type = item.types.find((t) => PASTE_TYPES.includes(t))
    if (type) {
      const blob = await item.getType(type)
      const name = `${i18n.t("editor:iconMenu.pastedName")}.${type === "image/png" ? "png" : "jpg"}`
      return new File([blob], name, { type })
    }
    if (item.types.some((t) => t.startsWith("image/"))) image = true
  }
  return image ? "format" : "empty"
}

/** The title suggestions of the one key's action type (plugin API 1.8). */
export function suggestionsOf(look: KeyLook, controls: ControlId[]): ActionTitle | undefined {
  const [only] = controls
  return controls.length === 1 && only ? look.typeOf(only)?.title : undefined
}
