// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import {
  HELLO,
  LIGHTS,
  createFakePluginClient,
  readyRuntime,
  type FakeOptions,
} from "@/test/plugins"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { PluginClientContext } from "./api"
import { PluginsSettings } from "./marketplace"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
// The default client must never be created in these tests.
vi.mock("./client", () => ({
  createTauriPluginClient: () => {
    throw new Error("the Tauri client is not used in tests")
  },
}))

beforeAll(installDomStubs)
beforeEach(resetToasts)
afterEach(cleanup)

function renderMarketplace(options: FakeOptions = {}) {
  const client = createFakePluginClient(options)
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginsSettings />
    </PluginClientContext>,
  )
  return { ...result, client }
}

const tab = (name: RegExp) => screen.getByRole("tab", { name })
const cards = () => within(screen.getByRole("list", { name: "Plugins" })).getAllByRole("listitem")
const installedItem = (name: string) =>
  within(
    within(screen.getByRole("list", { name: "Installed plugins" }))
      .getByText(name, { selector: "span" })
      .closest("li") as HTMLElement,
  )

async function openDetails(user: ReturnType<typeof renderMarketplace>["user"], name: string) {
  await user.click(await screen.findByRole("button", { name: new RegExp(name) }))
  return screen.findByRole("heading", { name, level: 3 })
}

async function openInstalled(user: ReturnType<typeof renderMarketplace>["user"]) {
  await screen.findByRole("list", { name: "Plugins" })
  await user.click(tab(/Installed/))
}

describe("runtime availability", () => {
  it.each([
    [
      "not built",
      {
        runtime: {
          ...readyRuntime(),
          available: false,
          built: false,
          enabled: false,
          reason: "built without plugin support",
        },
      },
      "Plugins are not available: the agent was built without plugin support",
    ],
    [
      "not enabled",
      { runtime: { ...readyRuntime(), available: false, enabled: false } },
      "Plugins are not available: they are not enabled in the agent",
    ],
    [
      "locked",
      { runtime: { ...readyRuntime(), available: false, reason: "the configuration is locked" } },
      "Plugins are not available right now",
    ],
    [
      "no agent",
      { runtimeError: { reason: "agentNotRunning", message: "the agent is not running" } },
      "Plugins are not available: the agent is not running",
    ],
    [
      "an old agent",
      { runtimeError: { reason: "agentTooOld", message: "protocol 9" } },
      "Plugins are not available: the running agent is too old",
    ],
  ] as const)("says why with %s and asks nothing else", async (_, options, title) => {
    const { client, user } = renderMarketplace(options)
    expect(await screen.findByText(title)).toBeInTheDocument()
    expect(screen.queryByRole("tab")).not.toBeInTheDocument()
    expect(client.listPlugins).not.toHaveBeenCalled()
    expect(client.listAvailablePlugins).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Check again" }))
    await waitFor(() => {
      expect(client.getRuntime).toHaveBeenCalledTimes(2)
    })
  })

  it("installs but does not enable or run on an unsupported platform", async () => {
    const { user } = renderMarketplace({
      runtime: readyRuntime({ executionSupported: false, platform: "windows" }),
      helloInstalled: true,
    })
    expect(await screen.findByText("Plugins cannot run on Windows")).toBeInTheDocument()
    await openInstalled(user)
    const item = installedItem("Hello Weather")
    await user.click(item.getByRole("switch", { name: "Disable Hello Weather" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather disabled")
    })
    expect(
      installedItem("Hello Weather").getByRole("switch", { name: "Enable Hello Weather" }),
    ).toBeDisabled()
  })

  it("reports other failures with the backend's hint and retries", async () => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.listPlugins).mockRejectedValueOnce(
      Object.assign(new Error("database is busy"), { reason: "storeUnavailable" }),
    )
    // A plain Error without the PluginError class still shows its message.
    expect(await screen.findByText("Plugins could not be loaded")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Try again" }))
    expect(await screen.findByRole("list", { name: "Plugins" })).toBeInTheDocument()
  })
})

