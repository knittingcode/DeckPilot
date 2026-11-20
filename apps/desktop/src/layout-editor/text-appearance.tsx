// The text layout parts of the title appearance popover (docs/key-text.md):
// the mode, only the settings that mode uses, and what the device shows
// when the title does not simply fit.

import { useId, useState } from "react"

import { FieldError } from "@/components/field-error"
import { Input } from "@/components/ui/input"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import type { FitLines, TextLayoutReport, TextOverflow } from "@/lib/types"
import { cn } from "@/lib/utils"

import { type KeyLookChange, type Shared, type SharedAppearance, withText } from "./appearance"
import { TEXT_LIMITS } from "./title-look"

const DEFAULT_SPEED = 30
const DEFAULT_PAUSE = 1000
const DEFAULT_MIN_SIZE = 7

/**
 * The settings of the chosen mode, in one row: wrap's line limit, single
 * line's overflow, auto fit's minimum and lines, marquee's speed and pause.
 */
export function TextModeSettings({
  text,
  preferred,
  disabled,
  onChange,
}: {
  text: SharedAppearance["text"]
  /** The preferred size in points, for auto fit's check of its minimum. */
  preferred: number | undefined
  disabled?: boolean
  onChange: (change: KeyLookChange, group?: string) => void
}) {
  const { t } = useTranslation()
  const mode = text.mode
  const set =
    <K extends keyof SharedAppearance["text"]>(field: K) =>
    (value: Parameters<typeof withText<K>>[1], group?: string) => {
      onChange(withText(field, value), group)
    }
  if (mode === "mixed") return null
  const lines = (
    <NumberField
      label={t("editor:text.maxLines.label")}
      value={text.maxLines}
      range={TEXT_LIMITS.maxLines}
      placeholder={t("editor:text.auto")}
      disabled={disabled}
      onCommit={(value) => {
        set("maxLines")(value, "text-lines")
      }}
    />
  )
  return (
    <div data-slot="text-mode-settings" className="flex min-w-0 flex-wrap items-start gap-2">
      {(mode === "wrap" || mode === undefined) && lines}
      {mode === "singleLine" && (
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          disabled={disabled}
          aria-label={t("editor:text.overflow.label")}
          value={text.overflow === "mixed" ? "" : (text.overflow ?? "ellipsis")}
          onValueChange={(value) => {
            if (value) set("overflow")(value === "ellipsis" ? null : (value as TextOverflow))
          }}
        >
          <ToggleGroupItem value="ellipsis" className="px-2.5">
            {t("editor:text.overflow.ellipsis")}
          </ToggleGroupItem>
          <ToggleGroupItem value="clip" className="px-2.5">
            {t("editor:text.overflow.clip")}
          </ToggleGroupItem>
        </ToggleGroup>
      )}
      {mode === "autoFit" && (
        <>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            disabled={disabled}
            aria-label={t("editor:text.fit.label")}
            value={text.fit === "mixed" ? "" : (text.fit ?? "singleLine")}
            onValueChange={(value) => {
              if (value) set("fit")(value === "singleLine" ? null : (value as FitLines))
            }}
          >
            <ToggleGroupItem value="singleLine" className="px-2.5">
              {t("editor:text.fit.singleLine")}
            </ToggleGroupItem>
            <ToggleGroupItem value="multiline" className="px-2.5">
              {t("editor:text.fit.multiline")}
            </ToggleGroupItem>
          </ToggleGroup>
          <NumberField
            label={t("editor:text.minSize.label")}
            value={text.minSize}
            range={TEXT_LIMITS.size}
            placeholder={String(DEFAULT_MIN_SIZE)}
            disabled={disabled}
            problem={(value) =>
              preferred !== undefined && value > preferred
                ? t("editor:text.minSize.tooLarge")
                : null
            }
            onCommit={(value) => {
              set("minSize")(value, "text-min-size")
            }}
          />
          {text.fit === "multiline" && lines}
        </>
      )}
      {mode === "marquee" && (
        <>
          <NumberField
            label={t("editor:text.speed.custom")}
            value={text.speed}
            range={TEXT_LIMITS.speed}
            placeholder={String(DEFAULT_SPEED)}
            disabled={disabled}
            onCommit={(value) => {
              set("speed")(value, "text-speed")
            }}
          />
          <NumberField
            label={t("editor:text.pause.label")}
            value={text.pause}
            range={TEXT_LIMITS.pause}
            step={100}
            wide
            placeholder={String(DEFAULT_PAUSE)}
            disabled={disabled}
            onCommit={(value) => {
              set("pause")(value, "text-pause")
            }}
          />
        </>
      )}
    </div>
  )
}

