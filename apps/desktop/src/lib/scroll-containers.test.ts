import { describe, expect, it } from "vitest"

/**
 * Every scroll container must also be the containing block of its content
 * (`relative`, or itself positioned). Otherwise absolutely positioned
 * descendants — visually hidden inputs, `sr-only` text, badges — are not
 * clipped by it and stretch the next scroll container up instead: the
 * Device Editor's workspace scrolled into blank space below the inspector
 * because of the icon file input (November 2026). The live layout check
 * (scripts/layout-check.mjs) verifies the running app; this keeps new code
 * from reintroducing the pattern.
 */

// Every component source except the shadcn/ui primitives, which position
// their own content, and tests.
const SOURCES = import.meta.glob<string>(
  ["../**/*.tsx", "!../components/ui/**", "!../**/*.test.tsx"],
  { query: "?raw", import: "default", eager: true },
)

const SCROLLS = /(^|\s)(\w+:)*overflow(-y|-x)?-(auto|scroll)(\s|$)/
const POSITIONED = /(^|\s)(relative|absolute|fixed|sticky)(\s|$)/

describe("scroll containers", () => {
  it("are the containing block of their content", () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(50)
    const offenders: string[] = []
    for (const [file, text] of Object.entries(SOURCES)) {
      for (const match of text.matchAll(/className=(?:\{cn\()?"([^"]*)"/g)) {
        const classes = match[1] ?? ""
        if (SCROLLS.test(classes) && !POSITIONED.test(classes)) {
          const line = text.slice(0, match.index).split("\n").length
          offenders.push(`${file.replace(/^(?:\.\.\/)+/, "")}:${line}: "${classes}"`)
        }
      }
    }
    expect(offenders, "add `relative` to these scroll containers").toEqual([])
  })
})
