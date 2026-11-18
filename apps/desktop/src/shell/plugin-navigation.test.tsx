// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { PluginClientContext } from "@/plugins/api"
import { HELLO, LIGHTS, createFakePluginClient, type FakePluginClient } from "@/test/plugins"

import {
  NAVIGATION_ORDER_KEY,
  loadNavigationOrder,
  moveInOrder,
  orderedItems,
  parsePluginViewPage,
  pluginVariableNamespace,
  pluginViewPage,
  saveNavigationOrder,
  splitEntries,
  usePluginNavigation,
} from "./plugin-navigation"

function fakeWithEntries(): FakePluginClient {
  return createFakePluginClient({
    helloInstalled: true,
    navigation: {
      [HELLO]: [
        { id: "vars", label: "Weather", view: { id: "v", kind: "variables" } },
        { id: "acts", label: "Greetings", view: { id: "a", kind: "actions" } },
        { id: "conf", label: "Weather setup", view: { id: "s", kind: "settings" } },
        // A kind a later agent may add: not drawn here, so not shown.
        { id: "dash", label: "Dashboard", view: { id: "d", kind: "dashboard" } },
      ],
    },
  })
}

function renderNavigation(client: FakePluginClient, reloadKey: unknown = 1) {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <PluginClientContext value={client}>{children}</PluginClientContext>
  )
  return renderHook(({ key }: { key: unknown }) => usePluginNavigation(key), {
    wrapper,
    initialProps: { key: reloadKey },
  })
}

afterEach(() => {
  vi.unstubAllEnvs()
})

describe("the plugin entries of the toolbar", () => {
  beforeEach(() => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  })

  it("lists the entries of enabled plugins whose view the app draws", async () => {
    const client = fakeWithEntries()
    const { result } = renderNavigation(client)
    await waitFor(() => {
      expect(result.current.map((i) => i.label)).toEqual(["Weather", "Greetings", "Weather setup"])
    })
    expect(result.current[0]).toMatchObject({ plugin: HELLO, pluginName: "Hello Weather" })
  })

  it("drops them when the plugin is disabled or uninstalled, and brings them back", async () => {
    const client = fakeWithEntries()
    const { result } = renderNavigation(client)
    await waitFor(() => {
      expect(result.current).toHaveLength(3)
    })
    await act(async () => {
      await client.disablePlugin(HELLO)
      client.emit({ type: "changed", plugin: HELLO, state: "disabled" })
    })
    await waitFor(() => {
      expect(result.current).toEqual([])
    })
    await act(async () => {
      await client.enablePlugin(HELLO, [])
      client.emit({ type: "changed", plugin: HELLO, state: "enabled" })
    })
    await waitFor(() => {
      expect(result.current).toHaveLength(3)
    })
    await act(async () => {
      await client.uninstallPlugin(HELLO)
      client.emit({ type: "changed", plugin: HELLO })
    })
    await waitFor(() => {
      expect(result.current).toEqual([])
    })
  })

  it("reads them again when the plugins are read again, and has none when the agent cannot answer", async () => {
    const client = fakeWithEntries()
    const { result, rerender } = renderNavigation(client)
    await waitFor(() => {
      expect(result.current).toHaveLength(3)
    })
    client.listNavigation.mockRejectedValueOnce(new Error("agent stopped"))
    rerender({ key: 2 })
    await waitFor(() => {
      expect(result.current).toEqual([])
    })
    expect(client.listNavigation).toHaveBeenCalledTimes(2)
  })

  it("asks nothing in builds without plugins", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "")
    const client = fakeWithEntries()
    const { result } = renderNavigation(client)
    await new Promise((resolve) => setTimeout(resolve, 10))
    expect(result.current).toEqual([])
    expect(client.listNavigation).not.toHaveBeenCalled()
  })
})

describe("plugin entry helpers", () => {
  const entry = (plugin: string, id: string) => ({ plugin, id })

  it("names the page of an entry's view", () => {
    const page = pluginViewPage(entry(HELLO, "vars"))
    expect(page).toBe("plugin-view/acme.hello-weather/vars")
    expect(parsePluginViewPage(page)).toEqual({ plugin: HELLO, entry: "vars" })
    expect(parsePluginViewPage("designer")).toBeNull()
    expect(parsePluginViewPage("projects/projects")).toBeNull()
    expect(parsePluginViewPage("plugin-view/acme.x")).toBeNull()
    expect(parsePluginViewPage("plugin-view/acme.x/a/b")).toBeNull()
  })

  it("finds a plugin's variables", () => {
    expect(pluginVariableNamespace(HELLO)).toBe("plugins.acme.helloWeather")
    expect(pluginVariableNamespace(LIGHTS)).toBe("plugins.knit.homeLights")
    expect(pluginVariableNamespace("my-org.x")).toBe("plugins.myOrg.x")
  })

  it("orders entries as the user did, new ones last", () => {
    const items = [entry("a.p", "1"), entry("a.p", "2"), entry("b.p", "3")]
    expect(orderedItems(items, ["b.p/3", "gone/9", "a.p/1"])).toEqual([
      entry("b.p", "3"),
      entry("a.p", "1"),
      entry("a.p", "2"),
    ])
    expect(moveInOrder(items, "b.p/3", 0)).toEqual(["b.p/3", "a.p/1", "a.p/2"])
    expect(moveInOrder(items, "a.p/1", 5)).toEqual(["a.p/1", "a.p/2", "b.p/3"])
  })

  it("keeps the order per computer, and survives storage that fails", () => {
    const memory = new Map<string, string>()
    const store = {
      getItem: (key: string) => memory.get(key) ?? null,
      setItem: (key: string, value: string) => {
        memory.set(key, value)
      },
    }
    saveNavigationOrder(["a.p/1"], store)
    expect(memory.get(NAVIGATION_ORDER_KEY)).toBe('["a.p/1"]')
    expect(loadNavigationOrder(store)).toEqual(["a.p/1"])
    memory.set(NAVIGATION_ORDER_KEY, "{not json")
    expect(loadNavigationOrder(store)).toEqual([])
    memory.set(NAVIGATION_ORDER_KEY, '[1, "a.p/2"]')
    expect(loadNavigationOrder(store)).toEqual(["a.p/2"])
    const broken = {
      getItem: () => {
        throw new Error("denied")
      },
      setItem: () => {
        throw new Error("denied")
      },
    }
    expect(loadNavigationOrder(broken)).toEqual([])
    expect(() => {
      saveNavigationOrder(["x"], broken)
    }).not.toThrow()
  })

  it("splits entries between the toolbar and its menu", () => {
    expect(splitEntries([1, 2, 3, 4], false)).toEqual({ inline: [1, 2, 3, 4], overflow: [] })
    expect(splitEntries([1, 2, 3, 4, 5], false)).toEqual({ inline: [1, 2, 3], overflow: [4, 5] })
    expect(splitEntries([1, 2], true)).toEqual({ inline: [], overflow: [1, 2] })
  })
})
