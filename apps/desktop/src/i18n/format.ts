// Dates, numbers, sizes and lists in the window's language, with `Intl`.
// Every function takes the locale explicitly; `useFormat()` binds them to
// the language on screen (and re-renders when it changes).

import { useMemo } from "react"
import { useTranslation } from "react-i18next"

import { type Language, isLanguage } from "./config"
import { currentLanguage } from "./index"

type DateInput = Date | number

const toDate = (value: DateInput) => (typeof value === "number" ? new Date(value) : value)

export function formatDate(
  locale: string,
  value: DateInput,
  options: Intl.DateTimeFormatOptions = { dateStyle: "medium" },
) {
  return new Intl.DateTimeFormat(locale, options).format(toDate(value))
}

export function formatTime(
  locale: string,
  value: DateInput,
  options: Intl.DateTimeFormatOptions = { timeStyle: "short" },
) {
  return new Intl.DateTimeFormat(locale, options).format(toDate(value))
}

export function formatDateTime(
  locale: string,
  value: DateInput,
  options: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" },
) {
  return new Intl.DateTimeFormat(locale, options).format(toDate(value))
}

export function formatNumber(locale: string, value: number, options?: Intl.NumberFormatOptions) {
  return new Intl.NumberFormat(locale, options).format(value)
}

/** `ratio` 0–1, e.g. 0.42 → "42%" (en) or "42%" (pl, with a no-break space). */
export function formatPercent(locale: string, ratio: number, maximumFractionDigits = 0) {
  return new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits }).format(ratio)
}

const SIZE_UNITS = ["byte", "kilobyte", "megabyte", "gigabyte", "terabyte"] as const

/** Bytes in binary steps (1 KB = 1024 bytes), e.g. "1.5 MB" / "1,5 MB". */
export function formatFileSize(locale: string, bytes: number) {
  let value = Math.max(0, bytes)
  let unit = 0
  while (value >= 1024 && unit < SIZE_UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  return new Intl.NumberFormat(locale, {
    style: "unit",
    unit: SIZE_UNITS[unit],
    unitDisplay: "short",
    maximumFractionDigits: unit === 0 || value >= 100 ? 0 : 1,
  }).format(value)
}

const RELATIVE_STEPS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["second", 60],
  ["minute", 60],
  ["hour", 24],
  ["day", 7],
  ["week", 4.345],
  ["month", 12],
  ["year", Number.POSITIVE_INFINITY],
]

/** `value` relative to `now`, e.g. "5 minutes ago" / "5 minut temu", "tomorrow" / "jutro". */
export function formatRelativeTime(locale: string, value: DateInput, now: DateInput = Date.now()) {
  let delta = (toDate(value).getTime() - toDate(now).getTime()) / 1000
  const format = new Intl.RelativeTimeFormat(locale, { numeric: "auto" })
  for (const [unit, size] of RELATIVE_STEPS) {
    if (Math.abs(delta) < size) return format.format(Math.round(delta), unit)
    delta /= size
  }
  return format.format(Math.round(delta), "year")
}

/** "a, b and c" / "a, b i c". */
export function formatList(
  locale: string,
  items: readonly string[],
  type: Intl.ListFormatType = "conjunction",
) {
  return new Intl.ListFormat(locale, { style: "long", type }).format(items)
}

/** Sorts by the language's alphabet (Polish: a, ą, b, c, ć, …). */
export function collator(locale: string, options?: Intl.CollatorOptions) {
  return new Intl.Collator(locale, { sensitivity: "base", numeric: true, ...options })
}

export interface Format {
  locale: Language
  date: (value: DateInput, options?: Intl.DateTimeFormatOptions) => string
  time: (value: DateInput, options?: Intl.DateTimeFormatOptions) => string
  dateTime: (value: DateInput, options?: Intl.DateTimeFormatOptions) => string
  number: (value: number, options?: Intl.NumberFormatOptions) => string
  percent: (ratio: number, maximumFractionDigits?: number) => string
  fileSize: (bytes: number) => string
  relativeTime: (value: DateInput, now?: DateInput) => string
  list: (items: readonly string[], type?: Intl.ListFormatType) => string
  compare: (a: string, b: string) => number
}

/** The formatters for `locale`. */
export function formatter(locale: Language = currentLanguage()): Format {
  const compare = collator(locale)
  return {
    locale,
    date: (value, options) => formatDate(locale, value, options),
    time: (value, options) => formatTime(locale, value, options),
    dateTime: (value, options) => formatDateTime(locale, value, options),
    number: (value, options) => formatNumber(locale, value, options),
    percent: (ratio, digits) => formatPercent(locale, ratio, digits),
    fileSize: (bytes) => formatFileSize(locale, bytes),
    relativeTime: (value, now) => formatRelativeTime(locale, value, now),
    list: (items, type) => formatList(locale, items, type),
    compare: (a, b) => compare.compare(a, b),
  }
}

/** The formatters of the language on screen. */
export function useFormat(): Format {
  const { i18n } = useTranslation()
  const language = i18n.resolvedLanguage ?? i18n.language
  const locale = isLanguage(language) ? language : currentLanguage()
  return useMemo(() => formatter(locale), [locale])
}
