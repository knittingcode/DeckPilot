// Pure logic of the key appearance editor: which binding carries a key's
// look, editing it for several keys at once, colours and contrast, and
// checks of icon files before they are uploaded.

import { bindingsFor } from "@/lib/layout-model"
import type {
  ControlBinding,
  ControlId,
  DeviceLayout,
  FontSize,
  IconFit,
  KeyAppearance,
  TitleAlign,
} from "@/lib/types"

// ---------------------------------------------------------------------------
// Which binding a key's look belongs to

/**
 * The binding a key's face shows, as the agent picks it: the press binding,
 * else the first binding of the control (mirrors `shown_binding`).
 */
export function shownBinding(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): ControlBinding | undefined {
  const bindings = bindingsFor(layout, pageId, control)
  return bindings.find((b) => b.trigger.type === "press") ?? bindings[0]
}

export function appearanceOf(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): KeyAppearance {
  return shownBinding(layout, pageId, control)?.appearance ?? {}
}

/** Controls whose look can be set: they show something and have a display. */
export function stylable(control: ControlId): boolean {
  return control.kind !== "encoder"
}

// ---------------------------------------------------------------------------
// Editing

type Field = keyof KeyAppearance

/** A change: a value sets a field, `null` resets it to the default. */
export type AppearancePatch = { [K in Field]?: KeyAppearance[K] | null }

/**
 * Drops unset fields and lower-cases colours; `null` when nothing is set,
 * because the backend does not store an appearance that sets nothing.
 */
export function normalizeAppearance(appearance: KeyAppearance): KeyAppearance | null {
  const result: KeyAppearance = {}
  if (appearance.background) result.background = appearance.background.toLowerCase()
  if (appearance.textColor) result.textColor = appearance.textColor.toLowerCase()
  if (appearance.fontSize) result.fontSize = appearance.fontSize
  if (appearance.titleAlign) result.titleAlign = appearance.titleAlign
  if (appearance.showTitle === false) result.showTitle = false
  if (appearance.icon) {
    result.icon =
      appearance.icon.fit && appearance.icon.fit !== "fit"
        ? { id: appearance.icon.id, fit: appearance.icon.fit }
        : { id: appearance.icon.id }
  }
  return Object.keys(result).length > 0 ? result : null
}

export interface AppearanceResult {
  layout: DeviceLayout
  /** Controls that were changed. */
  changed: ControlId[]
  /** Selected controls without a binding (or without a display) to carry a look. */
  skipped: ControlId[]
}

/** `null` in the patch resets a field; `normalizeAppearance` drops it. */
function patched(appearance: KeyAppearance, patch: AppearancePatch): KeyAppearance {
  return { ...appearance, ...patch }
}

/** A change of the look: fields to set, or a function of each key's look. */
export type KeyLookChange = AppearancePatch | ((appearance: KeyAppearance) => KeyAppearance)

/** Sets the fit of keys that show an icon, keeping each key's icon. */
export function withIconFit(fit: IconFit) {
  return (appearance: KeyAppearance): KeyAppearance =>
    appearance.icon ? { ...appearance, icon: { id: appearance.icon.id, fit } } : appearance
}

/**
 * Applies a change to the shown binding of every control: a patch of
 * fields, or a function of each key's current look.
 */
export function applyAppearance(
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
  change: KeyLookChange,
): AppearanceResult {
  const changed: ControlId[] = []
  const skipped: ControlId[] = []
  const targets = new Map<ControlBinding, ControlBinding>()
  for (const control of controls) {
    const binding = stylable(control) ? shownBinding(layout, pageId, control) : undefined
    if (!binding) {
      skipped.push(control)
      continue
    }
    const current = binding.appearance ?? {}
    const appearance = normalizeAppearance(
      typeof change === "function" ? change(current) : patched(current, change),
    )
    const next: ControlBinding = { ...binding }
    if (appearance) next.appearance = appearance
    else delete next.appearance
    targets.set(binding, next)
    changed.push(control)
  }
  if (targets.size === 0) return { layout, changed, skipped }
  return {
    layout: {
      ...layout,
      pages: layout.pages.map((p) =>
        p.id === pageId ? { ...p, bindings: p.bindings.map((b) => targets.get(b) ?? b) } : p,
      ),
    },
    changed,
    skipped,
  }
}

/** The value all controls share, `"mixed"` when they differ. */
export type Shared<T> = T | undefined | "mixed"

export type SharedAppearance = {
  background: Shared<string>
  textColor: Shared<string>
  fontSize: Shared<FontSize>
  titleAlign: Shared<TitleAlign>
  showTitle: Shared<boolean>
  iconId: Shared<string>
  iconFit: Shared<IconFit>
}

/** What the selected keys have in common, for one panel over all of them. */
export function sharedAppearance(
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
): SharedAppearance {
  const looks = controls
    .filter(stylable)
    .filter((c) => shownBinding(layout, pageId, c))
    .map((c) => appearanceOf(layout, pageId, c))
  const shared = <T>(read: (a: KeyAppearance) => T | undefined): Shared<T> => {
    const values = looks.map(read)
    const first = values[0]
    return values.every((v) => v === first) ? first : "mixed"
  }
  return {
    background: shared((a) => a.background ?? undefined),
    textColor: shared((a) => a.textColor ?? undefined),
    fontSize: shared((a) => a.fontSize ?? undefined),
    titleAlign: shared((a) => a.titleAlign ?? undefined),
    showTitle: shared((a) => a.showTitle ?? true),
    iconId: shared((a) => a.icon?.id),
    iconFit: shared((a) => (a.icon ? (a.icon.fit ?? "fit") : undefined)),
  }
}