/** What the device shows when the title does not simply fit. */
export function ReportNotice({
  report,
  reduced,
  preferred,
}: {
  report: TextLayoutReport
  reduced: boolean
  preferred: number | undefined
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const size = format.number(report.fontSize)
  let message: string | null = null
  let warning = false
  if (report.mode === "autoFit" && !report.fits) {
    warning = true
    message = t("editor:text.notice.doesNotFit", { size })
  } else if (report.needsScroll) {
    message = reduced ? t("editor:text.notice.scrollsReduced") : t("editor:text.notice.scrolls")
  } else if (report.truncated) {
    message =
      report.mode === "wrap"
        ? t("editor:text.notice.truncatedLines")
        : t("editor:text.notice.truncatedLine")
  } else if (
    report.mode === "autoFit" &&
    preferred !== undefined &&
    report.fontSize < preferred - 0.05
  ) {
    message = t("editor:text.notice.shrunk", { size })
  }
  if (!message) return null
  return (
    <p
      role="status"
      data-slot="text-fit-notice"
      className={
        warning ? "text-xs text-amber-700 dark:text-amber-400" : "text-xs text-muted-foreground"
      }
    >
      {message}
    </p>
  )
}

/**
 * A compact whole number setting (32px high) with its label before it:
 * empty uses the default (the placeholder); values outside `range` are
 * explained and not applied.
 */
export function NumberField({
  label,
  value,
  range,
  placeholder,
  step = 1,
  wide = false,
  hideLabel = false,
  disabled,
  problem,
  onCommit,
}: {
  label: string
  value: Shared<number>
  range: readonly [number, number]
  placeholder: string
  step?: number
  /** Room for five digits. */
  wide?: boolean
  /** The label for assistive technology only. */
  hideLabel?: boolean
  disabled?: boolean
  /** A warning about an accepted value. */
  problem?: (value: number) => string | null
  onCommit: (value: number | null) => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const id = useId()
  const shown = value === "mixed" || value === undefined ? "" : String(value)
  const [draft, setDraft] = useState<{ for: string; text: string } | null>(null)
  // A draft belongs to the value it was typed over; a new value (another
  // key, undo) replaces it.
  const text = draft && draft.for === shown ? draft.text : shown
  const [least, most] = range
  const parsed = text.trim() === "" ? null : Number(text)
  const invalid =
    parsed !== null && (!Number.isInteger(parsed) || parsed < least || parsed > most)
      ? t("editor:text.wholeNumber", { min: format.number(least), max: format.number(most) })
      : null
  const warning = !invalid && parsed !== null ? (problem?.(parsed) ?? null) : null
  const message = invalid ?? warning
  return (
    <div className="grid min-w-0 gap-1">
      <div className="flex items-center gap-1.5">
        <label
          htmlFor={id}
          className={cn("text-xs whitespace-nowrap text-muted-foreground", hideLabel && "sr-only")}
        >
          {label}
        </label>
        <Input
          id={id}
          type="number"
          inputMode="numeric"
          min={least}
          max={most}
          step={step}
          disabled={disabled}
          className={cn("h-8 px-2 tabular-nums", wide ? "w-20" : "w-14")}
          placeholder={value === "mixed" ? t("editor:text.mixed") : placeholder}
          value={text}
          aria-invalid={invalid ? true : undefined}
          aria-describedby={message ? `${id}-error` : undefined}
          onChange={(event) => {
            const next = event.target.value
            setDraft({ for: shown, text: next })
            const number = next.trim() === "" ? null : Number(next)
            if (number === null) {
              onCommit(null)
            } else if (Number.isInteger(number) && number >= least && number <= most) {
              onCommit(number)
            }
          }}
        />
      </div>
      <FieldError id={`${id}-error`} message={message} />
    </div>
  )
}
