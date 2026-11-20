// Pure logic of the key appearance editor: which binding carries a key's
// look, editing it for several keys at once, colours and contrast, and
// checks of icon files before they are uploaded.

import { type TranslationKey, i18n } from "@/i18n"
import { formatter } from "@/i18n/format"
import { bindingsFor } from "@/lib/layout-model"
import type {
  ControlBinding,
  ControlId,
  DefaultAppearance,
  DeviceLayout,
  DeviceModel,
  FontSize,
  IconFit,
  KeyAppearance,
  KeyBackground,
  TextAppearance,
  TitleAlign,
} from "@/lib/types"

// ---------------------------------------------------------------------------
// Which binding a key's look belongs to

/**
 * The binding a key's face shows, as the agent picks it: the press binding,
 * else the click binding, else the first binding of the control (mirrors
 * `shown_binding`).
 */
export function shownBinding(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): ControlBinding | undefined {
  const bindings = bindingsFor(layout, pageId, control)
  return (
    bindings.find((b) => b.trigger.type === "press") ??
    bindings.find((b) => b.trigger.type === "click") ??
    bindings[0]
  )
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
  if (appearance.background) {
    // Colours are lower-cased; an asset's id is kept as it is.
    result.background = isAssetBackground(appearance.background)
      ? appearance.background
      : appearance.background.toLowerCase()
  }
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
  const text = appearance.text ? normalizeText(appearance.text) : null
  if (text) result.text = text
  if (appearance.fontFamily) result.fontFamily = appearance.fontFamily
  // Both values are kept: an explicit `false` beats a plugin's suggestion.
  for (const field of ["bold", "italic", "underline"] as const) {
    const value = appearance[field]
    if (typeof value === "boolean") result[field] = value
  }
  return Object.keys(result).length > 0 ? result : null
}

const TEXT_FIELDS = [
  "size",
  "align",
  "maxLines",
  "overflow",
  "minSize",
  "fit",
  "speed",
  "pause",
] as const satisfies readonly (keyof TextAppearance)[]

/**
 * Drops unset text settings; `null` when nothing but the default mode
 * (wrap) is left, as the backend does not store it.
 */
export function normalizeText(text: TextAppearance): TextAppearance | null {
  const result: Record<string, unknown> = {}
  for (const field of TEXT_FIELDS) {
    const value = text[field]
    if (value !== null && value !== undefined) result[field] = value
  }
  const mode = text.mode ?? "wrap"
  if (mode === "wrap" && Object.keys(result).length === 0) return null
  return { mode, ...result }
}

/** Sets one text setting of every key (`null` resets it), keeping the others. */
export function withText<K extends keyof TextAppearance>(
  field: K,
  value: TextAppearance[K] | null,
) {
  return (appearance: KeyAppearance): KeyAppearance => ({
    ...appearance,
    text: { ...(appearance.text ?? {}), [field]: value },
  })
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
  fontFamily: Shared<string>
  bold: Shared<boolean>
  italic: Shared<boolean>
  underline: Shared<boolean>
  iconId: Shared<string>
  iconFit: Shared<IconFit>
  /** Each text setting the keys share (the mode defaults to wrap). */
  text: { [K in TextField]: Shared<NonNullable<TextAppearance[K]>> }
}

/** The settings of a text appearance. */
type TextField = keyof TextAppearance

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
    fontFamily: shared((a) => a.fontFamily ?? undefined),
    bold: shared((a) => a.bold ?? undefined),
    italic: shared((a) => a.italic ?? undefined),
    underline: shared((a) => a.underline ?? undefined),
    iconId: shared((a) => a.icon?.id),
    iconFit: shared((a) => (a.icon ? (a.icon.fit ?? "fit") : undefined)),
    text: {
      mode: shared((a) => a.text?.mode ?? "wrap"),
      size: shared((a) => a.text?.size ?? undefined),
      align: shared((a) => a.text?.align ?? undefined),
      maxLines: shared((a) => a.text?.maxLines ?? undefined),
      overflow: shared((a) => a.text?.overflow ?? undefined),
      minSize: shared((a) => a.text?.minSize ?? undefined),
      fit: shared((a) => a.text?.fit ?? undefined),
      speed: shared((a) => a.text?.speed ?? undefined),
      pause: shared((a) => a.text?.pause ?? undefined),
    },
  }
}

