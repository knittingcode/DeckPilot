import { afterAll, describe, expect, it } from "vitest"

import { i18n } from "./index"

afterAll(async () => {
  await i18n.changeLanguage("en")
})

describe("plurals", () => {
  it("uses the Polish forms one, few and many", async () => {
    await i18n.changeLanguage("pl")
    const projects = (count: number) => i18n.t("common:counts.projects", { count })
    expect(projects(1)).toBe("1 projekt")
    expect(projects(2)).toBe("2 projekty")
    expect(projects(5)).toBe("5 projektów")
    expect(projects(12)).toBe("12 projektów")
    expect(projects(22)).toBe("22 projekty")
    expect(projects(25)).toBe("25 projektów")
  })

  it("uses one and other in English", async () => {
    await i18n.changeLanguage("en")
    expect(i18n.t("common:counts.projects", { count: 1 })).toBe("1 project")
    expect(i18n.t("common:counts.projects", { count: 22 })).toBe("22 projects")
  })

  it("resolves regional tags to the language", async () => {
    await i18n.changeLanguage("pl-PL")
    expect(i18n.resolvedLanguage).toBe("pl")
    expect(i18n.t("common:actions.cancel")).toBe("Anuluj")
  })

  it("falls back to English for languages it does not have", async () => {
    await i18n.changeLanguage("de")
    expect(i18n.t("common:actions.cancel")).toBe("Cancel")
  })
})
