// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { ThemeProvider } from "@/components/theme-provider"
import { StatusContext, createStatusStores } from "@/hooks/use-status"
import { installDomStubs } from "@/test/dom"
import { deviceInfo, deviceModel, updateState } from "@/test/fixtures"
import { PluginClientContext } from "@/plugins/api"
import { ICON, createFakePluginClient, type FakePluginClient } from "@/test/plugins"
import { renderWithApp } from "@/test/render"

import { currentOption, deviceOptions } from "./device-options"
import {
  NAVIGATION_ORDER_KEY,
  type PluginNavigationItem,
  clearNavigationIcons,
} from "./plugin-navigation"
import { ToolbarEnd } from "./toolbar"

vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

beforeAll(installDomStubs)
beforeEach(() => {
  localStorage.clear()
  clearNavigationIcons()
})
afterEach(cleanup)

const item = (
  id: string,
  label: string,
  plugin = "acme.weather",
  kind: PluginNavigationItem["view"]["kind"] = "variables",
): PluginNavigationItem => ({
  id,
  label,
  view: { id: `${id}-view`, kind },
  plugin,
  pluginName: "Weather",
})

function renderEnd(
  plugins: PluginNavigationItem[],
  options: { marketplace?: boolean; available?: boolean; client?: FakePluginClient } = {},
) {
  const stores = createStatusStores()
  const onMarketplace = vi.fn()
  const onPlugin = vi.fn()
  const client = options.client ?? createFakePluginClient()
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <ThemeProvider>
        <StatusContext value={stores}>
          <ToolbarEnd
            plugins={plugins}
            onPlugin={onPlugin}
            marketplace={options.marketplace ?? true}
            onMarketplace={onMarketplace}
          />
        </StatusContext>
      </ThemeProvider>
    </PluginClientContext>,
    options.available ? { updates: updateState("github") } : {},
  )
  return {
    ...result,
    client,
    onMarketplace,
    onPlugin,
    group: screen.getByRole("group", { name: "App" }),
  }
}

const labels = (container: HTMLElement) =>
  within(container)
    .getAllByRole("button")
    .map((b) => b.getAttribute("aria-label"))

