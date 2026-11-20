import { afterEach, describe, expect, it } from "vitest"

import { i18n } from "@/i18n"

import { moduleMatches, moduleToggle } from "./modules"
import { moduleInfo } from "./test-fixtures"

describe("module toggles", () => {
  const base = moduleInfo({ id: "base" })
  const addon = moduleInfo({ id: "addon", dependencies: ["base"] })

  it("allows switching independent modules", () => {
    expect(moduleToggle(base, [base])).toEqual({ allowed: true, reason: null })
  })

  it("explains that a required module cannot be disabled", () => {
    const toggle = moduleToggle(base, [base, addon])
    expect(toggle.allowed).toBe(false)
    expect(toggle.reason).toBe("Required by Addon. Disable it first.")
    // A disabled dependent does not block.
    expect(moduleToggle(base, [base, { ...addon, enabled: false }]).allowed).toBe(true)
  })

  it("explains that a module needs its dependencies enabled first", () => {
    const disabledBase = { ...base, enabled: false }
    const disabledAddon = { ...addon, enabled: false }
    expect(moduleToggle(disabledAddon, [disabledBase, disabledAddon])).toEqual({
      allowed: false,
      reason: "Requires Base. Enable it first.",
    })
    expect(moduleToggle(disabledAddon, [disabledAddon]).reason).toBe(
      "Requires base, which is not installed.",
    )
    expect(moduleToggle(disabledAddon, [base, disabledAddon]).allowed).toBe(true)
  })

  it("does not offer modules this system does not support", () => {
    const other = moduleInfo({ id: "other", enabled: false, supported: false })
    expect(moduleToggle(other, [other]).reason).toBe("Not available on this operating system.")
  })
})

describe("module search", () => {
  const projects = moduleInfo({
    id: "projects",
    actionTypes: [
      {
        id: "projects.switch",
        name: "Switch project",
        description: "",
        category: "Projects",
        parameters: [],
        requiresProject: false,
      },
    ],
  })
  const off = moduleInfo({ id: "general", enabled: false })
  const unsupported = moduleInfo({ id: "windows", enabled: false, supported: false })

  it("filters by state", () => {
    const all = [projects, off, unsupported]
    expect(all.filter((m) => moduleMatches(m, "enabled", "")).map((m) => m.id)).toEqual([
      "projects",
    ])
    expect(all.filter((m) => moduleMatches(m, "disabled", "")).map((m) => m.id)).toEqual([
      "general",
    ])
    expect(all.filter((m) => moduleMatches(m, "unavailable", "")).map((m) => m.id)).toEqual([
      "windows",
    ])
    expect(all.filter((m) => moduleMatches(m, "all", ""))).toHaveLength(3)
  })

  it("searches names, descriptions and action types", () => {
    expect(moduleMatches(projects, "all", "switch")).toBe(true)
    expect(moduleMatches(projects, "all", "PROJECTS module")).toBe(true)
    expect(moduleMatches(projects, "all", "brightness")).toBe(false)
  })
})

describe("module toggles in Polish", () => {
  afterEach(async () => {
    await i18n.changeLanguage("en")
  })

  it("uses the plural forms and the language's list", async () => {
    await i18n.changeLanguage("pl")
    const base = moduleInfo({ id: "base" })
    const dependent = (id: string) => moduleInfo({ id, dependencies: ["base"] })
    const one = [base, dependent("git")]
    expect(moduleToggle(base, one).reason).toBe("Wymagany przez moduł Git. Najpierw go wyłącz.")
    const two = [base, dependent("git"), dependent("docker")]
    expect(moduleToggle(base, two).reason).toBe(
      "Wymagany przez moduły Git i Docker. Najpierw je wyłącz.",
    )
    const missing = moduleInfo({ id: "addon", enabled: false, dependencies: ["a", "b", "c"] })
    expect(moduleToggle(missing, [missing]).reason).toBe(
      "Wymaga modułów a, b i c, które nie są zainstalowane.",
    )
    const other = moduleInfo({ id: "other", enabled: false, supported: false })
    expect(moduleToggle(other, [other]).reason).toBe("Niedostępny w tym systemie operacyjnym.")
  })

  it("names several modules with the English list", () => {
    const base = moduleInfo({ id: "base" })
    const dependents = ["git", "docker"].map((id) => moduleInfo({ id, dependencies: ["base"] }))
    expect(moduleToggle(base, [base, ...dependents]).reason).toBe(
      "Required by Git and Docker. Disable them first.",
    )
  })
})
