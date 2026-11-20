import { RotateCcw } from "lucide-react"
import { useId, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

import { SWATCHES, type Shared, isAssetBackground, parseHex } from "./appearance"

const TRANSPARENT = "transparent"

/** A checkerboard: how a transparent swatch looks. */
const CHECKERBOARD = {
  backgroundImage:
    "linear-gradient(45deg, #a3a3a3 25%, transparent 25%, transparent 75%, #a3a3a3 75%), linear-gradient(45deg, #a3a3a3 25%, #f5f5f5 25%, #f5f5f5 75%, #a3a3a3 75%)",
  backgroundSize: "8px 8px",
  backgroundPosition: "0 0, 4px 4px",
}

/**
 * A key colour: swatches, a hex field and a reset to the device model's
 * default (`defaultValue`, shown when nothing is set). With
 * `allowTransparent`, a transparent swatch too. Typing in the hex field
 * forms one undo step (`group`). `compact`: a 32px swatch button that opens
 * the same field in a popover (the key header, the title popover).
 */
export function ColorField({
  label,
  value,
  defaultValue,
  effectiveDefault,
  transparentShows,
  allowTransparent = false,
  compact = false,
  disabled = false,
  onChange,
}: {
  label: string
  value: Shared<string>
  /** The model's default: `#rrggbb` or `transparent`. */
  defaultValue?: string
  /** The colour an unset value is drawn with (opaque `#rrggbb`). */
  effectiveDefault?: string
  /** The colour a transparent value is drawn with: the display's. */
  transparentShows?: string
  allowTransparent?: boolean
  compact?: boolean
  disabled?: boolean
  onChange: (value: string | null, group?: string) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const current = value === "mixed" ? undefined : value
  const transparent = current === TRANSPARENT
  // A library asset (a gradient, an image) is no colour: the field starts
  // empty and stores a colour only once one is chosen.
  const hex = transparent || isAssetBackground(current) ? undefined : current
  // What was typed, and the value it was typed against. An outside change
  // (undo, another key selected) replaces the text.
  const [typed, setTyped] = useState({ text: hex ?? "", base: current })
  const text = typed.base === current ? typed.text : (hex ?? "")
  const invalid = text.trim() !== "" && parseHex(text) === null
  const named = (colour: string | undefined) =>
    colour === TRANSPARENT ? t("editor:color.transparent") : colour
  const placeholder =
    value === "mixed"
      ? t("editor:color.mixed")
      : transparent
        ? t("editor:color.transparent")
        : defaultValue
          ? t("editor:color.defaultIs", { value: named(defaultValue) })
          : t("editor:color.default")
  const picked = transparent
    ? (transparentShows ?? effectiveDefault ?? "#000000")
    : (hex ?? effectiveDefault ?? "#000000")
  // What the swatch button shows: the colour as drawn, a checkerboard for
  // transparent (set, or the default).
  const showsTransparent = transparent || (value === undefined && defaultValue === TRANSPARENT)
  const fieldset = (
    <fieldset className="grid gap-2" disabled={disabled}>
      <legend className={cn("mb-1 text-sm font-medium", compact && "sr-only")}>{label}</legend>
      <div
        className="flex flex-wrap gap-1"
        role="group"
        aria-label={t("editor:color.swatches", { label })}
      >
        {allowTransparent && (
          <button
            type="button"
            aria-label={t("editor:color.transparent")}
            aria-pressed={transparent}
            title={t("editor:color.transparentTitle")}
            className={cn(
              "size-6 rounded-md border focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
              transparent && "ring-2 ring-primary ring-offset-1",
            )}
            style={CHECKERBOARD}
            onClick={() => {
              onChange(TRANSPARENT)
            }}
          />
        )}
        {SWATCHES.map((swatch) => (
          <button
            key={swatch.value}
            type="button"
            aria-label={t(swatch.name)}
            aria-pressed={current === swatch.value}
            title={t("editor:color.swatchTitle", { name: t(swatch.name), value: swatch.value })}
            className={cn(
              "size-6 rounded-md border focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
              current === swatch.value && "ring-2 ring-primary ring-offset-1",
            )}
            style={{ backgroundColor: swatch.value }}
            onClick={() => {
              onChange(swatch.value)
            }}
          />
        ))}
      </div>
      <div className="flex items-center gap-2">
        <input
          type="color"
          aria-label={t("editor:color.picker", { label })}
          className="h-8 w-10 cursor-pointer rounded border bg-transparent p-0.5"
          value={picked}
          onChange={(e) => {
            onChange(e.target.value, `appearance:${label}`)
          }}
        />
        <Input
          id={id}
          aria-label={t("editor:color.hex", { label })}
          aria-invalid={invalid}
          aria-describedby={invalid ? `${id}-error` : undefined}
          className="h-8 min-w-0 flex-1 font-mono"
          placeholder={placeholder}
          value={text}
          maxLength={7}
          onChange={(e) => {
            const next = e.target.value
            const parsed = parseHex(next)
            const empty = next.trim() === ""
            setTyped({ text: next, base: parsed ?? (empty ? undefined : current) })
            if (parsed) onChange(parsed, `appearance:${label}`)
            else if (empty) onChange(null, `appearance:${label}`)
          }}
        />
        <Button
          type="button"
          variant="ghost"
          size={compact ? "icon-sm" : "sm"}
          aria-label={compact ? t("editor:color.default") : undefined}
          title={compact ? t("editor:color.default") : undefined}
          disabled={value === undefined}
          onClick={() => {
            onChange(null)
          }}
        >
          {compact ? <RotateCcw /> : t("editor:color.default")}
        </Button>
      </div>
      {invalid && (
        <p id={`${id}-error`} className="text-xs text-destructive">
          {t("editor:color.invalid")}
        </p>
      )}
    </fieldset>
  )
  if (!compact) return fieldset
  const shownName =
    value === "mixed"
      ? t("editor:color.mixed")
      : current
        ? named(current)
        : t("editor:color.defaultIs", { value: named(defaultValue ?? effectiveDefault) })
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          data-slot="color-swatch-trigger"
          data-inherited={value === undefined ? "" : undefined}
          aria-label={t("editor:color.swatchButton", { label, value: shownName })}
          title={`${label}: ${shownName ?? ""}`}
          className="size-8 shrink-0 rounded-md border border-input p-1 shadow-xs focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none disabled:opacity-50"
        >
          <span
            aria-hidden
            className={cn(
              "block size-full rounded-sm border border-white/15",
              value === "mixed" && "bg-[linear-gradient(135deg,#737373_50%,#d4d4d4_50%)]",
            )}
            style={
              value === "mixed"
                ? undefined
                : showsTransparent
                  ? CHECKERBOARD
                  : { backgroundColor: picked }
            }
          />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        collisionPadding={8}
        data-slot="color-popover"
        className="w-72 p-3"
      >
        {fieldset}
      </PopoverContent>
    </Popover>
  )
}
