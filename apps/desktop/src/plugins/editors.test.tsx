// @vitest-environment jsdom
// Plugin actions in the action and workflow editors (experimental,
// VITE_DECKPILOT_PLUGINS=1): the plugin modules come from the agent's
// `listPlugins` (here the in-memory stand-in of test/plugins.ts).
import "@testing-library/jest-dom/vitest"

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { pluginModules } from "@/lib/plugin-actions"
import type { ModuleInfo } from "@/lib/types"
import { ActionsPage } from "@/pages/actions"
import { WorkflowsPage } from "@/pages/workflows"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { ACTION_TYPES, action, builtInModules, profileView } from "@/test/fixtures"
import { HELLO, createFakePluginClient, pluginDefinitions } from "@/test/plugins"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"
import type { BuilderProps } from "@/workflow-builder/builder"
import { moduleGroups } from "@/workflow-builder/catalog"
import { CatalogPanel } from "@/workflow-builder/catalog-panel"

import { PluginClientContext } from "./api"
import { usePluginModules } from "./use-plugin-modules"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
// The plugin client of the app is never loaded in these tests.
const loadedClient = vi.hoisted(() => ({ loaded: false }))
vi.mock("@/plugins/client", () => {
  loadedClient.loaded = true
  return { createTauriPluginClient: () => createFakePluginClient() }
})
// The graph editor has its own tests; here it shows what it was given.
const shown = vi.hoisted(() => ({ props: null as null | Pick<BuilderProps, "types" | "modules"> }))
vi.mock("@/workflow-builder/workflow-run", () => ({
  WorkflowRun: (props: Pick<BuilderProps, "types" | "modules">) => {
    shown.props = props
    return <section aria-label="Graph editor" />
  },
}))

const GREET = `${HELLO}.greet`
const TEMPERATURE = `${HELLO}.temperature`

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  shown.props = null
})
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

/** The plugin modules the app derives from the agent's answer. */
async function helloModules(state: "enabled" | "disabled" | "uninstalled") {
  const client = createFakePluginClient({ helloInstalled: true })
  if (state === "disabled") await client.disablePlugin(HELLO)
  if (state === "uninstalled") await client.uninstallPlugin(HELLO)
  return pluginModules(await client.listPlugins())
}

const greeting = action("a-greet", "Greet Ada", GREET, {
  parameters: { name: { type: "text", value: "Ada" } },
})
const docs = action("a-docs", "Open docs", ACTION_TYPES.openUrl.id)

async function renderLibrary(plugins: ModuleInfo[], actions = [docs, greeting]) {
  api.getProfile.mockResolvedValue(profileView({ actions }))
  const rendered = renderWithApp(<ActionsPage />, {
    modules: builtInModules(),
    ...(plugins.length > 0 ? { pluginModules: plugins } : {}),
  })
  await screen.findByRole("textbox", { name: "Search actions" })
  return rendered
}

