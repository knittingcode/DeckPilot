/**
 * Values of the structured text kinds (`lines`, `pairs`), which are stored
 * as plain text exactly as before — one item or `name<separator>value` per
 * line — and edited as lists and rows. Only an edit writes the value again;
 * list items round-trip exactly, and rows are written in the canonical form
 * (`Name: value`, `name=value`) the backend reads the same way.
 */

/** The items of a `lines` value: its non-empty lines, spaces kept. */
export function parseLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.replace(/\r$/, ""))
    .filter((line) => line.trim() !== "")
}

export function serializeLines(items: string[]): string {
  return items.join("\n")
}

export interface Pair {
  name: string
  value: string
}

/** Rows of a `pairs` value; a line without the separator is a name alone. */
export function parsePairs(text: string, separator: string): Pair[] {
  return parseLines(text).map((line) => {
    const at = line.indexOf(separator)
    return at < 0
      ? { name: line.trim(), value: "" }
      : { name: line.slice(0, at).trim(), value: line.slice(at + separator.length).trim() }
  })
}

/** `Name: value` for headers (`:`), `name=value` for query parameters. */
export function serializePairs(pairs: Pair[], separator: string): string {
  const join = separator === ":" ? ": " : separator
  return pairs.map((pair) => `${pair.name}${join}${pair.value}`).join("\n")
}

/**
 * Whether a header or parameter name looks like it holds a secret. Mirrors
 * `deckpilot_core::looks_secret`; the backend refuses these anyway.
 */
export function looksSecret(name: string): boolean {
  const lower = name.toLowerCase()
  if (
    [
      "authorization",
      "proxy-authorization",
      "cookie",
      "set-cookie",
      "proxy-authenticate",
      "key",
    ].includes(lower)
  )
    return true
  if (
    ["token", "secret", "api-key", "api_key", "apikey", "password", "session"].some((w) =>
      lower.includes(w),
    )
  )
    return true
  return lower.includes("auth") && !lower.includes("author")
}

/** Indexes of rows whose name repeats an earlier one. */
export function duplicateRows(pairs: Pair[], caseInsensitive: boolean): Set<number> {
  const seen = new Set<string>()
  const duplicates = new Set<number>()
  pairs.forEach((pair, index) => {
    const name = caseInsensitive ? pair.name.toLowerCase() : pair.name
    if (name === "") return
    if (seen.has(name)) duplicates.add(index)
    seen.add(name)
  })
  return duplicates
}

/** `null` when `text` is valid JSON (or empty), else why not. */
export function jsonProblem(text: string): string | null {
  if (text.trim() === "") return null
  try {
    JSON.parse(text)
    return null
  } catch (error) {
    return error instanceof Error ? error.message : "not valid JSON"
  }
}

/** Credential names: lower case letters, digits, `.`, `_` and `-`. */
export const CREDENTIAL_NAME = /^[a-z][a-z0-9._-]{0,63}$/
