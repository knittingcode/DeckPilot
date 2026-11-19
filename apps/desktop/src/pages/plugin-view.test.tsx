// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// The views of plugin toolbar entries: drawn by the app from the entry's
// kind, never by the plugin.

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { PluginClientContext } from "@/plugins/api"
import { PluginNavigationContext, type PluginNavigationItem } from "@/shell/plugin-navigation"
import { installDomStubs } from "@/test/dom"
import { HELLO, createFakePluginClient } from "@/test/plugins"
import { renderWithApp } from "@/test/render"

import { PluginViewPage } from "./plugin-view"

vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)
// The views reuse the app's own pages; what they are given is checked here.
vi.mock("@/pages/variables-table", () => ({
  VariablesTable: ({ namespace }: { namespace?: string }) => (
    <p data-testid="variables-table">variables under {namespace}</p>
  ),
}))
vi.mock("@/pages/actions", () => ({
  ActionLibraryView: ({ plugin }: { plugin?: { id: string; name: string } }) => (
    <p data-testid="action-library">
      actions of {plugin?.id} ({plugin?.name})
    </p>
  ),
}))

beforeAll(installDomStubs)
afterEach(cleanup)

const entry = (id: string, kind: string, label = "Weather"): PluginNavigationItem => ({
  id,
  label,
  view: { id: `${id}-view`, kind },
  plugin: HELLO,
  pluginName: "Hello Weather",
})

function renderView(items: PluginNavigationItem[], page: string) {
  const client = createFakePluginClient({ helloInstalled: true })
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginNavigationContext value={{ items, page }}>
        <PluginViewPage />
      </PluginNavigationContext>
    </PluginClientContext>,
  )
  return { ...result, client }
}

describe("plugin views", () => {
  it("show the plugin's variables, live, from the app's variables table", () => {
    const { titlebar } = renderView([entry("vars", "variables")], `plugin-view/${HELLO}/vars`)
    expect(screen.getByTestId("variables-table")).toHaveTextContent(
      "variables under plugins.acme.helloWeather",
    )
    expect(document.querySelector("[data-plugin-view='variables']")).not.toBeNull()
    expect(titlebar.getSnapshot().title).toBe("Weather")
  })

  it("show the Action Library with the plugin's actions only", () => {
    renderView([entry("acts", "actions", "Greetings")], `plugin-view/${HELLO}/acts`)
    expect(screen.getByTestId("action-library")).toHaveTextContent(
      `actions of ${HELLO} (Hello Weather)`,
    )
  })

  it("show the plugin's settings page", async () => {
    const { client } = renderView([entry("conf", "settings")], `plugin-view/${HELLO}/conf`)
    expect(screen.getByText("Settings of Hello Weather")).toBeInTheDocument()
    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument()
    expect(client.getSettings).toHaveBeenCalledWith(HELLO)
    expect(client.listCredentials).toHaveBeenCalledWith(HELLO)
  })

  it("say so when the entry is gone or of a kind the app does not draw", async () => {
    const { user, context } = renderView([], `plugin-view/${HELLO}/vars`)
    expect(screen.getByText("This plugin view is not available")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open the Device Editor" }))
    expect(context.navigate).toHaveBeenCalledWith("designer")
    cleanup()

    renderView([entry("dash", "dashboard")], `plugin-view/${HELLO}/dash`)
    expect(screen.getByText("This plugin view is not available")).toBeInTheDocument()
    expect(screen.queryByTestId("variables-table")).not.toBeInTheDocument()
  })
})