describe("registry", () => {
  it.each([
    [{ freshness: "stale" as const }, "Registry stale"],
    [{ freshness: "neverVerified" as const }, "Registry not verified yet"],
  ])("pauses installs while the registry is %o", async (registry, title) => {
    const { user } = renderMarketplace({
      runtime: readyRuntime({ registry: { source: "/tmp/registry", ...registry } }),
    })
    expect(await screen.findByText(title)).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Install from package file/ })).toBeDisabled()
    await openDetails(user, "Home Lights")
    expect(screen.getByRole("button", { name: /^Install/ })).toBeDisabled()
  })

  it("shows a registry that cannot be read and checks it again with refresh", async () => {
    const { user, client } = renderMarketplace({
      registryError: {
        reason: "registryUnavailable",
        message: "the timestamp signature is invalid",
      },
    })
    expect(await screen.findByText("The registry could not be read")).toBeInTheDocument()
    expect(screen.getByText("the timestamp signature is invalid")).toBeInTheDocument()
    expect(screen.getByText("No plugins to show")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Check the registry" }))
    await waitFor(() => {
      expect(client.listAvailablePlugins).toHaveBeenLastCalledWith(true)
    })
  })

  it("says when no registry is configured", async () => {
    renderMarketplace({ runtime: { ...readyRuntime(), registry: undefined }, available: [] })
    expect(await screen.findByText("No plugin registry configured")).toBeInTheDocument()
  })
})

describe("discover and details", () => {
  it("lists and searches the registry's plugins", async () => {
    const { user } = renderMarketplace({ helloInstalled: true })
    await screen.findByRole("list", { name: "Plugins" })
    expect(cards()).toHaveLength(4)
    await user.type(screen.getByLabelText("Search plugins"), "weather")
    expect(cards()).toHaveLength(1)
    expect(cards()[0]).toHaveTextContent("Installed 1.0.0")
    expect(cards()[0]).toHaveTextContent("Update available")
    await user.clear(screen.getByLabelText("Search plugins"))
    await user.type(screen.getByLabelText("Search plugins"), "nothing like this")
    expect(screen.getByText("No plugins found")).toBeInTheDocument()
  })

  it("explains incompatible and withdrawn versions", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Future Keys")
    expect(screen.getByRole("list", { name: "Incompatibility reasons" })).toHaveTextContent(
      "requires DeckPilot >=9.0.0",
    )
    expect(screen.getByRole("button", { name: /^Install/ })).toBeDisabled()
    await user.click(screen.getByRole("button", { name: "Back to plugins" }))
    await openDetails(user, "Gone Plugin")
    expect(screen.getByText("Version 1.0.0 was withdrawn")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /^Install/ })).toBeDisabled()
  })
})

describe("install", () => {
  it("sends exactly the consented grant keys", async () => {
    const { user, client } = renderMarketplace()
    const heading = await openDetails(user, "Home Lights")
    expect(heading).toHaveFocus()
    await user.click(screen.getByRole("button", { name: "Install 0.3.0" }))
    const dialog = screen.getByRole("dialog", { name: "Install Home Lights 0.3.0?" })
    const install = within(dialog).getByRole("button", { name: "Install" })
    expect(within(dialog).getAllByRole("checkbox")).toHaveLength(5)
    expect(install).toBeDisabled()
    await user.click(within(dialog).getByRole("checkbox", { name: /local network/ }))
    expect(dialog).toHaveTextContent("4 of 5 permissions still need your consent.")
    await user.click(within(dialog).getByRole("button", { name: "Allow all 5" }))
    await user.click(install)
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(client.installPlugin).toHaveBeenCalledWith(
      { type: "registry", plugin: LIGHTS, version: "0.3.0" },
      [
        "network:api.lifx.com",
        "network:private-network",
        "credentials:token",
        "device:state",
        "device:icons",
      ],
    )
    expect(toast.success).toHaveBeenCalledWith("Home Lights installed")
    expect(await screen.findByRole("region", { name: "Install status" })).toHaveTextContent(
      "Installed 0.3.0 (disabled)",
    )
  })

  it("installs nothing when cancelled", async () => {
    const { user, client } = renderMarketplace()
    await openDetails(user, "Home Lights")
    await user.click(screen.getByRole("button", { name: "Install 0.3.0" }))
    const dialog = screen.getByRole("dialog")
    expect(dialog).toContainElement(document.activeElement as HTMLElement)
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    expect(client.installPlugin).not.toHaveBeenCalled()
  })

  it("shows a refusal in the dialog with the backend's hint", async () => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.installPlugin).mockRejectedValueOnce(
      Object.assign(new (await import("./api")).PluginError("revoked", "the version was revoked")),
    )
    await openDetails(user, "Hello Weather")
    await user.click(screen.getByRole("button", { name: "Install 1.1.0" }))
    const dialog = screen.getByRole("dialog")
    await user.click(within(dialog).getByRole("button", { name: "Allow all 2" }))
    await user.click(within(dialog).getByRole("button", { name: "Install" }))
    expect(await within(dialog).findByText("the version was revoked")).toBeInTheDocument()
    expect(
      within(dialog).getByText(
        "The registry withdrew this version; choose another one or uninstall it.",
      ),
    ).toBeInTheDocument()
  })

  it("installs from a local package file without granting anything", async () => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.pickPackage).mockResolvedValueOnce("/tmp/acme.hello-weather-1.0.0.dpplugin")
    await user.click(await screen.findByRole("button", { name: /Install from package file/ }))
    await waitFor(() => {
      expect(client.installPlugin).toHaveBeenCalledWith(
        { type: "file", path: "/tmp/acme.hello-weather-1.0.0.dpplugin" },
        [],
      )
    })
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalled()
    })
    await user.click(tab(/Installed/))
    expect(installedItem("Hello Weather").getByText("Local package")).toBeInTheDocument()
  })

  it("reports a package the registry does not pin", async () => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.pickPackage).mockResolvedValueOnce("/tmp/other.dpplugin")
    await user.click(await screen.findByRole("button", { name: /Install from package file/ }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("the package is not a version the registry pins", {
        description: "Only package files whose bytes the verified registry pins can be installed.",
      })
    })
  })

  it("does nothing when the file dialog is cancelled", async () => {
    const { user, client } = renderMarketplace()
    await user.click(await screen.findByRole("button", { name: /Install from package file/ }))
    await waitFor(() => {
      expect(client.pickPackage).toHaveBeenCalled()
    })
    expect(client.installPlugin).not.toHaveBeenCalled()
  })

  it("shows the agent's progress and reloads when a plugin changed", async () => {
    const { client } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    act(() => {
      client.emit({ type: "progress", plugin: HELLO, operation: "install", stage: "verifying" })
    })
    expect(screen.getByRole("status", { name: "Plugin operations" })).toHaveTextContent(
      `Installing ${HELLO}: verifying signatures and pins`,
    )
    act(() => {
      client.emit({ type: "progress", plugin: HELLO, operation: "install", stage: "finished" })
    })
    expect(screen.queryByRole("status", { name: "Plugin operations" })).not.toBeInTheDocument()
    const calls = vi.mocked(client.listPlugins).mock.calls.length
    act(() => {
      client.emit({ type: "changed", plugin: HELLO, state: "disabled" })
    })
    await waitFor(() => {
      expect(vi.mocked(client.listPlugins).mock.calls.length).toBe(calls + 1)
    })
  })
})

