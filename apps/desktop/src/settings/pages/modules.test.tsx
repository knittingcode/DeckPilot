// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { ModulesSettings } from "@/settings/pages/modules"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { builtInModules } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  localStorage.clear()
})
afterEach(cleanup)

describe("Modules settings with the built-in modules", () => {
  it("disables a module and reports the new state", async () => {
    const next = builtInModules({ docker: { enabled: false } })
    let resolve: (value: typeof next) => void = () => undefined
    api.setModuleEnabled.mockReturnValue(
      new Promise((r) => {
        resolve = r
      }),
    )
    const { user, context } = renderWithApp(<ModulesSettings />, { modules: builtInModules() })
    await user.click(screen.getByRole("switch", { name: "Disable Docker" }))
    expect(api.setModuleEnabled).toHaveBeenCalledWith("docker", false)
    // Other switches wait until the change is applied.
    expect(screen.getByRole("switch", { name: "Disable HTTP" })).toBeDisabled()
    resolve(next)
    await waitFor(() => {
      expect(context.setModules).toHaveBeenCalledWith(next)
    })
    expect(context.refresh).toHaveBeenCalled()
    expect(toast.success).toHaveBeenCalledWith("Docker disabled")
    expect(screen.getByRole("switch", { name: "Disable HTTP" })).toBeEnabled()
  })

  it("explains dependencies in both directions", () => {
    renderWithApp(<ModulesSettings />, { modules: builtInModules() })
    const system = screen.getByRole("switch", { name: "Disable System" })
    expect(system).toBeDisabled()
    expect(system).toHaveAccessibleDescription("Required by Git. Disable it first.")
    cleanup()

    renderWithApp(<ModulesSettings />, {
      modules: builtInModules({ system: { enabled: false }, git: { enabled: false } }),
    })
    const git = screen.getByRole("switch", { name: "Enable Git" })
    expect(git).toBeDisabled()
    expect(git).toHaveAccessibleDescription("Requires System. Enable it first.")
    expect(screen.getByRole("switch", { name: "Enable System" })).toBeEnabled()
  })

  it("keeps the state and reports errors when switching fails", async () => {
    api.setModuleEnabled.mockRejectedValue(new Error("Module is busy"))
    const { user, context } = renderWithApp(<ModulesSettings />, { modules: builtInModules() })
    await user.click(screen.getByRole("switch", { name: "Disable HTTP" }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("Module is busy")
    })
    expect(context.setModules).not.toHaveBeenCalled()
    expect(screen.getByRole("switch", { name: "Disable HTTP" })).toBeChecked()
  })

  it("filters unavailable modules and says why they cannot be enabled", async () => {
    const { user } = renderWithApp(<ModulesSettings />, {
      modules: builtInModules({ docker: { enabled: false, supported: false } }),
    })
    await user.click(screen.getByRole("radio", { name: /Unavailable/ }))
    const list = within(screen.getByRole("list", { name: "Modules" }))
    expect(list.getAllByRole("listitem")).toHaveLength(1)
    expect(list.getByRole("switch", { name: "Enable Docker" })).toHaveAccessibleDescription(
      "Not available on this operating system.",
    )
    await user.type(screen.getByRole("searchbox", { name: "Search modules" }), "http")
    expect(screen.getByText("No modules found")).toBeInTheDocument()
  })
})