describe("the action picker with plugins", () => {
  beforeEach(() => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  })

  it("lists enabled plugin actions under the plugin's name and creates one with its parameters", async () => {
    const created = action("a-new", "Hello", GREET)
    api.createAction.mockResolvedValue(created)
    const { user, titlebarAction } = await renderLibrary(await helloModules("enabled"), [docs])

    titlebarAction("actions.new")
    const chooser = screen.getByRole("group", { name: "Action types" })
    const plugin = within(chooser).getByRole("region", { name: "Hello Weather" })
    expect(within(plugin).getByRole("heading")).toHaveTextContent("Hello Weather· plugin")
    expect(
      within(plugin)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual([
      "GreetSays hello to someone.",
      "Current temperatureShows the temperature of a city.",
    ])
    // Built-in modules are still offered first.
    expect(
      within(chooser)
        .getAllByRole("region")
        .map((r) => r.getAttribute("aria-label")),
    ).toEqual(["General", "System", "Git", "Docker", "HTTP", "Notifications", "Hello Weather"])

    await user.click(within(plugin).getByRole("button", { name: /Greet/ }))
    expect(screen.getByText(/Hello Weather › Greet/)).toBeInTheDocument()
    // The manifest's parameters, as editors.
    const [actionName, parameter] = screen.getAllByLabelText(/^Name/)
    if (!actionName || !parameter) throw new Error("two name fields")
    expect(actionName).toHaveAttribute("id", "action-name")
    expect(parameter).toHaveAttribute("id", "parameter-name")
    expect(screen.getByText("Style")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Create action" }))
    expect(screen.getByText("Name is required.")).toBeInTheDocument()
    expect(api.createAction).not.toHaveBeenCalled()

    await user.type(actionName, "Hello")
    await user.type(parameter, "Ada")
    await user.click(screen.getByRole("button", { name: "Create action" }))
    expect(api.createAction).toHaveBeenCalledWith(
      "profile-1",
      expect.objectContaining({
        name: "Hello",
        actionType: GREET,
        parameters: expect.objectContaining({ name: { type: "text", value: "Ada" } }) as unknown,
      }),
    )
    expect(toast.success).toHaveBeenCalledWith("Action created")
  })

  it("shows unavailable plugin actions with the reason and does not let them be chosen", async () => {
    const { user, titlebarAction } = await renderLibrary(await helloModules("disabled"), [docs])
    titlebarAction("actions.new")
    const plugin = within(screen.getByRole("group", { name: "Action types" })).getByRole("region", {
      name: "Hello Weather",
    })
    const greet = within(plugin).getByRole("button", { name: /Greet/ })
    expect(greet).toBeDisabled()
    expect(greet).toHaveTextContent("Unavailable: the plugin is disabled")
    expect(within(plugin).getByRole("button", { name: /Current temperature/ })).toBeDisabled()
    await user.click(greet)
    expect(screen.getByText("Step 1 of 2: choose what the action does")).toBeVisible()
  })

  it("keeps the actions of a disabled plugin, unavailable with the reason", async () => {
    const { user } = await renderLibrary(await helloModules("disabled"))
    const item = screen.getByRole("button", { name: /Greet Ada/ })
    expect(within(item).getByText("Unavailable")).toHaveAttribute(
      "title",
      "Hello Weather (acme.hello-weather): the plugin is disabled",
    )
    // Its type keeps its name and category.
    expect(within(item).getByText("Greet")).toBeInTheDocument()
    expect(screen.getByRole("region", { name: "Fun" })).toBeInTheDocument()

    await user.click(item)
    expect(
      screen.getByText(
        /Hello Weather \(acme.hello-weather\): the plugin is disabled\. The action is kept/,
      ),
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "Duplicate" })).toBeDisabled()
    // The saved parameters are still shown.
    expect(screen.getByDisplayValue("Ada")).toBeInTheDocument()
  })

  it("keeps the actions of an uninstalled plugin and says so", async () => {
    const { user } = await renderLibrary(await helloModules("uninstalled"))
    const item = screen.getByRole("button", { name: /Greet Ada/ })
    expect(within(item).getByText("Unavailable")).toHaveAttribute(
      "title",
      "The plugin acme.hello-weather is not installed",
    )
    await user.click(item)
    expect(
      screen.getByText(/The plugin acme.hello-weather is not installed\. The action is kept/),
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
    // Deleting stays possible.
    expect(screen.getByRole("button", { name: "Delete" })).toBeEnabled()
  })
})

describe("the action picker without the flag", () => {
  it("is what it was: no plugin group, the old wording for unknown types", async () => {
    const { user, titlebarAction } = await renderLibrary([])
    const item = screen.getByRole("button", { name: /Greet Ada/ })
    expect(within(item).getByText("Unavailable")).toHaveAttribute("title", "Its module is disabled")
    await user.click(item)
    expect(
      screen.getByText(
        "No installed module provides “acme.hello-weather.greet”. The action is kept but cannot run.",
      ),
    ).toBeInTheDocument()
    titlebarAction("actions.new")
    expect(
      within(screen.getByRole("group", { name: "Action types" }))
        .getAllByRole("region")
        .map((r) => r.getAttribute("aria-label")),
    ).toEqual(["General", "System", "Git", "Docker", "HTTP", "Notifications"])
  })
})

function Probe() {
  const modules = usePluginModules("agent")
  return <p aria-label="plugins">{modules.map((m) => `${m.id}:${String(m.enabled)}`).join(",")}</p>
}

describe("the installed plugins as modules", () => {
  it("asks the agent nothing and loads no plugin client without the flag", async () => {
    const client = createFakePluginClient({ helloInstalled: true })
    render(
      <PluginClientContext value={client}>
        <Probe />
      </PluginClientContext>,
    )
    render(<Probe />)
    await act(() => Promise.resolve())
    expect(client.listPlugins).not.toHaveBeenCalled()
    expect(client.subscribe).not.toHaveBeenCalled()
    expect(client.getRuntime).not.toHaveBeenCalled()
    expect(client.listKnownPlugins).not.toHaveBeenCalled()
    expect(client.getSettings).not.toHaveBeenCalled()
    expect(client.listCredentials).not.toHaveBeenCalled()
    expect(loadedClient.loaded).toBe(false)
    for (const probe of screen.getAllByLabelText("plugins")) expect(probe).toHaveTextContent("")
  })

  it("follows the agent's list and its change events with the flag", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const client = createFakePluginClient({ helloInstalled: true })
    render(
      <PluginClientContext value={client}>
        <Probe />
      </PluginClientContext>,
    )
    expect(await screen.findByText(`${HELLO}:true`)).toBeInTheDocument()
    expect(client.listPlugins).toHaveBeenCalledTimes(1)
    expect(client.subscribe).toHaveBeenCalledTimes(1)

    await client.disablePlugin(HELLO)
    act(() => {
      client.emit({ type: "changed", plugin: HELLO, state: "disabled" })
    })
    expect(await screen.findByText(`${HELLO}:false`)).toBeInTheDocument()
    // Progress alone does not ask again.
    act(() => {
      client.emit({ type: "progress", plugin: HELLO, operation: "enable", stage: "started" })
    })
    expect(client.listPlugins).toHaveBeenCalledTimes(2)

    // When the agent cannot answer, there are none.
    client.listPlugins.mockRejectedValueOnce(new Error("agent not running"))
    act(() => {
      client.emit({ type: "changed", plugin: HELLO })
    })
    await waitFor(() => {
      expect(screen.getByLabelText("plugins")).toHaveTextContent("")
    })
  })
})