describe("installed plugins", () => {
  it("enables with consent to every declared permission", async () => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.pickPackage).mockResolvedValueOnce("/tmp/acme.hello-weather-1.0.0.dpplugin")
    await user.click(await screen.findByRole("button", { name: /Install from package file/ }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalled()
    })
    await user.click(tab(/Installed/))
    expect(installedItem("Hello Weather").getByText(/1 permission not granted/)).toBeInTheDocument()
    await user.click(
      installedItem("Hello Weather").getByRole("switch", { name: "Enable Hello Weather" }),
    )
    const dialog = screen.getByRole("dialog", { name: "Enable Hello Weather?" })
    const enable = within(dialog).getByRole("button", { name: "Enable" })
    expect(enable).toBeDisabled()
    await user.click(within(dialog).getByRole("checkbox", { name: /api\.open-meteo\.com/ }))
    await user.click(enable)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather enabled")
    })
    expect(client.enablePlugin).toHaveBeenCalledWith(HELLO, ["network:api.open-meteo.com"])
  })

  it("runs actions, revokes, sees the refusal, grants again and disables", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await openInstalled(user)
    const item = () => installedItem("Hello Weather")
    await user.click(item().getByRole("button", { name: "Permissions and actions" }))

    await user.type(item().getByLabelText("Name"), "Ada")
    await user.click(item().getByRole("button", { name: "Run Greet" }))
    expect(await item().findByRole("status", { name: "Result of Greet" })).toHaveTextContent(
      "Succeeded: Hello, Ada.",
    )
    expect(client.invokeAction).toHaveBeenCalledWith(HELLO, "greet", { name: "Ada" })

    await user.click(
      item().getByRole("button", {
        name: "Revoke: Send data to and receive data from https://api.open-meteo.com",
      }),
    )
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Permission revoked")
    })
    expect(client.revokePermission).toHaveBeenCalledWith(HELLO, "network:api.open-meteo.com")
    await user.click(item().getByRole("button", { name: "Run Current temperature" }))
    expect(
      await item().findByRole("status", { name: "Result of Current temperature" }),
    ).toHaveTextContent("Failed (not-permitted)")

    await user.click(
      item().getByRole("button", {
        name: "Grant: Send data to and receive data from https://api.open-meteo.com",
      }),
    )
    await waitFor(() => {
      expect(client.grantPermissions).toHaveBeenCalledWith(HELLO, ["network:api.open-meteo.com"])
    })

    await user.click(item().getByRole("switch", { name: "Disable Hello Weather" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather disabled")
    })
    expect(item().getByRole("button", { name: "Run Greet" })).toBeDisabled()
  })

  it("reports a refused action with its reason", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    vi.mocked(client.invokeAction).mockRejectedValueOnce(
      new (await import("./api")).PluginError("hostFailed", "the host crashed"),
    )
    await openInstalled(user)
    await user.click(
      installedItem("Hello Weather").getByRole("button", { name: "Permissions and actions" }),
    )
    await user.click(installedItem("Hello Weather").getByRole("button", { name: "Run Greet" }))
    expect(
      await installedItem("Hello Weather").findByRole("status", { name: "Result of Greet" }),
    ).toHaveTextContent("the host crashed The plugin host failed")
  })

  it("uninstalls after confirmation", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await openInstalled(user)
    await user.click(installedItem("Hello Weather").getByRole("button", { name: "Uninstall" }))
    const confirm = screen.getByRole("alertdialog", { name: "Uninstall Hello Weather?" })
    await user.click(within(confirm).getByRole("button", { name: "Uninstall" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather uninstalled")
    })
    expect(client.uninstallPlugin).toHaveBeenCalledWith(HELLO)
    expect(await screen.findByText("No plugins installed")).toBeInTheDocument()
  })

  it("reports a failed request as a toast with the reason's hint", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    vi.mocked(client.disablePlugin).mockRejectedValueOnce(
      new (await import("./api")).PluginError("locked", "the configuration is locked"),
    )
    await openInstalled(user)
    await user.click(
      installedItem("Hello Weather").getByRole("switch", { name: "Disable Hello Weather" }),
    )
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("the configuration is locked", {
        description: "Unlock DeckPilot, then try again.",
      })
    })
  })
})