/**
 * The fields of a key's look that belong to its title's formatting (the
 * title appearance editor): what its Reset clears. The background and the
 * icon are the key's, and whether the title shows is the header's own
 * switch, not formatting.
 */
export const TITLE_FIELDS = [
  "titleAlign",
  "textColor",
  "fontSize",
  "fontFamily",
  "bold",
  "italic",
  "underline",
  "text",
] as const satisfies readonly (keyof KeyAppearance)[]

/** Clears every title setting (`TITLE_FIELDS`): the inherited defaults again. */
export function withoutTitleLook(appearance: KeyAppearance): KeyAppearance {
  const title: readonly string[] = TITLE_FIELDS
  return Object.fromEntries(Object.entries(appearance).filter(([field]) => !title.includes(field)))
}

/** Whether a look sets anything of its title (what Reset would clear). */
export function hasTitleLook(appearance: KeyAppearance): boolean {
  const normalized = normalizeAppearance(appearance)
  return normalized !== null && TITLE_FIELDS.some((field) => normalized[field] !== undefined)
}

/**
 * Sets the visible title of a key: the label of the binding its face shows
 * (`shownBinding`); nothing else of the binding changes (a folder key keeps
 * opening its folder). An empty title is no label: the default title.
 */
export function setShownLabel(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  label: string,
): DeviceLayout {
  const binding = shownBinding(layout, pageId, control)
  if (!binding) return layout
  const next: ControlBinding = { ...binding, label: label === "" ? null : label }
  return {
    ...layout,
    pages: layout.pages.map((p) =>
      p.id === pageId ? { ...p, bindings: p.bindings.map((b) => (b === binding ? next : b)) } : p,
    ),
  }
}

/**
 * How many keys of the layout (all pages) show each icon: as their icon or
 * as their background image (`asset:<id>`), counted once per key.
 */
export function iconUsage(layout: DeviceLayout): Map<string, number> {
  const usage = new Map<string, number>()
  for (const p of layout.pages) {
    for (const b of p.bindings) {
      const icon = b.appearance?.icon?.id
      const background = backgroundAsset(b.appearance?.background)
      for (const id of new Set([icon, background])) {
        if (id) usage.set(id, (usage.get(id) ?? 0) + 1)
      }
    }
  }
  return usage
}

// ---------------------------------------------------------------------------
// Library assets (docs/key-text.md, "Icons and backgrounds")

/** The prefix of a background that is an asset of the library. */
export const ASSET_PREFIX = "asset:"

/** The namespace of DeckPilot's own assets (`deckpilot.<name>`). */
export const BUILTIN_NAMESPACE = "deckpilot"

/** Whether a stored background is an asset (a gradient or an image). */
export function isAssetBackground(background: string | null | undefined): boolean {
  return typeof background === "string" && background.startsWith(ASSET_PREFIX)
}

/** The asset a background refers to (`asset:<id>` → `<id>`), if any. */
export function backgroundAsset(background: string | null | undefined): string | undefined {
  return isAssetBackground(background) ? background?.slice(ASSET_PREFIX.length) : undefined
}

/** The stored background of an asset. */
export function assetBackground(id: string): string {
  return `${ASSET_PREFIX}${id}`
}

/** The plugin an asset id belongs to (`<publisher>.<plugin>.<name>`), if any. */
export function assetPlugin(id: string): string | undefined {
  const parts = id.split(".")
  return parts.length === 3 ? `${parts[0]}.${parts[1]}` : undefined
}

