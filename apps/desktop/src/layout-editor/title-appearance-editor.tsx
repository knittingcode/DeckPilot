// The one title appearance editor of every key with a display (docs/key-text.md,
// "Title appearance"): a compact T▾ button next to the key's title that opens a
// popover with the title's visibility, position, font, size, style, colour and
// text layout. Folder keys, actions, Multi Actions, plugin actions and layout
// navigation keys all use it; a plugin only says which parts it offers
// (`ActionType.title.supports`), it cannot replace it.

import {
  AlignCenter,
  AlignLeft,
  AlignRight,
  AlignVerticalJustifyCenter,
  AlignVerticalJustifyEnd,
  AlignVerticalJustifyStart,
  Bold,
  ChevronDown,
  Italic,
  RotateCcw,
  Type,
  Underline,
} from "lucide-react"
import { type ComponentType, useId, useState } from "react"

import { Button } from "@/components/ui/button"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { usePrefersReducedMotion } from "@/hooks/use-reduced-motion"
import { type TranslationKey, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { controlKey } from "@/lib/layout-model"
import type {
  ControlId,
  KeyAppearance,
  TextAlign,
  TextMode,
  TitleAlign,
  TitleFeature,
} from "@/lib/types"

import {
  hasTitleLook,
  keyColors,
  sharedAppearance,
  shownBinding,
  stylable,
  withText,
  withoutTitleLook,
} from "./appearance"
import { ColorField } from "./color-field"
import { suggestionsOf } from "./title-look"
import { type KeyLook, useKeyLook } from "./key-look-context"
import { NumberField, ReportNotice, TextModeSettings } from "./text-appearance"
import {
  DEFAULT_TITLE_FONT,
  PRESET_POINTS,
  TEXT_LIMITS,
  TEXT_MODES,
  useTitleFonts,
} from "./title-look"

const POSITIONS: { value: TitleAlign; label: TranslationKey; icon: ComponentType }[] = [
  { value: "top", label: "editor:title.position.top", icon: AlignVerticalJustifyStart },
  { value: "middle", label: "editor:title.position.middle", icon: AlignVerticalJustifyCenter },
  { value: "bottom", label: "editor:title.position.bottom", icon: AlignVerticalJustifyEnd },
]

const STYLES: {
  value: "bold" | "italic" | "underline"
  label: TranslationKey
  icon: ComponentType
}[] = [
  { value: "bold", label: "editor:title.style.bold", icon: Bold },
  { value: "italic", label: "editor:title.style.italic", icon: Italic },
  { value: "underline", label: "editor:title.style.underline", icon: Underline },
]

const ALIGNS: { value: TextAlign; label: TranslationKey; icon: ComponentType }[] = [
  { value: "left", label: "editor:text.align.left", icon: AlignLeft },
  { value: "center", label: "editor:text.align.center", icon: AlignCenter },
  { value: "right", label: "editor:text.align.right", icon: AlignRight },
]

/** Compact controls are 32px high (`h-8`), icons centred. */
const ICON_ITEM = "size-8 min-w-8 px-0"

/**
 * The compact title format button (T▾) next to a key's title and the
 * popover it opens. Opening it changes nothing; every change is an undo
 * step of the editor and shows on the Canvas at once.
 */
export function TitleFormatButton({ controls }: { controls: ControlId[] }) {
  const { t } = useTranslation()
  const look = useKeyLook()
  const [open, setOpen] = useState(false)
  if (!look) return null
  const styled = controls.filter(
    (c) => stylable(c) && shownBinding(look.draft, look.pageId, c) !== undefined,
  )
  if (styled.length === 0) return null
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="sm"
          data-slot="title-format-trigger"
          aria-label={t("editor:title.open")}
          title={t("editor:title.open")}
          className="h-8 shrink-0 gap-0.5 px-1.5"
        >
          <Type />
          <ChevronDown className="size-3 opacity-70" />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        side="bottom"
        collisionPadding={8}
        sticky="always"
        data-slot="title-popover"
        aria-label={t("editor:title.popover")}
        role="dialog"
        className="grid w-[min(20rem,calc(100vw-1rem))] gap-2.5 p-3"
        // The popover never grows past the room the window has.
        style={{ maxHeight: "var(--radix-popover-content-available-height)" }}
      >
        <TitleAppearanceEditor look={look} controls={styled} />
      </PopoverContent>
    </Popover>
  )
}

