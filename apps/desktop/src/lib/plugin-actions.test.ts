import { afterEach, describe, expect, it, vi } from "vitest"

import { ACTION_TYPES, builtInModules } from "@/test/fixtures"
import { HELLO, LIGHTS, createFakePluginClient, pluginDefinitions } from "@/test/plugins"
import type { PluginSummary } from "@/plugins/types"

import {
  isPluginId,
  isPluginType,
  ownerOf,
  pluginModules,
  pluginsEnabled,
  typeAvailability,
  withPluginModules,
  withPluginTypes,
} from "./plugin-actions"

afterEach(() => {
  vi.unstubAllEnvs()
})

async function installed(state: "enabled" | "disabled"): Promise<PluginSummary[]> {
  const client = createFakePluginClient({ helloInstalled: true })
  if (state === "disabled") await client.disablePlugin(HELLO)
  return client.listPlugins()
}

describe("owner of an action type", () => {
  it("is everything before the last dot, as ModuleId::owner_of decides", () => {
    expect(ownerOf("projects.next")).toBe("projects")
    expect(ownerOf("work-session.toggle")).toBe("work-session")
    expect(ownerOf("acme.hello-weather.greet")).toBe("acme.hello-weather")
    expect(ownerOf("acme.hello-weather.current-temperature")).toBe("acme.hello-weather")
    expect(ownerOf("nodot")).toBeNull()
    expect(ownerOf(".x")).toBeNull()
  })

  it("tells plugin types from built-in ones", () => {
    for (const type of Object.values(ACTION_TYPES)) expect(isPluginType(type.id)).toBe(false)
    expect(isPluginType("acme.hello-weather.greet")).toBe(true)
    expect(isPluginId("acme.hello-weather")).toBe(true)
    expect(isPluginId("projects")).toBe(false)
  })
})

describe("plugin modules", () => {
  it("are one module per installed plugin, with the agent's definitions", async () => {
    const [hello] = pluginModules(await installed("enabled"))
    expect(hello).toMatchObject({
      id: HELLO,
      name: "Hello Weather",
      enabled: true,
      supported: true,
      pages: [],
      config: [],
      capabilities: [],
      plugin: { state: "enabled", unavailable: {} },
    })
    const definitions = pluginDefinitions(HELLO)
    expect(hello?.actionTypes).toEqual([definitions.greet, definitions.temperature])
  })

  it("keep the definitions of a disabled plugin and say why they cannot run", async () => {
    const [hello] = pluginModules(await installed("disabled"))
    expect(hello?.enabled).toBe(false)
    expect(hello?.actionTypes.map((t) => t.id)).toEqual([`${HELLO}.greet`, `${HELLO}.temperature`])
    expect(hello?.plugin?.unavailable).toEqual({
      [`${HELLO}.greet`]: "the plugin is disabled",
      [`${HELLO}.temperature`]: "the plugin is disabled",
    })
  })

  it("ignore definitions outside the plugin's namespace and actions without one", async () => {
    const [summary] = await installed("enabled")
    const [greet, temperature] = summary?.actions ?? []
    if (!summary || !greet || !temperature) throw new Error("installed")
    const definitions = pluginDefinitions(HELLO)
    const forged: PluginSummary = {
      ...summary,
      actions: [
        {
          ...greet,
          actionType: "projects.next",
          definition: { ...definitions.greet, id: "projects.next" },
        },
        {
          ...temperature,
          definition: { ...definitions.temperature, id: `${LIGHTS}.temperature` },
        },
        { action: "bare", actionType: `${HELLO}.bare`, name: "Bare", available: true },
      ],
    }
    const [hello] = pluginModules([forged])
    expect(hello?.actionTypes).toEqual([])
    expect(hello?.enabled).toBe(false)
  })

  it("leave the built-in modules alone when there are none", () => {
    const modules = builtInModules()
    expect(withPluginModules(modules, [])).toBe(modules)
  })
})

