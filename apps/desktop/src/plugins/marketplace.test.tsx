// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { PluginClientContext } from "./api"
import { FIXTURE_LABEL, createFixtureClient, type FixtureOptions } from "./fixtures"
import { PluginsSettings } from "./marketplace"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(resetToasts)
afterEach(cleanup)

function renderMarketplace(options: FixtureOptions = {}) {
  const client = createFixtureClient(options)
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginsSettings />
    </PluginClientContext>,
  )
  return { ...result, client }
}

const cards = () => within(screen.getByRole("list", { name: "Plugins" })).getAllByRole("listitem")
const tab = (name: RegExp) => screen.getByRole("tab", { name })
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

describe("marketplace states", () => {
  it("labels fixture data and shows loading first", async () => {
    renderMarketplace({ delayMs: 20 })
    expect(screen.getByText("Loading…")).toBeInTheDocument()
    expect(screen.getByRole("note", { name: FIXTURE_LABEL })).toBeInTheDocument()
    expect(await screen.findByRole("list", { name: "Plugins" })).toBeInTheDocument()
    expect(screen.getByText(FIXTURE_LABEL)).toBeInTheDocument()
  })

  it("reports a failure with a recovery hint and retries", async () => {
    const { user } = renderMarketplace({ failList: true })
    expect(await screen.findByText("Plugins could not be loaded")).toBeInTheDocument()
    expect(
      screen.getByText("Check Settings › Background Agent, then try again."),
    ).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Try again" }))
    expect(await screen.findByText("Plugins could not be loaded")).toBeInTheDocument()
  })

  it("shows empty states", async () => {
    const { user } = renderMarketplace({ empty: true })
    expect(await screen.findByText("No plugins to show")).toBeInTheDocument()
    await user.click(tab(/Installed/))
    expect(screen.getByText("No plugins installed")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Browse plugins" }))
    expect(tab(/Discover/)).toHaveAttribute("aria-selected", "true")
    await user.click(tab(/Updates/))
    expect(screen.getByText("All plugins are up to date")).toBeInTheDocument()
  })

  it("browses nothing before the registry was ever reached", async () => {
    renderMarketplace({ registry: "never-reached" })
    expect(await screen.findByText("Registry not reached yet")).toBeInTheDocument()
    expect(
      screen.getByText("The registry has not been reached on this computer yet."),
    ).toBeInTheDocument()
  })

  it.each([
    ["offline", "Offline: showing the last verified registry"],
    ["stale", "Registry stale"],
    ["verification-failed", "Registry verification failed"],
  ] as const)("pauses installs while the registry is %s", async (registry, title) => {
    const { user } = renderMarketplace({ registry })
    expect(await screen.findByText(title)).toBeInTheDocument()
    expect(screen.getByText(/Installed plugins keep running/)).toBeInTheDocument()
    // Cached data stays browsable, but nothing can be installed or updated.
    await openDetails(user, "Home Lights")
    expect(screen.getByRole("button", { name: /^Install/ })).toBeDisabled()
    await user.click(screen.getByRole("button", { name: "Back to plugins" }))
    await user.click(tab(/Updates/))
    for (const button of screen.getAllByRole("button", { name: /update/i })) {
      if (button.textContent.trim() !== "Check for updates") expect(button).toBeDisabled()
    }
  })

  it("switches the simulated registry state from the fixture notice", async () => {
    const { user } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(screen.getByRole("radio", { name: "Stale" }))
    expect(await screen.findByText("Registry stale")).toBeInTheDocument()
    await user.click(screen.getByRole("radio", { name: "Up to date" }))
    await waitFor(() => {
      expect(screen.queryByText("Registry stale")).not.toBeInTheDocument()
    })
  })
})

describe("discover", () => {
  it("lists browsable plugins and hides deprecated ones", async () => {
    renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    const names = cards().map((card) => card.querySelector(".font-medium")?.textContent)
    expect(names).toContain("GitHub Pull Requests")
    expect(names).not.toContain("Old Clock")
    expect(within(cards()[3] as HTMLElement).getByText(/publisher/)).toBeInTheDocument()
  })

  it("searches and filters by category", async () => {
    const { user } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(screen.getByRole("radio", { name: "Smart home" }))
    expect(cards()).toHaveLength(1)
    expect(cards()[0]).toHaveTextContent("Home Lights")
    await user.click(screen.getByRole("radio", { name: "All" }))
    await user.type(screen.getByLabelText("Search plugins"), "pull requests")
    expect(cards()).toHaveLength(1)
    expect(cards()[0]).toHaveTextContent("Installed 1.2.0")
    expect(cards()[0]).toHaveTextContent("Update available")
    await user.clear(screen.getByLabelText("Search plugins"))
    await user.type(screen.getByLabelText("Search plugins"), "nothing like this")
    expect(screen.getByText("No plugins found")).toBeInTheDocument()
  })
})

describe("plugin details", () => {
  it("shows publisher, version, compatibility, platforms and permissions by risk", async () => {
    const { user } = renderMarketplace()
    const heading = await openDetails(user, "GitHub Pull Requests")
    expect(heading).toHaveFocus()
    expect(screen.getByText("Verified publisher")).toBeInTheDocument()
    // The version Discover offers; the installed one is in the status.
    expect(screen.getByText("Version 1.3.0")).toBeInTheDocument()
    expect(screen.getByText("Works with DeckPilot 0.6.0 on macOS")).toBeInTheDocument()
    expect(screen.getByText("macOS, Windows, Linux")).toBeInTheDocument()
    const status = screen.getByRole("region", { name: "Install status" })
    expect(status).toHaveTextContent("Installed 1.2.0")
    expect(within(status).getByRole("button", { name: "Update to 1.3.0" })).toBeEnabled()
    const permissions = screen.getByRole("group", { name: "Permissions" })
    const high = within(permissions).getByRole("region", { name: "High risk" })
    expect(high).toHaveTextContent("Use the secret “token” that you store for it")
    const medium = within(permissions).getByRole("region", { name: "Medium risk" })
    expect(medium).toHaveTextContent("Send data to and receive data from https://api.github.com")
    expect(medium).toHaveTextContent("Reads the pull requests that wait for your review.")

    await user.click(screen.getByRole("button", { name: "Back to plugins" }))
    expect(screen.getByRole("list", { name: "Plugins" })).toBeInTheDocument()
  })

  it("explains why an incompatible plugin cannot be installed", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Weather Keys")
    expect(screen.getByText("Not compatible with this DeckPilot")).toBeInTheDocument()
    const reasons = screen.getByRole("list", { name: "Incompatibility reasons" })
    expect(reasons).toHaveTextContent("Requires DeckPilot >=0.9.0; this is DeckPilot 0.6.0.")
    expect(reasons).toHaveTextContent("Not available for macOS; supports Windows.")
    expect(screen.getByRole("button", { name: /^Install/ })).toBeDisabled()
    expect(screen.getByText("Verified publisher")).toBeInTheDocument()
  })

  it("offers an older version when the newest was withdrawn", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Chat Bridge")
    expect(screen.getByRole("button", { name: "Install 1.0.0" })).toBeEnabled()
    expect(screen.getByRole("list", { name: "Versions" })).toHaveTextContent(
      "Withdrawn: Crashes on start.",
    )
  })

  it("shows revocations by the registry", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Clipboard Sync")
    expect(screen.getByText("Revoked by the registry")).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Versions" })).toHaveTextContent("Revoked")
  })
})

