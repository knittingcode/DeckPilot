// The values a plugin dialog's form sends (docs/dialogs.md): typed, within
// the limits the agent checks again (it refuses what does not fit).

import { i18n } from "@/i18n"

import type { PluginDialogField, PluginDialogValue } from "./types"

/** Longest a text field is without its own `maxLength` (the agent's limit). */
export const DEFAULT_MAX_LENGTH = 2000
/** Most lines of a `lines` field, and the longest line. */
export const MAX_LINES = 100
export const MAX_LINE_LENGTH = 200
/** Largest answer the agent accepts: the UTF-8 bytes of the values' JSON. */
export const MAX_ANSWER_BYTES = 49152

/** Why the whole answer cannot be sent. */
export function tooLongText(): string {
  return i18n.t("plugins:dialog.tooLong")
}

/** Characters as the agent counts them (Unicode scalar values). */
function characters(text: string): number {
  return Array.from(text).length
}

/** What a field holds while it is edited: typed text, a switch or a choice. */
export type Draft = Record<string, string | boolean>

export function initialDraft(fields: PluginDialogField[]): Draft {
  const draft: Draft = {}
  for (const field of fields) {
    const value = field.default
    switch (field.kind) {
      case "boolean":
        draft[field.id] = value === true
        break
      case "lines":
        draft[field.id] = Array.isArray(value) ? value.join("\n") : ""
        break
      case "integer":
        draft[field.id] = typeof value === "number" ? String(value) : ""
        break
      default:
        draft[field.id] = typeof value === "string" ? value : ""
    }
  }
  return draft
}

export function linesOf(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
}

/**
 * The field's value to send (`undefined`: left empty), or why it cannot be
 * sent (`error`). An empty required field has no error text: it only keeps
 * the primary button disabled.
 */
export function fieldValue(
  field: PluginDialogField,
  draft: string | boolean | undefined,
): { value?: PluginDialogValue; error?: string; missing?: boolean } {
  if (field.kind === "boolean") return { value: draft === true }
  const text = typeof draft === "string" ? draft : ""
  switch (field.kind) {
    case "text": {
      if (text.trim() === "") return field.required ? { missing: true } : {}
      const max = field.maxLength ?? DEFAULT_MAX_LENGTH
      if (characters(text) > max)
        return { error: i18n.t("plugins:dialog.error.maxCharacters", { count: max }) }
      return { value: text }
    }
    case "integer": {
      const trimmed = text.trim()
      if (trimmed === "") return field.required ? { missing: true } : {}
      const number = /^-?\d+$/.test(trimmed) ? Number(trimmed) : Number.NaN
      if (!Number.isSafeInteger(number))
        return { error: i18n.t("plugins:dialog.error.wholeNumber") }
      if (field.min !== undefined && number < field.min)
        return { error: i18n.t("plugins:dialog.error.atLeast", { min: field.min }) }
      if (field.max !== undefined && number > field.max)
        return { error: i18n.t("plugins:dialog.error.atMost", { max: field.max }) }
      return { value: number }
    }
    case "choice": {
      if (text === "") return field.required ? { missing: true } : {}
      if (!(field.options ?? []).some((o) => o.value === text))
        return { error: i18n.t("plugins:dialog.error.chooseOption") }
      return { value: text }
    }
    case "lines": {
      const lines = linesOf(text)
      if (lines.length === 0) return field.required ? { missing: true } : {}
      if (lines.length > MAX_LINES)
        return { error: i18n.t("plugins:dialog.error.maxLines", { count: MAX_LINES }) }
      if (lines.some((line) => characters(line) > MAX_LINE_LENGTH))
        return { error: i18n.t("plugins:dialog.error.lineLength", { count: MAX_LINE_LENGTH }) }
      return { value: lines }
    }
  }
}
