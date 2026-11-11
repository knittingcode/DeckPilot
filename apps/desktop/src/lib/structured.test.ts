import { describe, expect, it } from "vitest"

import {
  duplicateRows,
  jsonProblem,
  looksSecret,
  parseLines,
  parsePairs,
  serializeLines,
  serializePairs,
} from "./structured"

describe("structured text values", () => {
  it("keeps list items exactly and round-trips the stored text", () => {
    const stored = "--name\nJohn Smith\n  indented  \n; rm -rf /"
    const items = parseLines(stored)
    expect(items).toEqual(["--name", "John Smith", "  indented  ", "; rm -rf /"])
    expect(serializeLines(items)).toBe(stored)
    // Empty lines are no items, as the backend sees them.
    expect(parseLines("a\r\n\n   \nb")).toEqual(["a", "b"])
  })

  it("parses and writes rows in the stored format", () => {
    expect(parsePairs("Accept: application/json\nX-Trace:1", ":")).toEqual([
      { name: "Accept", value: "application/json" },
      { name: "X-Trace", value: "1" },
    ])
    expect(serializePairs([{ name: "Accept", value: "a: b" }], ":")).toBe("Accept: a: b")
    expect(serializePairs([{ name: "q", value: "a=b" }], "=")).toBe("q=a=b")
    expect(parsePairs("q=a=b", "=")).toEqual([{ name: "q", value: "a=b" }])
  })

  it("finds duplicates and names that look secret", () => {
    const rows = [
      { name: "Accept", value: "1" },
      { name: "accept", value: "2" },
      { name: "", value: "" },
    ]
    expect([...duplicateRows(rows, true)]).toEqual([1])
    expect([...duplicateRows(rows, false)]).toEqual([])
    for (const secret of ["Authorization", "X-Api-Key", "access_token", "Cookie", "key"]) {
      expect(looksSecret(secret)).toBe(true)
    }
    for (const plain of ["Accept", "X-Author", "page"]) expect(looksSecret(plain)).toBe(false)
  })

  it("checks JSON bodies", () => {
    expect(jsonProblem("")).toBeNull()
    expect(jsonProblem('{"a": 1}')).toBeNull()
    expect(jsonProblem("{nope")).not.toBeNull()
  })
})