describe("consent", () => {
  it("cannot install without consent to every permission", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Home Lights")
    await user.click(screen.getByRole("button", { name: "Install 0.3.0" }))
    const dialog = screen.getByRole("dialog", { name: "Install Home Lights 0.3.0?" })
    const install = within(dialog).getByRole("button", { name: "Install" })
    expect(install).toBeDisabled()
    expect(dialog).toHaveTextContent(FIXTURE_LABEL)
    // High-risk permissions are listed and need their own check.
    const boxes = within(dialog).getAllByRole("checkbox")
    expect(boxes).toHaveLength(4)
    expect(
      within(dialog).getByRole("checkbox", { name: /local network that you add/ }),
    ).not.toBeChecked()
    await user.click(within(dialog).getByRole("checkbox", { name: /https:\/\/api\.lifx\.com/ }))
    expect(dialog).toHaveTextContent("3 of 4 new permissions still need your consent.")
    expect(install).toBeDisabled()
    // "Allow all" is an explicit action of its own.
    await user.click(within(dialog).getByRole("button", { name: "Allow all 4" }))
    expect(install).toBeEnabled()
    await user.click(install)
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(toast.success).toHaveBeenCalledWith("Home Lights installed")
    expect(screen.getByRole("region", { name: "Install status" })).toHaveTextContent(
      "Installed 0.3.0",
    )
  })

  it("keeps focus in the dialog and installs nothing when cancelled", async () => {
    const { user, client } = renderMarketplace()
    await openDetails(user, "Home Lights")
    await user.click(screen.getByRole("button", { name: "Install 0.3.0" }))
    const dialog = screen.getByRole("dialog")
    expect(dialog).toContainElement(document.activeElement as HTMLElement)
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    expect((await client.listPlugins()).installed.some((p) => p.id === "knit.home-lights")).toBe(
      false,
    )
  })

  it("reports a package that fails verification", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Calendar Keys")
    await user.click(screen.getByRole("button", { name: "Install 1.0.0" }))
    const dialog = screen.getByRole("dialog")
    await user.click(within(dialog).getByRole("checkbox"))
    await user.click(within(dialog).getByRole("button", { name: "Install" }))
    expect(await within(dialog).findByText(/Verification failed/)).toBeInTheDocument()
    expect(
      within(dialog).getByText(/report the plugin to the registry maintainers/),
    ).toBeInTheDocument()
  })

  it("asks only for the new permissions of an update", async () => {
    const { user } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Updates/))
    const updates = screen.getByRole("list", { name: "Available updates" })
    const item = within(within(updates).getByRole("listitem", { name: /GitHub Pull Requests/ }))
    expect(item.getByText("1 new permission")).toBeInTheDocument()
    await user.click(item.getByRole("button", { name: "Review and update" }))
    const dialog = screen.getByRole("dialog", { name: "Update GitHub Pull Requests to 1.3.0?" })
    expect(within(dialog).getAllByRole("checkbox")).toHaveLength(1)
    expect(within(dialog).getAllByText("Already granted")).toHaveLength(4)
    const update = within(dialog).getByRole("button", { name: "Update" })
    expect(update).toBeDisabled()
    await user.click(
      within(dialog).getByRole("checkbox", { name: /https:\/\/github\.com in your browser/ }),
    )
    await user.click(update)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("GitHub Pull Requests updated to 1.3.0")
    })
  })

  it("applies an update that asks for less without a dialog", async () => {
    const { user } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Updates/))
    const item = within(screen.getByRole("listitem", { name: /Status Page Monitor/ }))
    expect(item.getByText("No new permissions")).toBeInTheDocument()
    await user.click(item.getByRole("button", { name: "Update" }))
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Status Page Monitor updated to 2.0.1")
    })
  })

  it("reviews permissions before enabling and keeps revoked ones revoked", async () => {
    const { user, client } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Installed/))
    await user.click(
      installedItem("Focus Timer").getByRole("switch", { name: "Enable Focus Timer" }),
    )
    const dialog = screen.getByRole("dialog", { name: "Enable Focus Timer?" })
    expect(within(dialog).getByText("Already granted")).toBeInTheDocument()
    expect(
      within(dialog).getByRole("checkbox", { name: /Allow again: Show notifications/ }),
    ).not.toBeChecked()
    await user.click(within(dialog).getByRole("button", { name: "Enable" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Focus Timer enabled")
    })
    const timer = (await client.listPlugins()).installed.find((p) => p.id === "tempo.focus-timer")
    expect(timer?.enabled).toBe(true)
    expect(timer?.grants.find((g) => g.key === "notifications")?.enabled).toBe(false)
  })

  it("revokes and allows again a single permission", async () => {
    const { user } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Installed/))
    const item = () => installedItem("GitHub Pull Requests")
    await user.click(item().getByRole("button", { name: "Permissions" }))
    await user.click(
      item().getByRole("button", {
        name: "Revoke: Send data to and receive data from https://api.github.com",
      }),
    )
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Permission revoked")
    })
    expect(item().getByText(/1 permission revoked/)).toBeInTheDocument()
    await user.click(
      item().getByRole("button", {
        name: "Allow again: Send data to and receive data from https://api.github.com",
      }),
    )
    await waitFor(() => {
      expect(item().queryByText(/permission revoked/)).not.toBeInTheDocument()
    })
  })
})

