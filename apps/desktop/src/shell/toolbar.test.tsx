// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, within } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { ThemeProvider } from "@/components/theme-provider"
import { StatusContext, createStatusStores } from "@/hooks/use-status"
import { installDomStubs } from "@/test/dom"
import { deviceInfo, deviceModel, updateState } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"

import { currentOption, deviceOptions } from "./device-options"
import type { PluginNavigationItem } from "./plugin-navigation"
import { ToolbarEnd } from "./toolbar"

vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

beforeAll(installDomStubs)
afterEach(cleanup)

const item = (id: string, label: string): PluginNavigationItem => ({
  id: `acme.weather/${id}`,
  plugin: "acme.weather",
  label,
  onSelect: vi.fn(),
})

function renderEnd(
  plugins: PluginNavigationItem[],
  options: { marketplace?: boolean; available?: boolean } = {},
) {
  const stores = createStatusStores()
  const onMarketplace = vi.fn()
  const result = renderWithApp(
    <ThemeProvider>
      <StatusContext value={stores}>
        <ToolbarEnd
          plugins={plugins}
          marketplace={options.marketplace ?? true}
          onMarketplace={onMarketplace}
        />
      </StatusContext>
    </ThemeProvider>,
    options.available ? { updates: updateState("github") } : {},
  )
  return { ...result, onMarketplace, group: screen.getByRole("group", { name: "App" }) }
}

describe("toolbar end", () => {
  it("shows plugin entries first and a separator only when there are any", async () => {
    const weather = item("weather", "Weather")
    const { group, user } = renderEnd([weather, item("radar", "Radar")])
    const nav = within(group).getByRole("navigation", { name: "Plugins" })
    expect(
      within(group)
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label")),
    ).toEqual(["Weather", "Radar", "Notifications", "Marketplace", "Settings"])
    const separator = group.querySelector("[data-slot='plugin-navigation-separator']")
    expect(separator).not.toBeNull()
    // In order: plugins, separator, the app's commands.
    expect(nav.nextElementSibling).toBe(separator)
    await user.click(within(nav).getByRole("button", { name: "Weather" }))
    expect(weather.onSelect).toHaveBeenCalled()
  })

  it("has no plugin navigation and no separator without entries", () => {
    const { group } = renderEnd([])
    expect(within(group).queryByRole("navigation")).not.toBeInTheDocument()
    expect(group.querySelector("[data-slot='plugin-navigation-separator']")).toBeNull()
    expect(group.firstElementChild).toBe(screen.getByRole("button", { name: "Notifications" }))
  })

  it("leaves the marketplace out of builds without plugins", () => {
    const { group } = renderEnd([], { marketplace: false })
    expect(
      within(group)
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label")),
    ).toEqual(["Notifications", "Settings"])
  })

  it("opens the marketplace and the settings (at the update when there is one)", async () => {
    const { user, onMarketplace, context } = renderEnd([], { available: true })
    await user.click(screen.getByRole("button", { name: "Marketplace" }))
    expect(onMarketplace).toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Settings, an update is available" }))
    expect(context.openSettings).toHaveBeenCalledWith("updates")
  })
})

describe("device options", () => {
  const mk2 = deviceModel()
  const plus = deviceModel({ id: "elgato.streamdeck.plus", name: "Stream Deck +" })
  const mini = deviceModel({ id: "elgato.streamdeck.mini", name: "Stream Deck Mini" })

  it("lists connected devices first, then the models without one", () => {
    const options = deviceOptions(
      [mk2, plus, mini],
      [
        deviceInfo({ id: { model: plus.id, serial: "A1" }, modelName: "Stream Deck +" }),
        deviceInfo({ id: { model: "vendor.unknown", serial: "X9" }, modelName: "Unknown deck" }),
        deviceInfo({ id: { model: mini.id, serial: "OFF" }, state: "disconnected" }),
      ],
    )
    expect(options.map((o) => [o.label, o.detail, o.available])).toEqual([
      ["Stream Deck +", "Connected · A1", true],
      ["Unknown deck", "Connected · X9 · not in the device catalog", false],
      ["Stream Deck MK.2", "Not connected", true],
      ["Stream Deck Mini", "Not connected", true],
    ])
  })

  it("names devices as their descriptor does", () => {
    const [option] = deviceOptions(
      [plus],
      [
        deviceInfo({
          id: { model: "legacy.id", serial: "D1" },
          modelName: "Old name",
          descriptor: {
            vendorId: "elgato",
            vendorName: "Elgato",
            modelId: plus.id,
            displayName: "Stream Deck + (desk)",
            serial: "D1",
            capabilities: plus.capabilities,
            driver: null,
            verification: [],
          },
        }),
      ],
    )
    expect(option).toMatchObject({ label: "Stream Deck + (desk)", model: plus.id, available: true })
  })

  it("finds what the editor shows", () => {
    const options = deviceOptions(
      [mk2, plus],
      [
        deviceInfo({ id: { model: plus.id, serial: "A1" } }),
        deviceInfo({ id: { model: plus.id, serial: "A2" } }),
      ],
    )
    expect(currentOption(options, null, null)).toBeUndefined()
    expect(currentOption(options, plus.id, null)?.key).toBe(`device:${plus.id}:A1`)
    expect(currentOption(options, plus.id, { model: plus.id, serial: "A2" })?.key).toBe(
      `device:${plus.id}:A2`,
    )
    expect(currentOption(options, mk2.id, null)?.key).toBe(`model:${mk2.id}`)
  })
})
