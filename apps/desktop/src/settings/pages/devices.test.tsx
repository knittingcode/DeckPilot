// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen } from "@testing-library/react"
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
    deviceModel({ id: "elgato.streamdeck.xl", name: "Stream Deck XL" }),
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

    expect(screen.getByRole("region", { name: "Supported Models" })).toBeInTheDocument()
    expect(await screen.findByText("Stream Deck XL")).toBeInTheDocument()
    expect(screen.getAllByText("Stream Deck MK.2").length).toBeGreaterThan(0)
  })

  it("leads from a model to the layout designer", async () => {
    const { context, user } = renderWithApp(<DevicesSettings />, { agent: agentConnection() })
    const [button] = await screen.findAllByRole("button", { name: "Design a layout" })
    if (!button) throw new Error("no model card")
    await user.click(button)
    expect(context.navigate).toHaveBeenCalledWith("designer")
  })
})
