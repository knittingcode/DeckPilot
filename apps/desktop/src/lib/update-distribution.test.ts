import { afterEach, describe, expect, it } from "vitest"

import { i18n } from "@/i18n"
import { updateState } from "@/test/fixtures"

import { CAPABILITIES } from "./capabilities"
import { describeLayout, osLabel } from "./format"
import {
  deliveredBy,
  describeUpdateError,
  installedFrom,
  notConfiguredExplanation,
  providerLabel,
} from "./update-distribution"

afterEach(async () => {
  await i18n.changeLanguage("en")
})

describe("update texts", () => {
  it("names channels, formats and providers in English", () => {
    expect(installedFrom(updateState("homebrew"))).toBe("Homebrew")
    expect(deliveredBy(updateState("macAppStore"))).toBe(
      "Updates are delivered by the Mac App Store",
    )
    expect(providerLabel(updateState("apt"))).toBe("Your package manager (APT)")
    expect(
      notConfiguredExplanation("the compiled package format deb differs from the bundled appimage"),
    ).toBe(
      "This build was made as a .deb package but is installed as AppImage, so it does not update itself: an update could replace it with the wrong package. Reinstall DeckPilot from where you got it.",
    )
  })

  it("follows the window's language", async () => {
    await i18n.changeLanguage("pl")
    expect(installedFrom(updateState("apt"))).toBe("Repozytorium APT")
    expect(deliveredBy(updateState("macAppStore"))).toBe("Aktualizacje dostarcza Mac App Store")
    expect(providerLabel(updateState("apt"))).toBe("Twój menedżer pakietów (APT)")
    expect(
      notConfiguredExplanation("GitHub updates do not install msi packages on this platform"),
    ).toMatch(/^Aktualizacje z GitHub nie mogą instalować pakietów MSI na tej platformie/)
  })

  it("titles an update error in Polish and keeps the backend's words", async () => {
    await i18n.changeLanguage("pl")
    const described = describeUpdateError(
      "update download failed: the signature of the update is not valid (bad key id)",
    )
    expect(described).toEqual({
      title: "Odrzucono aktualizację: podpis jest nieprawidłowy",
      message: "The signature of the update is not valid.",
      detail: "bad key id",
    })
    expect(describeUpdateError("update check failed: offline").title).toBe(
      "Nie udało się sprawdzić dostępności aktualizacji",
    )
  })
})

describe("other helper texts in Polish", () => {
  it("translates a permission description, a layout state and an operating system", async () => {
    expect(i18n.t(CAPABILITIES.network.title)).toBe("Send network requests")
    await i18n.changeLanguage("pl")
    expect(i18n.t(CAPABILITIES.network.description)).toBe(
      "Wysyła żądania HTTP. Żądania do tego komputera lub do sieci lokalnej muszą być dozwolone osobno.",
    )
    expect(describeLayout({ type: "compatible", layout: "1", name: "Główny" })).toBe(
      "Układ „Główny”",
    )
    expect(osLabel("other")).toBe("Inny")
  })
})