describe("availability of an action type", () => {
  it("is unchanged for built-in modules", () => {
    const modules = builtInModules({ git: { enabled: false } })
    expect(typeAvailability(modules, ACTION_TYPES.log.id)).toMatchObject({
      available: true,
      reason: null,
    })
    expect(typeAvailability(modules, ACTION_TYPES.gitStatus.id)).toMatchObject({
      available: false,
      reason: null,
      owner: expect.objectContaining({ id: "git" }) as unknown,
    })
    expect(typeAvailability(modules, "acme.hello-weather.greet")).toEqual({
      available: false,
      owner: undefined,
      reason: null,
    })
  })

  it("follows the agent for plugin types, with the plugin and the reason", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    expect(pluginsEnabled()).toBe(true)
    const enabled = withPluginModules(builtInModules(), pluginModules(await installed("enabled")))
    expect(typeAvailability(enabled, `${HELLO}.greet`)).toMatchObject({
      available: true,
      reason: null,
      owner: expect.objectContaining({ id: HELLO }) as unknown,
    })
    const disabled = withPluginModules(builtInModules(), pluginModules(await installed("disabled")))
    expect(typeAvailability(disabled, `${HELLO}.greet`)).toMatchObject({
      available: false,
      reason: "Hello Weather (acme.hello-weather): the plugin is disabled",
    })
    // An action the installed version no longer has.
    expect(typeAvailability(enabled, `${HELLO}.gone`)).toMatchObject({
      available: false,
      reason: "Hello Weather (acme.hello-weather) no longer provides this action",
    })
    // Uninstalled: the instance stays and says so.
    expect(typeAvailability(builtInModules(), `${HELLO}.greet`)).toEqual({
      available: false,
      owner: undefined,
      reason: "The plugin acme.hello-weather is not installed",
    })
  })

  it("names an uninstalled plugin that plugin_known has, as the agent does", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const client = createFakePluginClient({ helloInstalled: true })
    await client.uninstallPlugin(HELLO)
    const known = await client.listKnownPlugins()
    expect(known).toMatchObject([{ id: HELLO, name: "Hello Weather", installed: false }])
    const modules = withPluginModules(
      builtInModules(),
      pluginModules(await client.listPlugins(), known),
    )
    const gone = modules.find((m) => m.id === HELLO)
    // Nothing to place: no action types, not enabled.
    expect(gone).toMatchObject({
      enabled: false,
      actionTypes: [],
      plugin: { state: "notInstalled" },
    })
    expect(typeAvailability(modules, `${HELLO}.greet`)).toMatchObject({
      available: false,
      reason: "Hello Weather (acme.hello-weather): the plugin is not installed",
    })
    expect(withPluginTypes([ACTION_TYPES.log], modules)).toEqual([ACTION_TYPES.log])
    // An installed plugin is never listed twice.
    const again = createFakePluginClient({ helloInstalled: true })
    expect(
      pluginModules(await again.listPlugins(), await again.listKnownPlugins()).map((m) => m.id),
    ).toEqual([HELLO])
  })
})

describe("workflow action types", () => {
  it("are the backend's list, unchanged, without the flag", async () => {
    const types = [ACTION_TYPES.log]
    const modules = withPluginModules(builtInModules(), pluginModules(await installed("enabled")))
    expect(withPluginTypes(types, modules)).toBe(types)
  })

  it("are the backend's list with the plugin types available now", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const definitions = pluginDefinitions(HELLO)
    const types = [ACTION_TYPES.log]
    expect(withPluginTypes(types, builtInModules())).toBe(types)
    const enabled = withPluginModules(builtInModules(), pluginModules(await installed("enabled")))
    expect(withPluginTypes(types, enabled)).toEqual([
      ACTION_TYPES.log,
      definitions.greet,
      definitions.temperature,
    ])
    // A stale plugin type from the backend goes when the plugin is disabled.
    const disabled = withPluginModules(builtInModules(), pluginModules(await installed("disabled")))
    expect(withPluginTypes([ACTION_TYPES.log, definitions.greet], disabled)).toEqual([
      ACTION_TYPES.log,
    ])
  })
})
