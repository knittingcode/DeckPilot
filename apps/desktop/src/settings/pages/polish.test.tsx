// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { ModulesSettings } from "@/settings/pages/modules"
import { PermissionsSettings } from "@/settings/pages/permissions"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { builtInModules } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(async () => {
  resetApi()
  resetToasts()
  await act(async () => {
    await i18n.changeLanguage("pl")
  })
})
afterEach(async () => {
  cleanup()
  await act(async () => {
    await i18n.changeLanguage("en")
  })
})

describe("Settings pages in Polish", () => {
  it("shows the Modules page in Polish, with Polish plurals", async () => {
    const { user } = renderWithApp(<ModulesSettings />, {
      modules: builtInModules({ docker: { enabled: false } }),
    })
    expect(screen.getByRole("searchbox", { name: "Szukaj modułów" })).toBeInTheDocument()
    expect(screen.getByLabelText("Pokaż moduły")).toBeInTheDocument()
    expect(screen.getByRole("radio", { name: /Wszystkie/ })).toBeInTheDocument()
    expect(screen.getByRole("switch", { name: "Wyłącz HTTP" })).toBeInTheDocument()
    expect(screen.getByRole("switch", { name: "Włącz Docker" })).toBeInTheDocument()
    expect(screen.getAllByText(/\d+ (typ|typy|typów) akcji/).length).toBeGreaterThan(0)
    expect(screen.queryByText(/action types?/)).not.toBeInTheDocument()

    await user.type(screen.getByRole("searchbox", { name: "Szukaj modułów" }), "nic takiego")
    expect(screen.getByText("Nie znaleziono modułów")).toBeInTheDocument()
    expect(screen.getByText("Żaden moduł nie pasuje do wyszukiwania.")).toBeInTheDocument()
  })

  it("shows a refusal of the backend in Polish, by its code", async () => {
    api.setModuleEnabled.mockRejectedValue({
      code: "locked",
      message: "the configuration is locked",
    })
    const { user, context } = renderWithApp(<ModulesSettings />, { modules: builtInModules() })
    await user.click(screen.getByRole("switch", { name: "Wyłącz HTTP" }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("Nie wyłączono modułu HTTP", {
        description: "Konfiguracja jest zablokowana.",
      })
    })
    expect(context.setModules).not.toHaveBeenCalled()
  })

  it("keeps the link inside a translated sentence", async () => {
    const { user, context } = renderWithApp(<PermissionsSettings />, {
      modules: builtInModules(),
    })
    expect(screen.getByText(/Każdy moduł deklaruje, do czego może mieć dostęp/)).toBeInTheDocument()
    expect(screen.getAllByText("Przyznane").length).toBeGreaterThan(0)
    await user.click(screen.getByRole("button", { name: "Moduły" }))
    expect(context.openSettings).toHaveBeenCalledWith("modules")
  })
})