describe("installed plugins", () => {
  it("reports problems with recovery options", async () => {
    const { user } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Installed/))

    const deploy = installedItem("Deploy Buttons")
    expect(deploy.getByText("Deploy Buttons failed to load")).toBeInTheDocument()
    expect(deploy.getByText(/Roll back to the previous version or uninstall/)).toBeInTheDocument()
    await user.click(deploy.getByRole("button", { name: "Roll back to 0.8.2" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Deploy Buttons rolled back to 0.8.2")
    })
    expect(installedItem("Deploy Buttons").queryByText("Deploy Buttons failed to load")).toBeNull()

    const clipboard = installedItem("Clipboard Sync")
    expect(clipboard.getByText("Disabled: revoked by the registry")).toBeInTheDocument()
    expect(clipboard.getByRole("switch", { name: "Enable Clipboard Sync" })).toBeDisabled()

    const status = installedItem("Status Page Monitor")
    expect(status.getByText("Version 2.0.0 was withdrawn")).toBeInTheDocument()
    expect(status.getByText(/Updating to 2.0.1 is recommended/)).toBeInTheDocument()
  })

  it("disables and uninstalls a plugin", async () => {
    const { user } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Installed/))
    await user.click(
      installedItem("GitHub Pull Requests").getByRole("switch", {
        name: "Disable GitHub Pull Requests",
      }),
    )
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("GitHub Pull Requests disabled")
    })
    await user.click(
      installedItem("GitHub Pull Requests").getByRole("button", { name: "Uninstall" }),
    )
    const confirm = screen.getByRole("alertdialog", { name: "Uninstall GitHub Pull Requests?" })
    expect(
      within(confirm).getByRole("checkbox", { name: "Also delete the secrets stored for it" }),
    ).not.toBeChecked()
    await user.click(within(confirm).getByRole("button", { name: "Uninstall" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("GitHub Pull Requests uninstalled")
    })
    expect(tab(/Installed/)).toHaveTextContent("Installed 4")
  })

  it("reports a failed request as a toast with its hint", async () => {
    const { user, client } = renderMarketplace()
    await screen.findByRole("list", { name: "Plugins" })
    await user.click(tab(/Installed/))
    vi.spyOn(client, "rollbackPlugin").mockRejectedValue(new Error("The agent is busy."))
    await user.click(
      installedItem("Deploy Buttons").getByRole("button", { name: "Roll back to 0.8.2" }),
    )
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("The agent is busy.", undefined)
    })
  })
})
