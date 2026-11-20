// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { moduleInfo } from "@/lib/test-fixtures"
import type { ResourceCheck } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"
import { resetToasts } from "@/test/toast"

import { ResourcesSettings } from "./resources"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const checks: ResourceCheck[] = [
  {
    key: "shop.repository",
    kind: "directory",
    value: "/src/shop",
    status: "ok",
    usedBy: ["SHOP repository"],
  },
  // Only a project asks for it; nothing is bound.
  { key: "shop.docs", kind: null, value: null, status: "unbound", usedBy: ["SHOP docs"] },
]

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  api.checkResources.mockResolvedValue(checks)
})
afterEach(cleanup)

describe("Settings › Resources", () => {
  it("edits this computer's bindings without the Projects plugin", async () => {
    // No Projects module, no plugin: resources are the core's.
    const { user } = renderWithApp(<ResourcesSettings />, { modules: [] })
    expect(await screen.findByText("shop.repository")).toBeInTheDocument()
    expect(screen.getByText("Directory: /src/shop")).toBeInTheDocument()
    // Project roles and the keys only projects ask for are not shown.
    expect(screen.queryByText("shop.docs")).not.toBeInTheDocument()
    expect(screen.queryByText(/Used by/)).not.toBeInTheDocument()
    expect(api.checkResources).toHaveBeenCalledWith("profile-1")

    await user.click(screen.getByRole("button", { name: "Add mapping" }))
    const dialog = await screen.findByRole("dialog", { name: "New mapping" })
    await user.type(within(dialog).getByLabelText("Resource key"), "tools.editor")
    await user.type(within(dialog).getByLabelText("Value"), "/Applications/Editor.app")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))
    expect(api.setResourceBinding).toHaveBeenCalledWith(
      "tools.editor",
      "directory",
      "/Applications/Editor.app",
    )

    await user.click(screen.getByRole("button", { name: "Remove shop.repository" }))
    const confirm = await screen.findByRole("alertdialog")
    await user.click(within(confirm).getByRole("button", { name: "Remove" }))
    expect(api.removeResourceBinding).toHaveBeenCalledWith("shop.repository")
  })

  it("shows the project roles using each resource while Projects is active", async () => {
    renderWithApp(<ResourcesSettings />, { modules: [moduleInfo({ id: "projects" })] })
    expect(await screen.findByText("shop.docs")).toBeInTheDocument()
    expect(screen.getByText("Used by SHOP repository")).toBeInTheDocument()
    expect(screen.getByText("Used by SHOP docs")).toBeInTheDocument()
  })

  it("speaks Polish", async () => {
    await i18n.changeLanguage("pl")
    try {
      renderWithApp(<ResourcesSettings />, { modules: [] })
      expect(await screen.findByText("Zasoby na tym komputerze")).toBeInTheDocument()
      expect(screen.getByRole("button", { name: "Usuń shop.repository" })).toBeInTheDocument()
    } finally {
      await i18n.changeLanguage("en")
    }
  })
})