/** The rows of the title appearance popover, for `controls` (one or several keys). */
export function TitleAppearanceEditor({
  look,
  controls,
}: {
  look: KeyLook
  controls: ControlId[]
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const id = useId()
  const reduced = usePrefersReducedMotion()
  const fonts = useTitleFonts()
  const shared = sharedAppearance(look.draft, look.pageId, controls)
  const suggestions = suggestionsOf(look, controls)
  const offers = (feature: TitleFeature) =>
    !suggestions?.supports || suggestions.supports.includes(feature)
  const set = (change: Parameters<KeyLook["restyle"]>[0], group?: string) => {
    look.restyle(change, group)
  }
  const [only] = controls
  const single = controls.length === 1 && only ? controlKey(only) : null
  const face = single ? look.faces.get(single) : undefined
  // Where the title is drawn when the key sets nothing: the plugin's
  // suggestion, else bottom under an icon and middle without (the face
  // the agent resolved carries both).
  const hasMark =
    Boolean(face?.symbol) ||
    Boolean(face?.glyph) ||
    controls.some((c) => shownBinding(look.draft, look.pageId, c)?.appearance?.icon)
  const position =
    shared.titleAlign === "mixed"
      ? ""
      : (shared.titleAlign ??
        face?.appearance?.titleAlign ??
        suggestions?.position ??
        (hasMark ? "bottom" : "middle"))
  const looks = controls.map(
    (c): KeyAppearance => shownBinding(look.draft, look.pageId, c)?.appearance ?? {},
  )
  const overridden = looks.some(hasTitleLook)
  const text = shared.text
  const preset = shared.fontSize === "mixed" ? undefined : (shared.fontSize ?? "medium")
  const presetPoints = preset ? PRESET_POINTS[preset] : undefined
  const preferred = typeof text.size === "number" ? text.size : presetPoints
  const family = shared.fontFamily === "mixed" ? "" : (shared.fontFamily ?? DEFAULT_TITLE_FONT)
  const styles = STYLES.filter((style) => shared[style.value] === true).map((s) => s.value)
  const unset = keyColors({}, look.defaults)
  const report = single ? look.reports?.get(single) : undefined
  const fontName = (font: string) => fonts.find((f) => f.id === font)?.name ?? font
  // A font a later version wrote is kept and offered as it is.
  const fontOptions =
    family && !fonts.some((f) => f.id === family)
      ? [...fonts, { id: family, name: fontName(family) }]
      : fonts

  return (
    <>
      <div data-slot="title-row" className="flex items-center gap-2">
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-label={t("editor:title.position.label")}
          disabled={!offers("position")}
          value={position}
          onValueChange={(value) => {
            if (value) set({ titleAlign: value as TitleAlign })
          }}
        >
          {POSITIONS.map((option) => (
            <ToggleGroupItem
              key={option.value}
              value={option.value}
              aria-label={t(option.label)}
              title={t(option.label)}
              className={ICON_ITEM}
            >
              <option.icon />
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="ml-auto"
          aria-label={t("editor:title.reset")}
          title={t("editor:title.reset")}
          disabled={!overridden}
          onClick={() => {
            // Clears the overrides: the device's (and a plugin's) defaults
            // again; nothing resolved is written.
            set(withoutTitleLook)
          }}
        >
          <RotateCcw />
        </Button>
      </div>

      <Select
        value={family}
        disabled={!offers("font") || fontOptions.length === 0}
        onValueChange={(value) => {
          // The default font is not stored.
          set({ fontFamily: value === DEFAULT_TITLE_FONT ? null : value })
        }}
      >
        <SelectTrigger
          size="sm"
          className="w-full"
          aria-label={t("editor:title.font")}
          data-slot="title-font"
        >
          <SelectValue placeholder={family ? fontName(family) : t("editor:text.mixed")} />
        </SelectTrigger>
        <SelectContent>
          {fontOptions.map((font) => (
            <SelectItem key={font.id} value={font.id}>
              {font.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>

      <div data-slot="title-row" className="flex items-start gap-2">
        <NumberField
          label={t("editor:title.size")}
          hideLabel
          value={text.size}
          range={TEXT_LIMITS.size}
          disabled={!offers("font")}
          placeholder={
            presetPoints === undefined
              ? t("editor:text.mixed")
              : format.number(Math.round(presetPoints))
          }
          onCommit={(value) => {
            set(withText("size", value), "title-size")
          }}
        />
        <ToggleGroup
          type="multiple"
          variant="outline"
          size="sm"
          aria-label={t("editor:title.style.label")}
          disabled={!offers("style")}
          value={styles}
          onValueChange={(values) => {
            set((current) => {
              const next = { ...current }
              for (const style of STYLES) {
                const on = values.includes(style.value)
                // Off is the default and not stored.
                if (on !== (current[style.value] === true)) next[style.value] = on ? true : null
              }
              return next
            })
          }}
        >
          {STYLES.map((style) => (
            <ToggleGroupItem
              key={style.value}
              value={style.value}
              aria-label={t(style.label)}
              title={t(style.label)}
              className={ICON_ITEM}
            >
              <style.icon />
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <div className="ml-auto">
          <ColorField
            compact
            label={t("editor:appearance.textColor")}
            value={shared.textColor}
            defaultValue={look.defaults.textColor}
            effectiveDefault={unset.text}
            disabled={!offers("color")}
            onChange={(value, group) => {
              set({ textColor: value }, group)
            }}
          />
        </div>
      </div>

      <div data-slot="title-row" className="flex items-center gap-2">
        <Select
          value={text.mode === "mixed" ? "" : (text.mode ?? "wrap")}
          disabled={!offers("layout")}
          onValueChange={(value) => {
            // Wrap is the default and not stored.
            set(withText("mode", value === "wrap" ? null : (value as TextMode)))
          }}
        >
          <SelectTrigger
            id={`${id}-mode`}
            size="sm"
            className="min-w-0 flex-1"
            aria-label={t("editor:text.mode.label")}
          >
            <SelectValue placeholder={t("editor:text.mixed")} />
          </SelectTrigger>
          <SelectContent>
            {TEXT_MODES.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {t(option.label)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-label={t("editor:text.align.label")}
          disabled={!offers("layout")}
          value={text.align === "mixed" ? "" : (text.align ?? "center")}
          onValueChange={(value) => {
            // Centred is the default and not stored.
            if (value) set(withText("align", value === "center" ? null : (value as TextAlign)))
          }}
        >
          {ALIGNS.map((option) => (
            <ToggleGroupItem
              key={option.value}
              value={option.value}
              aria-label={t(option.label)}
              title={t(option.label)}
              className={ICON_ITEM}
            >
              <option.icon />
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>

      <TextModeSettings
        text={text}
        preferred={preferred}
        disabled={!offers("layout")}
        onChange={set}
      />
      {report && <ReportNotice report={report} reduced={reduced} preferred={preferred} />}
    </>
  )
}
