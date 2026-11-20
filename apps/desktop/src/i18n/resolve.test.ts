import { describe, expect, it } from "vitest"

import { resolveLocale } from "./resolve"

describe("resolveLocale", () => {
  it.each([
    [["pl-PL"], "pl"],
    [["en-GB"], "en"],
    [["de-DE", "pl-PL"], "pl"],
    [["de-DE"], "en"],
    [[], "en"],
    // Exact tags, case and POSIX spellings.
    [["pl"], "pl"],
    [["PL_pl.UTF-8"], "pl"],
    // The first supported language wins, not the "best" one.
    [["en-US", "pl-PL"], "en"],
    [["fr-CA", "de-AT", "pl"], "pl"],
    // Empty and odd entries are skipped.
    [["", "  ", "pl-PL"], "pl"],
  ])("%j → %s", (preferred, expected) => {
    expect(resolveLocale(preferred)).toBe(expected)
  })

  it("prefers an exact regional match over the language alone", () => {
    expect(resolveLocale(["pt-BR"], ["pt", "pt-BR"], "pt")).toBe("pt-BR")
    expect(resolveLocale(["pt-PT"], ["pt", "pt-BR"], "pt")).toBe("pt")
  })
})
