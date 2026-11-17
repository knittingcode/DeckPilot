// The states of a Multi Action Switch (`deck.multi-switch`) as the agent
// stores them: one `<target> | <title> | <glyph>` per line, where the target
// is `action:<id>` or `workflow:<id>` and title and glyph may be empty (see
// modules/deck/src/switch.rs).

export const MULTI_SWITCH_TYPE = "deck.multi-switch"
export const STATES_PARAMETER = "states"
export const MIN_STATES = 2
export const MAX_STATES = 8

/** The renderer's glyphs, by the names profiles store. */
export const GLYPHS = [
  "check",
  "cross",
  "play",
  "pause",
  "stop",
  "timer",
  "bell",
  "up",
  "down",
  "dot",
  "circle",
  "square",
  "diamond",
  "triangle",
  "star",
  "heart",
] as const

export interface SwitchStateLine {
  /** `action:<id>` or `workflow:<id>`. */
  target: string
  title: string
  /** One of `GLYPHS`, or "" for none. */
  glyph: string
  /** The line as stored, when this version cannot read it. */
  raw?: string
}

/** The states of the stored text, in order; unreadable lines are kept as they are. */
export function parseStates(text: string): SwitchStateLine[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
    .map((line) => {
      const [target = "", title = "", glyph = "", ...rest] = line.split("|").map((p) => p.trim())
      const valid =
        /^(action|workflow):\S+$/.test(target) &&
        rest.length === 0 &&
        (glyph === "" || (GLYPHS as readonly string[]).includes(glyph))
      return valid ? { target, title, glyph } : { target: "", title: "", glyph: "", raw: line }
    })
}

/** The text to store: empty trailing parts are left out. */
export function formatStates(lines: SwitchStateLine[]): string {
  return lines
    .map((line) => {
      if (line.raw !== undefined) return line.raw
      // A title cannot hold the separator.
      const title = line.title.replaceAll("|", "/").trim()
      if (line.glyph) return `${line.target} |${title ? ` ${title}` : ""} | ${line.glyph}`
      if (title) return `${line.target} | ${title}`
      return line.target
    })
    .join("\n")
}
