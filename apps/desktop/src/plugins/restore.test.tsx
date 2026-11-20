// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { installDomStubs } from "@/test/dom"
import { HELLO, createFakePluginClient, readyRuntime, type FakeOptions } from "@/test/plugins"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { PluginClientContext, PluginError } from "./api"
import { PluginsSettings } from "./marketplace"
import type { KnownPlugin } from "./types"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("./client", () => ({
  createTauriPluginClient: () => {
    throw new Error("the Tauri client is not used in tests")
  },
}))

beforeAll(installDomStubs)
beforeEach(resetToasts)
afterEach(cleanup)

/** An agent that can restore DeckPilot's own plugins (protocol 15). */
const restoring = readyRuntime({ features: ["restoreBundled"] })

/** `Hello Weather` as a bundled plugin the user uninstalled. */
const removed: KnownPlugin = {
  id: HELLO,
  name: "Hello Weather",
  publisher: "acme",
  namespace: "plugins.acme.helloWeather",
  lastVersion: "1.0.0",
  installed: false,
  firstSeenAt: 1_767_225_600,
  lastSeenAt: 1_767_225_600,
  restorable: true,
}

function renderInstalled(options: FakeOptions) {
  const client = createFakePluginClient(options)
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginsSettings initialTab="installed" />
    </PluginClientContext>,
  )
  return { ...result, client }
}

describe("removed DeckPilot plugins", () => {
  it("lists a removed bundled plugin and restores it from DeckPilot's own copy", async () => {
    const { user, client } = renderInstalled({ runtime: restoring, known: [removed] })
    const list = await screen.findByRole("list", { name: "Removed DeckPilot plugins" })
    expect(screen.getByText("No plugins installed")).toBeInTheDocument()
    expect(within(list).getByText("Hello Weather")).toBeInTheDocument()
    expect(within(list).getByText("Last installed: version 1.0.0")).toBeInTheDocument()

    await user.click(within(list).getByRole("button", { name: "Restore Hello Weather" }))
    expect(client.restoreBundledPlugin).toHaveBeenCalledWith(HELLO)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather restored")
    })
    // Read again: installed now, and no longer among the removed ones.
    const installed = await screen.findByRole("list", { name: "Installed plugins" })
    expect(within(installed).getByText("Hello Weather")).toBeInTheDocument()
    expect(within(installed).getByText("Bundled")).toBeInTheDocument()
    expect(
      screen.queryByRole("list", { name: "Removed DeckPilot plugins" }),
    ).not.toBeInTheDocument()
  })

  it("offers nothing to restore with an agent that cannot restore", async () => {
    renderInstalled({ runtime: readyRuntime(), known: [removed] })
    await screen.findByText("No plugins installed")
    expect(
      screen.queryByRole("list", { name: "Removed DeckPilot plugins" }),
    ).not.toBeInTheDocument()
  })

  it("lists only plugins the agent can restore", async () => {
    renderInstalled({
      runtime: restoring,
      known: [{ ...removed, restorable: false }],
    })
    await screen.findByText("No plugins installed")
    expect(screen.queryByRole("button", { name: /Restore/ })).not.toBeInTheDocument()
  })

  it("reports a refused restore with what it means", async () => {
    const { user, client } = renderInstalled({ runtime: restoring, known: [removed] })
    vi.mocked(client.restoreBundledPlugin).mockRejectedValueOnce(
      new PluginError("invalidPackage", "the bundled copy failed verification"),
    )
    await user.click(await screen.findByRole("button", { name: "Restore Hello Weather" }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalled()
    })
    const call: unknown[] = vi.mocked(toast.error).mock.calls[0] ?? []
    const title = call[0]
    const options = call[1] as { description: string }
    expect(title).toBe("The package was refused")
    expect(options.description).toContain("the bundled copy failed verification")
    expect(screen.getByRole("list", { name: "Removed DeckPilot plugins" })).toBeInTheDocument()
  })
})

describe("a broken bundled plugin", () => {
  const broken: FakeOptions = {
    runtime: restoring,
    helloInstalled: true,
    helloState: "broken",
    helloStateReason: "plugin.wasm differs from the pins",
    helloOrigin: { type: "bundled" },
  }

  it("is reinstalled from DeckPilot's own copy", async () => {
    const { user, client } = renderInstalled(broken)
    const reinstall = await screen.findByRole("button", { name: "Reinstall Hello Weather" })
    expect(
      screen.getByText(
        "Its files no longer match the copy that comes with DeckPilot. Reinstall it from that copy; its settings and your permission choices are kept.",
      ),
    ).toBeInTheDocument()
    await user.click(reinstall)
    expect(client.restoreBundledPlugin).toHaveBeenCalledWith(HELLO)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather reinstalled")
    })
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "Reinstall Hello Weather" }),
      ).not.toBeInTheDocument()
    })
    expect(screen.getByRole("switch", { name: "Disable Hello Weather" })).toBeChecked()
  })

  it("is not offered for a plugin from the registry or an older agent", async () => {
    renderInstalled({ ...broken, helloOrigin: { type: "registry", registry: "/tmp/registry" } })
    await screen.findByText("Hello Weather is broken")
    expect(screen.queryByRole("button", { name: /Reinstall/ })).not.toBeInTheDocument()
    cleanup()
    renderInstalled({ ...broken, runtime: readyRuntime() })
    await screen.findByText("Hello Weather is broken")
    expect(screen.queryByRole("button", { name: /Reinstall/ })).not.toBeInTheDocument()
    expect(
      screen.getByText("It cannot run or be enabled. Roll back, update or uninstall it."),
    ).toBeInTheDocument()
  })
})

describe("in Polish", () => {
  beforeEach(async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
  })
  afterEach(async () => {
    cleanup()
    await act(async () => {
      await i18n.changeLanguage("en")
    })
  })

  it("restores a removed plugin with Przywróć", async () => {
    const { user, client } = renderInstalled({ runtime: restoring, known: [removed] })
    const list = await screen.findByRole("list", { name: "Usunięte wtyczki DeckPilot" })
    expect(screen.getByRole("heading", { name: "Usunięte wtyczki DeckPilot" })).toBeInTheDocument()
    expect(within(list).getByText("Ostatnio zainstalowana wersja: 1.0.0")).toBeInTheDocument()
    const restore = within(list).getByRole("button", { name: "Przywróć: Hello Weather" })
    expect(restore).toHaveTextContent("Przywróć")
    await user.click(restore)
    expect(client.restoreBundledPlugin).toHaveBeenCalledWith(HELLO)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Przywrócono: Hello Weather")
    })
  })

  it("reinstalls a broken plugin with Zainstaluj ponownie", async () => {
    const { user } = renderInstalled({
      runtime: restoring,
      helloInstalled: true,
      helloState: "broken",
      helloOrigin: { type: "bundled" },
    })
    const reinstall = await screen.findByRole("button", {
      name: "Zainstaluj ponownie: Hello Weather",
    })
    expect(reinstall).toHaveTextContent("Zainstaluj ponownie")
    await user.click(reinstall)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Zainstalowano ponownie: Hello Weather")
    })
  })
})