/** Whether an id is one of DeckPilot's own assets (`deckpilot.<name>`). */
export function isBuiltinAsset(id: string): boolean {
  const parts = id.split(".")
  return parts.length === 2 && parts[0] === BUILTIN_NAMESPACE
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

export const SWATCHES: { name: TranslationKey; value: string }[] = [
  { name: "editor:color.black", value: "#000000" },
  { name: "editor:color.graphite", value: "#262626" },
  { name: "editor:color.white", value: "#ffffff" },
  { name: "editor:color.red", value: "#dc2626" },
  { name: "editor:color.orange", value: "#ea580c" },
  { name: "editor:color.amber", value: "#f59e0b" },
  { name: "editor:color.green", value: "#16a34a" },
  { name: "editor:color.teal", value: "#0d9488" },
  { name: "editor:color.blue", value: "#2563eb" },
  { name: "editor:color.indigo", value: "#4f46e5" },
  { name: "editor:color.purple", value: "#9333ea" },
  { name: "editor:color.pink", value: "#db2777" },
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

/**
 * The defaults of models that set none, as `DefaultAppearance::BUILTIN`:
 * transparent keys on a black display, white text (the Stream Decks').
 */
export const BUILTIN_APPEARANCE: DefaultAppearance = {
  keyBackground: "transparent",
  textColor: "#ffffff",
  displayBackground: "#000000",
}

/** A model's colours for keys that set none (`DeviceModel::effective_appearance`). */
export function defaultAppearanceOf(
  model: Pick<DeviceModel, "defaultAppearance"> | null | undefined,
): DefaultAppearance {
  return model?.defaultAppearance ?? BUILTIN_APPEARANCE
}

/** The colours a key is drawn with, opaque `#rrggbb`. */
export interface KeyColors {
  background: string
  text: string
}

/**
 * The colours of a key's look on a model, as the renderer resolves them
 * (`deckpilot_render::key_colors`, without a live state's accent): the
 * key's own colour, else the model's default; a transparent background is
 * the display background behind it. The kind of key does not matter.
 */
export function keyColors(
  appearance: Pick<KeyAppearance, "background" | "textColor">,
  defaults: DefaultAppearance = BUILTIN_APPEARANCE,
): KeyColors {
  // An asset (gradient, image) is drawn over the default background, and
  // stands in for it while the asset is not available.
  const own = isAssetBackground(appearance.background) ? undefined : appearance.background
  const background = own ?? defaults.keyBackground
  return {
    background: (background === "transparent"
      ? defaults.displayBackground
      : background
    ).toLowerCase(),
    text: (appearance.textColor ?? defaults.textColor).toLowerCase(),
  }
}

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
  background: KeyBackground | null | undefined,
  text: string | null | undefined,
  defaults: DefaultAppearance = BUILTIN_APPEARANCE,
): ContrastHint {
  const colors = keyColors({ background, textColor: text }, defaults)
  const ratio = contrastRatio(colors.background, colors.text)
  const rounded = Math.round(ratio * 10) / 10
  const shown = formatter().number(rounded)
  const level: ContrastHint["level"] = ratio >= 4.5 ? "good" : ratio >= 3 ? "low" : "poor"
  const messages = {
    good: "editor:appearance.contrast.good",
    low: "editor:appearance.contrast.low",
    poor: "editor:appearance.contrast.poor",
  } as const
  return { ratio: rounded, level, message: i18n.t(messages[level], { ratio: shown }) }
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
    return i18n.t("editor:icons.tooMany", { max: MAX_ICONS })
  }
  const typed = ICON_TYPES.includes(file.type) || /\.(png|jpe?g)$/i.test(file.name)
  if (!typed) return i18n.t("editor:icons.wrongType")
  if (file.size === 0) return i18n.t("editor:icons.emptyFile")
  if (file.size > MAX_ICON_BYTES) return i18n.t("editor:icons.tooLarge")
  return null
}

export function iconSizeProblem(width: number, height: number): string | null {
  return width > MAX_ICON_PIXELS || height > MAX_ICON_PIXELS
    ? i18n.t("editor:icons.tooManyPixels", { max: MAX_ICON_PIXELS })
    : null
}

/** A name for an imported icon: the file name without extension, 1–64 characters. */
export function iconName(fileName: string): string {
  const base = fileName.replace(/\.[^.]+$/, "").trim()
  return (base || i18n.t("editor:icons.defaultName")).slice(0, 64)
}

/** How an icon fit shows in CSS, for the live preview before rendering. */
export const FIT_OBJECT: Record<IconFit, "contain" | "fill" | "cover"> = {
  fit: "contain",
  fill: "fill",
  crop: "cover",
}
