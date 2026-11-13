// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { agentConnection, agentStatus, deviceInfo, deviceModel } from "@/test/fixtures"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"
import { DevicesSettings } from "@/settings/pages/devices"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))

beforeAll(() => {
  installDomStubs()
})

beforeEach(() => {
  resetApi()
  api.getGestureDefaults.mockReturnValue(new Promise(() => undefined))
  api.listDeviceModels.mockResolvedValue([
    deviceModel(),
    deviceModel({
      id: "elgato.streamdeck.xl",
      name: "Stream Deck XL",
      support: "descriptorOnly",
      verification: ["macos", "windows", "linux"].map((platform) => ({
        platform: platform as "macos",
        level: "defined" as const,
        evidenceDate: "2026-11-13",
        evidence: "Descriptor only.",
      })),
    }),
  ])
})

afterEach(() => {
  cleanup()
})

describe("Settings › Devices", () => {
  it("shows the connected devices and the supported models on one page", async () => {
    const agent = agentConnection({ status: agentStatus({ devices: [deviceInfo()] }) })
    renderWithApp(<DevicesSettings />, { agent })

    expect(screen.getByText("Connected devices")).toBeInTheDocument()
    expect(screen.getByText("SIM-MK2-01", { exact: false })).toBeInTheDocument()

    const supported = screen.getByRole("region", { name: "Supported Models" })
    expect(await within(supported).findByText("Stream Deck MK.2")).toBeInTheDocument()
    expect(within(supported).getByText("macOS: Physically verified")).toBeInTheDocument()
    expect(within(supported).getByText("Windows: Automated tests only")).toBeInTheDocument()
    expect(within(supported).getByText("Linux: Automated tests only")).toBeInTheDocument()
  })

  it("lists only verified models; experimental ones are hidden behind a toggle", async () => {
    const { user } = renderWithApp(<DevicesSettings />, { agent: agentConnection() })
    const supported = screen.getByRole("region", { name: "Supported Models" })
    await within(supported).findByText("Stream Deck MK.2")
    expect(screen.queryByText("Stream Deck XL")).not.toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Experimental, not verified" })).toBeNull()

    await user.click(screen.getByRole("switch", { name: "Show experimental models" }))
    const experimental = screen.getByRole("region", { name: "Experimental, not verified" })
    expect(within(experimental).getByText("Stream Deck XL")).toBeInTheDocument()
    expect(within(experimental).getByText("macOS: Layout design only")).toBeInTheDocument()
    expect(within(supported).queryByText("Stream Deck XL")).toBeNull()
  })

  it("leads from a model to the layout designer", async () => {
    const { context, user } = renderWithApp(<DevicesSettings />, { agent: agentConnection() })
    const [button] = await screen.findAllByRole("button", { name: "Design a layout" })
    if (!button) throw new Error("no model card")
    await user.click(button)
    expect(context.navigate).toHaveBeenCalledWith("designer")
  })
})