describe("updates", () => {
  it("stages an escalation, approves it with exactly the new keys and rolls back", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Updates 1/))
    const row = within(screen.getByRole("listitem", { name: /Hello Weather 1.0.0 to 1.1.0/ }))
    await user.click(row.getByRole("button", { name: "Update" }))
    expect(client.updatePlugin).toHaveBeenCalledWith(HELLO, "1.1.0", [])
    const dialog = await screen.findByRole("dialog", { name: "Update Hello Weather to 1.1.0?" })
    expect(within(dialog).getAllByRole("checkbox")).toHaveLength(1)
    const update = within(dialog).getByRole("button", { name: "Update" })
    expect(update).toBeDisabled()
    await user.click(within(dialog).getByRole("checkbox", { name: /Show notifications/ }))
    await user.click(update)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather updated to 1.1.0")
    })
    expect(client.approveUpdate).toHaveBeenCalledWith(HELLO, ["notifications"])

    await user.click(tab(/Installed/))
    await user.click(
      installedItem("Hello Weather").getByRole("button", { name: "Roll back to 1.0.0" }),
    )
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather rolled back to 1.0.0")
    })
    expect(client.rollbackPlugin).toHaveBeenCalledWith(HELLO)
  })

  it("keeps a cancelled escalation staged and declines it later", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Updates/))
    await user.click(screen.getByRole("button", { name: "Update" }))
    await user.click(
      within(await screen.findByRole("dialog")).getByRole("button", { name: "Cancel" }),
    )
    const row = within(await screen.findByRole("listitem", { name: /Hello Weather/ }))
    expect(row.getByText("Waiting for consent: 1 permission")).toBeInTheDocument()
    await user.click(row.getByRole("button", { name: "Review and update" }))
    const dialog = screen.getByRole("dialog", { name: "Update Hello Weather to 1.1.0?" })
    await user.click(within(dialog).getByRole("button", { name: "Decline update" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Update of Hello Weather declined")
    })
    expect(client.declineUpdate).toHaveBeenCalledWith(HELLO)
    expect(client.approveUpdate).not.toHaveBeenCalled()
  })

  it("applies an update that asks for nothing new without a dialog", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    vi.mocked(client.updatePlugin).mockResolvedValueOnce({
      plugin: HELLO,
      outcome: {
        type: "applied",
        version: "1.1.0",
        changes: { added: [], broadened: [], removed: [], escalation: false },
      },
    })
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Updates/))
    await user.click(screen.getByRole("button", { name: "Update" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather updated to 1.1.0")
    })
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })
})