describe("toolbar end", () => {
  it("shows plugin entries first and a separator only when there are any", async () => {
    const weather = item("weather", "Weather")
    const { group, user, onPlugin } = renderEnd([weather, item("radar", "Radar")])
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
    expect(onPlugin).toHaveBeenCalledWith(weather)
    // The label is the tooltip.
    expect(within(nav).getByRole("button", { name: "Radar" })).toHaveAttribute("title", "Radar")
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

describe("plugin entries", () => {
  it("show the agent's icon for the dark appearance as an image, else the app's own icon", async () => {
    const client = createFakePluginClient({
      navigationIcons: { "acme.weather/weather/dark": ICON },
    })
    renderEnd([item("weather", "Weather"), item("radar", "Radar")], { client })
    const nav = screen.getByRole("navigation", { name: "Plugins" })
    await waitFor(() => {
      expect(
        within(nav).getByRole("button", { name: "Weather" }).querySelector("img"),
      ).toHaveAttribute("src", ICON)
    })
    expect(client.getNavigationIcon).toHaveBeenCalledWith("acme.weather", "weather", "dark")
    // No icon for the radar: the fallback, never an empty image.
    const radar = within(nav).getByRole("button", { name: "Radar" })
    await waitFor(() => {
      expect(client.getNavigationIcon).toHaveBeenCalledWith("acme.weather", "radar", "dark")
    })
    expect(radar.querySelector("img")).toBeNull()
    expect(radar.querySelector("[data-slot='plugin-icon-fallback']")).not.toBeNull()
  })

  it("fall back when the icon fails or is not an image the agent prepared", async () => {
    const client = createFakePluginClient()
    client.getNavigationIcon.mockImplementation((_plugin, entry) =>
      entry === "broken"
        ? Promise.reject(new Error("agent gone"))
        : Promise.resolve("https://example.com/x.png"),
    )
    renderEnd([item("broken", "Broken"), item("remote", "Remote")], { client })
    await waitFor(() => {
      expect(client.getNavigationIcon).toHaveBeenCalledTimes(2)
    })
    for (const name of ["Broken", "Remote"]) {
      const button = screen.getByRole("button", { name })
      await waitFor(() => {
        expect(button.querySelector("[data-slot='plugin-icon-fallback']")).not.toBeNull()
      })
      expect(button.querySelector("img")).toBeNull()
    }
  })

  it("put entries that do not fit into a “More” menu", async () => {
    const items = ["one", "two", "three", "four", "five"].map((id) => item(id, id.toUpperCase()))
    const { user, onPlugin } = renderEnd(items)
    const nav = screen.getByRole("navigation", { name: "Plugins" })
    expect(labels(nav)).toEqual(["ONE", "TWO", "THREE", "More plugin views"])
    await user.click(within(nav).getByRole("button", { name: "More plugin views" }))
    const menu = screen.getByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: "FOUR" })).toBeInTheDocument()
    await user.click(within(menu).getByRole("menuitem", { name: "FIVE" }))
    expect(onPlugin).toHaveBeenCalledWith(items[4])
  })

  it("keep four entries in the toolbar, and none in a narrow window", () => {
    const items = ["one", "two", "three", "four"].map((id) => item(id, id.toUpperCase()))
    renderEnd(items)
    expect(labels(screen.getByRole("navigation", { name: "Plugins" }))).toEqual([
      "ONE",
      "TWO",
      "THREE",
      "FOUR",
    ])
    cleanup()
    const narrow = vi.spyOn(window, "matchMedia").mockImplementation(
      (query: string) =>
        ({
          matches: query.includes("max-width"),
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
        }) as unknown as MediaQueryList,
    )
    try {
      renderEnd(items)
      expect(labels(screen.getByRole("navigation", { name: "Plugins" }))).toEqual([
        "More plugin views",
      ])
    } finally {
      // Back to the wide window of the other tests.
      narrow.mockImplementation(
        () =>
          ({
            matches: false,
            addEventListener: vi.fn(),
            removeEventListener: vi.fn(),
          }) as unknown as MediaQueryList,
      )
    }
  })

  it("keep the order the user gives them", async () => {
    const items = [item("one", "One"), item("two", "Two"), item("three", "Three", "knit.lights")]
    const { user } = renderEnd(items)
    const nav = () => screen.getByRole("navigation", { name: "Plugins" })
    fireEvent.contextMenu(within(nav()).getByRole("button", { name: "One" }))
    await user.click(await screen.findByRole("menuitem", { name: "Move right" }))
    expect(labels(nav())).toEqual(["Two", "One", "Three"])
    expect(JSON.parse(localStorage.getItem(NAVIGATION_ORDER_KEY) ?? "[]")).toEqual([
      "acme.weather/two",
      "acme.weather/one",
      "knit.lights/three",
    ])
    cleanup()

    // Next time, with a new plugin's entry at the end.
    renderEnd([...items, item("four", "Four", "knit.new")])
    expect(labels(nav())).toEqual(["Two", "One", "Three", "Four"])
  })

  it("disappear with their plugin", () => {
    const { rerender } = renderEnd([item("weather", "Weather")])
    expect(screen.getByRole("navigation", { name: "Plugins" })).toBeInTheDocument()
    rerender(
      <PluginClientContext value={createFakePluginClient()}>
        <ThemeProvider>
          <StatusContext value={createStatusStores()}>
            <ToolbarEnd plugins={[]} marketplace onMarketplace={vi.fn()} />
          </StatusContext>
        </ThemeProvider>
      </PluginClientContext>,
    )
    expect(screen.queryByRole("navigation", { name: "Plugins" })).not.toBeInTheDocument()
    expect(document.querySelector("[data-slot='plugin-navigation-separator']")).toBeNull()
  })
})
