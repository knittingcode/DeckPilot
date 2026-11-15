// The settings form of a plugin, generated from the fields the agent
// returns with `getPluginSettings` (the manifest's
// `contributes.settings.fields`). Pure helpers: what each field is edited
// with, the draft the form holds, what `setPluginSettings` is sent, and
// which field each problem of the agent's `invalidData` refusal belongs to.
//
// The agent is the validator (the core's `ActionType::normalize`); the
// form only refuses what it cannot send at all (a whole number that is
// not one). Settings are never secret: secrets are credential slots, so
// no field here is ever a password input.

import type { PluginSettingField, PluginSettings, PluginSettingValue } from "./types"

/** How a field is edited. */
export type SettingControl =
  | { type: "text"; maxLength?: number }
  | { type: "multiline"; maxLength?: number; monospace: boolean }
  | { type: "integer"; min?: number; max?: number }
  | { type: "boolean" }
  | { type: "choice"; options: string[] }
  /** A kind the app cannot edit (`project`, or one it does not know). */
  | { type: "unsupported"; kind: string }

const num = (v: unknown): number | undefined => (typeof v === "number" ? v : undefined)

export function settingControl(field: PluginSettingField): SettingControl {
  const kind = field.kind
  const maxLength = num(kind.maxLength)
  const limit = maxLength === undefined ? {} : { maxLength }
  switch (kind.type) {
    case "text":
      return kind.multiline === true
        ? { type: "multiline", monospace: false, ...limit }
        : { type: "text", ...limit }
    case "lines":
    case "pairs":
      return { type: "multiline", monospace: false, ...limit }
    case "code":
      return { type: "multiline", monospace: true, ...limit }
    case "integer": {
      const min = num(kind.min)
      const max = num(kind.max)
      return {
        type: "integer",
        ...(min === undefined ? {} : { min }),
        ...(max === undefined ? {} : { max }),
      }
    }
    case "boolean":
      return { type: "boolean" }
    case "choice":
      return {
        type: "choice",
        options: Array.isArray(kind.options)
          ? kind.options.filter((o): o is string => typeof o === "string")
          : [],
      }
    default:
      return { type: "unsupported", kind: kind.type }
  }
}

/**
 * One field of the form: `value` is what the user sees and edits; `null`
 * means "no saved value" (the default applies). Booleans and choices hold
 * their value, everything else the typed text.
 */
export type SettingDraft = Record<string, string | boolean | null>

/** The draft of the saved values: what the user saved, nothing for the rest. */
export function draftOf(settings: PluginSettings): SettingDraft {
  return Object.fromEntries(
    settings.fields.map((f) => {
      const saved = settings.values[f.key]
      if (saved === undefined) return [f.key, null]
      return [f.key, typeof saved === "boolean" ? saved : String(saved)]
    }),
  )
}

/** The default of a field as text, for placeholders and "Default: …". */
export function defaultText(field: PluginSettingField): string | null {
  if (field.default === undefined) return null
  if (typeof field.default === "boolean") return field.default ? "On" : "Off"
  return String(field.default)
}

function sameValue(a: PluginSettingValue | undefined, b: PluginSettingValue | null): boolean {
  return a === undefined ? b === null : a === b
}

/** A draft value as what the agent stores, or the reason it cannot be sent. */
function parse(
  field: PluginSettingField,
  draft: string | boolean | null,
): { value: PluginSettingValue | null } | { error: string } {
  if (draft === null) return { value: null }
  const control = settingControl(field)
  if (typeof draft === "boolean") return { value: draft }
  // An empty field means "no value": the default applies again.
  if (draft === "") return { value: null }
  if (control.type === "integer") {
    const trimmed = draft.trim()
    const n = Number(trimmed)
    if (trimmed === "" || !Number.isSafeInteger(n)) return { error: "Enter a whole number." }
    return { value: n }
  }
  return { value: draft }
}

/**
 * What `setPluginSettings` is sent for `draft`: only the settings that
 * changed, `null` for those reset to the default. `errors` are the fields
 * that cannot be sent as they are.
 */
export function settingsChanges(
  settings: PluginSettings,
  draft: SettingDraft,
): { values: Record<string, PluginSettingValue | null>; errors: Record<string, string> } {
  const values: Record<string, PluginSettingValue | null> = {}
  const errors: Record<string, string> = {}
  for (const field of settings.fields) {
    if (settingControl(field).type === "unsupported") continue
    const parsed = parse(field, draft[field.key] ?? null)
    if ("error" in parsed) {
      errors[field.key] = parsed.error
      continue
    }
    if (!sameValue(settings.values[field.key], parsed.value)) values[field.key] = parsed.value
  }
  return { values, errors }
}

/**
 * The problems of the agent's refusal (`the settings of <id> do not match
 * their declaration: parameter \`a\`: …; \`b\` expects text`), each under
 * the field it names; the rest under `form`.
 */
export function settingsProblems(
  message: string,
  fields: PluginSettingField[],
): { fields: Record<string, string>; form: string[] } {
  const keys = new Set(fields.map((f) => f.key))
  const marker = "do not match their declaration: "
  const at = message.indexOf(marker)
  const list = at < 0 ? [message] : message.slice(at + marker.length).split("; ")
  const out: { fields: Record<string, string>; form: string[] } = { fields: {}, form: [] }
  for (const problem of list) {
    const key = /`([^`]+)`/.exec(problem)?.[1]
    if (key !== undefined && keys.has(key)) {
      const text = problem
        .replace(/^parameter /, "")
        .replace(`\`${key}\``, "")
        .replace(/^[:\s]+/, "")
      const readable = text.charAt(0).toUpperCase() + text.slice(1)
      out.fields[key] = out.fields[key] ? `${out.fields[key]} ${readable}` : readable
    } else if (problem.trim() !== "") {
      out.form.push(problem)
    }
  }
  return out
}

/** The labels of the required settings the plugin does not get (`missing`). */
export function missingLabels(settings: PluginSettings): string[] {
  return (settings.missing ?? []).map(
    (key) => settings.fields.find((f) => f.key === key)?.label ?? key,
  )
}
