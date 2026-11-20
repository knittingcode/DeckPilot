// The logic of the Icon and Background Libraries (icon-library.tsx,
// background-library.tsx): which source an icon is from, whether a
// referenced asset is drawn now, and the basics every key can have.

import type { CSSProperties } from "react"

import type { TranslationKey } from "@/i18n"
import type { IconInfo, VisualLibrary } from "@/lib/types"

import { assetPlugin, backgroundAsset, isBuiltinAsset } from "./appearance"

/** A checkerboard: how transparency looks. */
export const CHECKERBOARD: CSSProperties = {
  backgroundImage:
    "linear-gradient(45deg, #a3a3a3 25%, transparent 25%, transparent 75%, #a3a3a3 75%), linear-gradient(45deg, #a3a3a3 25%, #f5f5f5 25%, #f5f5f5 75%, #a3a3a3 75%)",
  backgroundSize: "8px 8px",
  backgroundPosition: "0 0, 4px 4px",
}

/** Tiles per row of a library grid. */
export const GRID_COLUMNS = 7

/** The three backgrounds that are always there, in this order. */
export const BASIC_BACKGROUNDS = [
  { value: "transparent", name: "editor:color.transparent" },
  { value: "#000000", name: "editor:color.black" },
  { value: "#ffffff", name: "editor:color.white" },
] as const satisfies readonly { value: string; name: TranslationKey }[]

/** Whether a background is drawn now: `missing` and `disabled` draw the default. */
export function backgroundStatus(
  background: string | undefined,
  library: VisualLibrary,
  mine: IconInfo[],
): "available" | "disabled" | "missing" {
  const id = backgroundAsset(background)
  if (!id) return "available"
  if (isBuiltinAsset(id)) {
    return library.gradients.length === 0 || library.gradients.some((g) => g.id === id)
      ? "available"
      : "missing"
  }
  if (!assetPlugin(id)) return mine.some((icon) => icon.id === id) ? "available" : "missing"
  const asset = library.plugins.find((a) => a.id === id)
  if (!asset) return "missing"
  return asset.available ? "available" : "disabled"
}

export type IconSource = "deckpilot" | "mine" | "plugins"

/** The tab an icon id belongs to. */
export function sourceOf(id: string | undefined, mine: IconInfo[]): IconSource {
  if (!id || isBuiltinAsset(id)) return "deckpilot"
  if (mine.some((icon) => icon.id === id) || !assetPlugin(id)) return "mine"
  return "plugins"
}

/** A Lucide name as a word list: `arrow-big-up` → `arrow big up`. */
export function iconTitle(name: string): string {
  return name.replace(/-/g, " ")
}

export function matches(query: string, ...texts: string[]): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  const haystack = texts.join(" ").toLowerCase()
  return words.every((word) => haystack.includes(word))
}

/** Whether an icon reference is drawn now (a plugin's needs its plugin enabled). */
export function iconAvailable(
  id: string,
  library: VisualLibrary,
  mine: IconInfo[],
): "available" | "disabled" | "missing" {
  if (isBuiltinAsset(id)) {
    return library.icons.length === 0 || library.icons.some((i) => i.id === id)
      ? "available"
      : "missing"
  }
  if (!assetPlugin(id)) return mine.some((i) => i.id === id) ? "available" : "missing"
  const asset = library.plugins.find((a) => a.id === id)
  if (!asset) return "missing"
  return asset.available ? "available" : "disabled"
}
