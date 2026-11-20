// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, within } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it } from "vitest"

import { i18n } from "@/i18n"
import { CAPABILITIES } from "@/lib/capabilities"
import { PermissionsSettings } from "@/settings/pages/permissions"
import { installDomStubs } from "@/test/dom"
import { builtInModules } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"

beforeAll(installDomStubs)
afterEach(cleanup)

const grants = (title: string) =>
  within(screen.getByRole("list", { name: title }))
    .getAllByRole("listitem")
    .map((item) => item.textContent)

describe("Permissions settings", () => {
  it("lists which modules hold each permission and whether it is granted", () => {
    renderWithApp(<PermissionsSettings />, {
      modules: builtInModules({ docker: { enabled: false } }),
    })
    expect(grants(i18n.t(CAPABILITIES["process-execution"].title))).toEqual([
      "SystemGranted",
      "GitGranted",
      "DockerDisabled",
    ])
    expect(grants(i18n.t(CAPABILITIES.network.title))).toEqual(["HTTPGranted"])
    expect(grants(i18n.t(CAPABILITIES.notifications.title))).toEqual(["NotificationsGranted"])
    expect(grants(i18n.t(CAPABILITIES.credentials.title))).toEqual(["HTTPGranted"])
    // Every capability is described, used or not.
    for (const { description } of Object.values(CAPABILITIES)) {
      expect(screen.getByText(i18n.t(description))).toBeInTheDocument()
    }
  })

  it("says when no module uses a permission", () => {
    renderWithApp(<PermissionsSettings />, {
      modules: builtInModules().filter((m) => m.id === "general"),
    })
    expect(screen.getAllByText("No module uses this permission.")).toHaveLength(
      Object.keys(CAPABILITIES).length - 1,
    )
    expect(grants(i18n.t(CAPABILITIES["profile-read"].title))).toEqual(["GeneralGranted"])
  })

  it("leads to the modules page to withdraw a permission", async () => {
    const { user, context } = renderWithApp(<PermissionsSettings />, { modules: builtInModules() })
    await user.click(screen.getByRole("button", { name: "Modules" }))
    expect(context.openSettings).toHaveBeenCalledWith("modules")
  })
})