describe("the workflow node palette with plugins", () => {
  const actions = [greeting, action("a-temp", "Paris", TEMPERATURE)]

  it("lists enabled plugin actions under the plugin's name, placeable and creatable", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const groups = moduleGroups([...builtInModules(), ...(await helloModules("enabled"))], actions)
    const hello = groups.find((g) => g.id === HELLO)
    expect(hello).toMatchObject({ title: "Hello Weather" })
    expect(hello?.unavailableTypes).toBeUndefined()
    const onAdd = vi.fn()
    const onCreateAction = vi.fn()
    render(<CatalogPanel groups={groups} onAdd={onAdd} onCreateAction={onCreateAction} />)
    const entry = screen.getByRole("listitem", { name: /Greet Ada/ })
    expect(entry).toBeEnabled()
    expect(entry).toHaveAttribute("draggable", "true")
    fireEvent.click(entry)
    expect(onAdd).toHaveBeenCalledWith(expect.objectContaining({ key: "action:a-greet" }))
    fireEvent.click(screen.getByRole("button", { name: "Current temperature" }))
    expect(onCreateAction).toHaveBeenCalledWith(pluginDefinitions(HELLO).temperature)
  })

  it("shows the actions of a disabled plugin with the reason, not placeable", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const groups = moduleGroups([...builtInModules(), ...(await helloModules("disabled"))], actions)
    const onAdd = vi.fn()
    const onCreateAction = vi.fn()
    render(<CatalogPanel groups={groups} onAdd={onAdd} onCreateAction={onCreateAction} />)
    const entry = screen.getByRole("listitem", { name: /Greet Ada/ })
    expect(entry).toBeDisabled()
    expect(entry).not.toHaveAttribute("draggable")
    expect(entry).toHaveTextContent("Unavailable: the plugin is disabled")
    fireEvent.click(entry)
    expect(onAdd).not.toHaveBeenCalled()
    const greet = screen.getByRole("button", { name: "Greet" })
    expect(greet).toBeDisabled()
    expect(greet).toHaveAttribute("title", "Unavailable: the plugin is disabled")
    fireEvent.click(greet)
    expect(onCreateAction).not.toHaveBeenCalled()
    // Search keeps them.
    fireEvent.change(screen.getByLabelText("Search nodes"), { target: { value: "weather" } })
    expect(screen.getByRole("button", { name: "Current temperature" })).toBeDisabled()
  })

  it("is unchanged for built-in modules", () => {
    const modules = builtInModules({ git: { enabled: false } })
    const groups = moduleGroups(modules, [docs])
    expect(groups.map((g) => g.id)).toEqual([
      "general",
      "system",
      "docker",
      "http",
      "notifications",
    ])
    for (const group of groups) {
      expect(group).not.toHaveProperty("unavailableTypes")
      for (const entry of group.entries) expect(entry).not.toHaveProperty("unavailable")
    }
  })

  it("gives the graph editor the plugin modules and the plugin types available now", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    api.getProfile.mockResolvedValue(profileView({ actions, workflows: [] }))
    api.listActionTypes.mockResolvedValue([ACTION_TYPES.log])
    const plugins = await helloModules("enabled")
    renderWithApp(<WorkflowsPage />, { modules: builtInModules(), pluginModules: plugins })
    await screen.findByRole("region", { name: "Graph editor" })
    const definitions = pluginDefinitions(HELLO)
    expect(shown.props?.types).toEqual([
      ACTION_TYPES.log,
      definitions.greet,
      definitions.temperature,
    ])
    expect(shown.props?.modules.map((m) => m.id)).toContain(HELLO)
  })

  it("gives the graph editor the backend's types unchanged without the flag", async () => {
    const listed = [ACTION_TYPES.log]
    api.getProfile.mockResolvedValue(profileView({ actions, workflows: [] }))
    api.listActionTypes.mockResolvedValue(listed)
    const modules = builtInModules()
    renderWithApp(<WorkflowsPage />, { modules })
    await screen.findByRole("region", { name: "Graph editor" })
    expect(shown.props?.types).toBe(listed)
    expect(shown.props?.modules).toBe(modules)
  })
})