/** How many keys of the layout (all pages) show each icon. */
export function iconUsage(layout: DeviceLayout): Map<string, number> {
  const usage = new Map<string, number>()
  for (const p of layout.pages) {
    for (const b of p.bindings) {
      const id = b.appearance?.icon?.id
      if (id) usage.set(id, (usage.get(id) ?? 0) + 1)
    }
  }
  return usage
}

/**
 * Keys showing an icon, counting this layout as edited: the stored count
 * (all layouts as saved) without this layout's saved keys, plus its draft.
 */
export function effectiveUsage(
  stored: number,
  iconId: string,
  saved: DeviceLayout,
  draft: DeviceLayout,
): number {
  return Math.max(
    0,
    stored - (iconUsage(saved).get(iconId) ?? 0) + (iconUsage(draft).get(iconId) ?? 0),
  )
}

// ---------------------------------------------------------------------------
// Colours

export const SWATCHES: { name: string; value: string }[] = [
  { name: "Black", value: "#000000" },
  { name: "Graphite", value: "#262626" },
  { name: "White", value: "#ffffff" },
  { name: "Red", value: "#dc2626" },
  { name: "Orange", value: "#ea580c" },
  { name: "Amber", value: "#f59e0b" },
  { name: "Green", value: "#16a34a" },
  { name: "Teal", value: "#0d9488" },
  { name: "Blue", value: "#2563eb" },
  { name: "Indigo", value: "#4f46e5" },
  { name: "Purple", value: "#9333ea" },
  { name: "Pink", value: "#db2777" },
]

/** `#rrggbb` (or `#rgb`, expanded) in lower case; `null` when not a colour. */
export function parseHex(input: string): string | null {
  const value = input.trim().replace(/^#?/, "#").toLowerCase()
  if (/^#[0-9a-f]{6}$/.test(value)) return value
  if (/^#[0-9a-f]{3}$/.test(value)) {
    const [r, g, b] = value.slice(1)
    return `#${r}${r}${g}${g}${b}${b}`
  }
  return null
}

function luminance(hex: string): number {
  const channel = (offset: number) => {
    const c = parseInt(hex.slice(offset, offset + 2), 16) / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
}

/** WCAG contrast ratio of two `#rrggbb` colours, 1–21. */
export function contrastRatio(a: string, b: string): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number]
  return (light + 0.05) / (dark + 0.05)
}

/** Colours a key uses when its appearance sets none (the action style). */
export const DEFAULT_BACKGROUND = "#262626"
export const DEFAULT_TEXT = "#fafafa"

export interface ContrastHint {
  ratio: number
  level: "good" | "low" | "poor"
  message: string
}

/**
 * How readable the title is. Small key text needs strong contrast, so the
 * thresholds follow WCAG for normal text (4.5) and large text (3).
 */
export function contrastHint(
  background: string | null | undefined,
  text: string | null | undefined,
): ContrastHint {
  const ratio = contrastRatio(background ?? DEFAULT_BACKGROUND, text ?? DEFAULT_TEXT)
  const rounded = Math.round(ratio * 10) / 10
  if (ratio >= 4.5) return { ratio: rounded, level: "good", message: `Contrast ${rounded}:1` }
  if (ratio >= 3) {
    return {
      ratio: rounded,
      level: "low",
      message: `Contrast ${rounded}:1 — readable only with large text`,
    }
  }
  return { ratio: rounded, level: "poor", message: `Contrast ${rounded}:1 — hard to read` }
}

// ---------------------------------------------------------------------------
// Icon files

export const MAX_ICONS = 64
export const MAX_ICON_BYTES = 2 * 1024 * 1024
export const MAX_ICON_PIXELS = 4096
const ICON_TYPES = ["image/png", "image/jpeg"]

/** Checks a file before uploading it; the backend checks the content again. */
export function iconFileProblem(
  file: { name: string; type: string; size: number },
  iconCount: number,
): string | null {
  if (iconCount >= MAX_ICONS) {
    return `A profile holds at most ${MAX_ICONS} icons. Delete unused icons first.`
  }
  const typed = ICON_TYPES.includes(file.type) || /\.(png|jpe?g)$/i.test(file.name)
  if (!typed) return "Choose a PNG or JPEG image."
  if (file.size === 0) return "The file is empty."
  if (file.size > MAX_ICON_BYTES) return "The image is larger than 2 MiB."
  return null
}

export function iconSizeProblem(width: number, height: number): string | null {
  return width > MAX_ICON_PIXELS || height > MAX_ICON_PIXELS
    ? `The image is larger than ${MAX_ICON_PIXELS} × ${MAX_ICON_PIXELS} pixels.`
    : null
}

/** A name for an imported icon: the file name without extension, 1–64 characters. */
export function iconName(fileName: string): string {
  const base = fileName.replace(/\.[^.]+$/, "").trim()
  return (base || "Icon").slice(0, 64)
}

/** How an icon fit shows in CSS, for the live preview before rendering. */
export const FIT_OBJECT: Record<IconFit, "contain" | "fill" | "cover"> = {
  fit: "contain",
  fill: "fill",
  crop: "cover",
}
