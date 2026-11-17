// The candidates of a Random Action (`deck.random`) as the agent stores
// them: one `action:<id>` or `workflow:<id>` per line, with an optional
// weight (see modules/deck/src/random.rs).

/** The action type and parameter this control replaces the text field of. */
export const RANDOM_TYPE = "deck.random"
export const CANDIDATES_PARAMETER = "candidates"
export const MAX_WEIGHT = 100

/** One line of the stored text: `action:<id>` or `workflow:<id>`, and a weight. */
export interface CandidateLine {
  /** `action:<id>`, `workflow:<id>`, or a line this version cannot read. */
  target: string
  weight: number
  /** The line as stored, when it is not a candidate this field understands. */
  raw?: string
}

/** The lines of the stored text, in order; unreadable lines are kept as they are. */
export function parseCandidates(text: string): CandidateLine[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
    .map((line) => {
      const [target = "", weight, ...rest] = line.split(/\s+/)
      const number = weight === undefined ? 1 : Number(weight)
      const valid =
        /^(action|workflow):\S+$/.test(target) &&
        rest.length === 0 &&
        Number.isInteger(number) &&
        number >= 1 &&
        number <= MAX_WEIGHT
      return valid ? { target, weight: number } : { target: "", weight: 1, raw: line }
    })
}

/** The text to store: one line per candidate, the weight only when it is not 1. */
export function formatCandidates(lines: CandidateLine[]): string {
  return lines
    .map((line) => line.raw ?? (line.weight === 1 ? line.target : `${line.target} ${line.weight}`))
    .join("\n")
}
