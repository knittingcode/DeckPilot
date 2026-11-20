// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it } from "vitest"

import { i18n } from "@/i18n"
import { LANGUAGE_STORAGE_KEY } from "@/i18n/preference"
import { settingsPages } from "@/settings/registry"

import { LanguageSettings } from "./language"

afterEach(async () => {
  cleanup()
  localStorage.clear()
  await act(async () => {
    await i18n.changeLanguage("en")
  })
})

describe("language settings", () => {
  it("is the first page of General", () => {
    const general = settingsPages([]).filter((page) => page.category === "general")
    expect(general[0]).toMatchObject({ id: "language", title: "Language" })
  })

  it("offers the system default first and names each language in itself", async () => {
    const user = userEvent.setup()
    render(<LanguageSettings />)
    const radios = screen.getAllByRole("radio")
    expect(radios.map((radio) => radio.closest("label")?.textContent)).toEqual([
      "System default (recommended)",
      "English",
      "Polski",
    ])
    expect(screen.getByRole("radio", { name: "System default (recommended)" })).toBeChecked()

    await user.click(screen.getByRole("radio", { name: "Polski" }))
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe("pl")
    expect(screen.getByRole("radio", { name: "Polski" })).toBeChecked()
    // The page itself switches at once; the language names stay as they are.
    expect(screen.getByRole("radio", { name: "Domyślny systemowy (zalecane)" })).toBeInTheDocument()
    expect(screen.getByRole("radio", { name: "English" })).toBeInTheDocument()
    expect(settingsPages([]).find((page) => page.id === "language")?.title).toBe("Język")

    await user.click(screen.getByRole("radio", { name: "Domyślny systemowy (zalecane)" }))
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBeNull()
  })
})
