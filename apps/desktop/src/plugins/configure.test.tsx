// @vitest-environment jsdom
// Configure (settings and credentials), uninstall retention, storage usage,
// known plugins and the classification of the marketplace, against the
// in-memory stand-in of the agent (test/plugins.ts).
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import { builtInModules } from "@/test/fixtures"
import { HELLO, LIGHTS, createFakePluginClient, type FakeOptions } from "@/test/plugins"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { PluginClientContext, PluginError } from "./api"
import { PluginsSettings } from "./marketplace"
import { secretProblem } from "./model"
import { settingsChanges, settingsProblems } from "./settings-form"
import type { PluginCredentialSlot, PluginSettings } from "./types"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("./client", () => ({
  createTauriPluginClient: () => {
    throw new Error("the Tauri client is not used in tests")
  },
}))

beforeAll(installDomStubs)
beforeEach(resetToasts)
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const SECRET = "sk-live-0123456789abcdef"

function renderMarketplace(options: FakeOptions = {}) {
  const client = createFakePluginClient({ helloInstalled: true, ...options })
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginsSettings />
    </PluginClientContext>,
    { modules: builtInModules() },
  )
  return { ...result, client }
}

type User = ReturnType<typeof renderMarketplace>["user"]

const installedItem = (name: string) =>
  within(
    within(screen.getByRole("list", { name: "Installed plugins" }))
      .getByText(name, { selector: "span" })
      .closest("li") as HTMLElement,
  )

async function openInstalled(user: User) {
  await screen.findByRole("list", { name: "Plugins" })
  await user.click(screen.getByRole("tab", { name: /Installed/ }))
}

async function openConfigure(user: User) {
  await openInstalled(user)
  await user.click(installedItem("Hello Weather").getByRole("button", { name: "Configure" }))
  const dialog = screen.getByRole("dialog", { name: "Configure Hello Weather" })
  await within(dialog).findByRole("form", { name: "Settings" })
  await within(dialog).findByRole("list", { name: "Credentials of Hello Weather" })
  return within(dialog)
}

describe("the settings form", () => {
  it("is generated from the agent's fields, with types, defaults and limits", async () => {
    const { user, client } = renderMarketplace()
    const dialog = await openConfigure(user)
    expect(client.getSettings).toHaveBeenCalledWith(HELLO)

    const units = dialog.getByRole("combobox", { name: /Units/ })
    expect(units).toHaveTextContent("Default (celsius)")
    expect(units).toHaveAccessibleDescription("Celsius or Fahrenheit. Default: celsius.")
    const refresh = dialog.getByRole("textbox", { name: /Refresh every/ })
    expect(refresh).toHaveValue("")
    expect(refresh).toHaveAttribute("placeholder", "10")
    expect(refresh).toHaveAttribute("inputmode", "numeric")
    expect(refresh).toHaveAccessibleDescription("Default: 10. at least 1, at most 60.")
    expect(dialog.getByRole("switch", { name: /Show “feels like”/ })).not.toBeChecked()
    // Settings are never secret inputs.
    expect(
      screen.getByRole("dialog").querySelector("form[aria-label=Settings] input[type=password]"),
    ).toBeNull()
    expect(dialog.getByRole("button", { name: "Save settings" })).toBeDisabled()
  })

  it("saves only what changed", async () => {
    const { user, client } = renderMarketplace()
    const dialog = await openConfigure(user)
    await user.type(dialog.getByRole("textbox", { name: /Refresh every/ }), "20")
    await user.click(dialog.getByRole("combobox", { name: /Units/ }))
    await user.click(screen.getByRole("option", { name: "fahrenheit" }))
    await user.click(dialog.getByRole("switch", { name: /Show “feels like”/ }))
    await user.click(dialog.getByRole("button", { name: "Save settings" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Settings saved")
    })
    expect(client.setSettings).toHaveBeenCalledWith(HELLO, {
      refreshMinutes: 20,
      units: "fahrenheit",
      showFeelsLike: true,
    })
    expect(client.stored(HELLO).settings).toEqual({
      refreshMinutes: 20,
      units: "fahrenheit",
      showFeelsLike: true,
    })
    // The form shows what is saved now.
    expect(dialog.getByRole("textbox", { name: /Refresh every/ })).toHaveValue("20")
    expect(dialog.getByRole("button", { name: "Save settings" })).toBeDisabled()
  })

  it("shows the agent's validation messages under their fields", async () => {
    const { user, client } = renderMarketplace()
    const dialog = await openConfigure(user)
    const refresh = dialog.getByRole("textbox", { name: /Refresh every/ })
    await user.type(refresh, "99")
    await user.click(dialog.getByRole("button", { name: "Save settings" }))
    expect(await dialog.findByText("99 is out of range")).toBeInTheDocument()
    expect(refresh).toHaveAttribute("aria-invalid", "true")
    expect(refresh).toHaveAccessibleDescription(/99 is out of range/)
    expect(client.setSettings).toHaveBeenCalledTimes(1)
    expect(client.stored(HELLO).settings).toEqual({})
    // Editing the field clears its message.
    await user.clear(refresh)
    expect(dialog.queryByText("99 is out of range")).not.toBeInTheDocument()
  })

  it("refuses what it cannot send without asking the agent", async () => {
    const { user, client } = renderMarketplace()
    const dialog = await openConfigure(user)
    await user.type(dialog.getByRole("textbox", { name: /Refresh every/ }), "ten")
    await user.click(dialog.getByRole("button", { name: "Save settings" }))
    expect(dialog.getByText("Enter a whole number.")).toBeInTheDocument()
    expect(client.setSettings).not.toHaveBeenCalled()
  })

  it("resets a saved value to its default", async () => {
    const { user, client } = renderMarketplace({ helloSettings: { units: "fahrenheit" } })
    const dialog = await openConfigure(user)
    expect(dialog.getByRole("combobox", { name: /Units/ })).toHaveTextContent("fahrenheit")
    await user.click(dialog.getByRole("button", { name: "Reset Units to its default" }))
    expect(dialog.getByRole("combobox", { name: /Units/ })).toHaveTextContent("Default (celsius)")
    await user.click(dialog.getByRole("button", { name: "Save settings" }))
    await waitFor(() => {
      expect(client.setSettings).toHaveBeenCalledWith(HELLO, { units: null })
    })
    await waitFor(() => {
      expect(client.stored(HELLO).settings).toEqual({})
    })
  })

  it("cancels without saving", async () => {
    const { user, client } = renderMarketplace()
    const dialog = await openConfigure(user)
    await user.type(dialog.getByRole("textbox", { name: /Refresh every/ }), "5")
    await user.click(dialog.getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    expect(client.setSettings).not.toHaveBeenCalled()
  })

  it("shows missing required settings until they are set", async () => {
    const { user, client } = renderMarketplace({ helloRequiresCity: true })
    await openInstalled(user)
    const item = installedItem("Hello Weather")
    expect(item.getByText("Settings missing")).toBeInTheDocument()
    expect(item.getByText(/City has no value and no default/)).toBeInTheDocument()

    await user.click(item.getByRole("button", { name: "Configure" }))
    const dialog = within(screen.getByRole("dialog", { name: "Configure Hello Weather" }))
    const city = await dialog.findByRole("textbox", { name: /City/ })
    expect(city).toHaveAttribute("aria-required", "true")
    expect(dialog.getByText("Required settings are missing")).toBeInTheDocument()
    expect(dialog.getByText("Missing")).toBeInTheDocument()
    await user.type(city, "Berlin")
    await user.click(dialog.getByRole("button", { name: "Save settings" }))
    await waitFor(() => {
      expect(client.setSettings).toHaveBeenCalledWith(HELLO, { city: "Berlin" })
    })
    await waitFor(() => {
      expect(dialog.queryByText("Required settings are missing")).not.toBeInTheDocument()
    })
    await user.click(dialog.getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(installedItem("Hello Weather").queryByText("Settings missing")).not.toBeInTheDocument()
    })
  })
})

describe("credentials", () => {
  it("lists each slot with its label, hosts and whether it is set", async () => {
    const { user, client } = renderMarketplace()
    const dialog = await openConfigure(user)
    expect(client.listCredentials).toHaveBeenCalledWith(HELLO)
    const row = within(dialog.getByRole("listitem", { name: "API key" }))
    expect(row.getByText("Not set")).toBeInTheDocument()
    expect(row.getByText(/Sent only to api\.open-meteo\.com \(bearer\)/)).toBeInTheDocument()
    expect(row.getByText(/Your Open-Meteo API key/)).toBeInTheDocument()
    expect(row.getByText(/not granted/)).toBeInTheDocument()
    expect(row.queryByRole("button", { name: "Remove API key" })).not.toBeInTheDocument()
    // No input until the user asks for one.
    expect(screen.getByRole("dialog").querySelector("input[type=password]")).toBeNull()
  })

  it("sends a secret once, clears it at once and keeps it nowhere", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem")
    const logs = (["log", "info", "warn", "error", "debug"] as const).map((m) =>
      vi.spyOn(console, m),
    )
    const { user, client } = renderMarketplace()
    // Holds the answer so the moment after the submit can be inspected.
    let answer: (slots: PluginCredentialSlot[]) => void = () => undefined
    const real = client.setCredential.getMockImplementation()
    client.setCredential.mockImplementationOnce(
      (id, key, secret) =>
        new Promise((resolve) => {
          answer = resolve
          void real?.(id, key, secret)
        }),
    )
    const dialog = await openConfigure(user)
    const row = within(dialog.getByRole("listitem", { name: "API key" }))
    await user.click(row.getByRole("button", { name: "Set API key" }))
    const input = row.getByLabelText("API key for Hello Weather")
    expect(input).toHaveAttribute("type", "password")
    expect(input).toHaveAttribute("autocomplete", "off")
    expect(input).toHaveFocus()
    await user.type(input, SECRET)
    await user.click(row.getByRole("button", { name: "Store" }))

    // Sent once, as typed; the input is empty while the agent answers.
    expect(client.setCredential).toHaveBeenCalledTimes(1)
    expect(client.setCredential).toHaveBeenCalledWith(HELLO, "apiKey", SECRET)
    expect(input).toHaveValue("")
    answer(await client.listCredentials(HELLO))

    expect(await row.findByText("Set")).toBeInTheDocument()
    expect(row.queryByLabelText("API key for Hello Weather")).not.toBeInTheDocument()
    expect(toast.success).toHaveBeenCalledWith("API key stored")
    expect(client.stored(HELLO).credentials).toEqual(["apiKey"])
    // Nowhere afterwards: not in the page, not in browser storage, not in a log.
    expect(document.body.innerHTML).not.toContain(SECRET)
    expect(screen.queryByDisplayValue(SECRET)).not.toBeInTheDocument()
    expect(setItem).not.toHaveBeenCalled()
    expect(JSON.stringify(window.localStorage)).not.toContain(SECRET)
    expect(JSON.stringify(window.sessionStorage)).not.toContain(SECRET)
    for (const log of logs) {
      expect(JSON.stringify(log.mock.calls)).not.toContain(SECRET)
    }
    // Only set or not set comes back; nothing to show a value with.
    expect(row.getByRole("button", { name: "Replace API key" })).toBeInTheDocument()
    expect(screen.getByRole("dialog").querySelector("input[type=password]")).toBeNull()
  })

  it("clears the input when the agent refuses the secret, and says why", async () => {
    const { user, client } = renderMarketplace()
    client.setCredential.mockRejectedValueOnce(
      new PluginError("invalidData", "a secret must be between 1 and 16384 bytes"),
    )
    const dialog = await openConfigure(user)
    const row = within(dialog.getByRole("listitem", { name: "API key" }))
    await user.click(row.getByRole("button", { name: "Set API key" }))
    const input = row.getByLabelText("API key for Hello Weather")
    await user.type(input, SECRET)
    await user.click(row.getByRole("button", { name: "Store" }))
    expect(await row.findByRole("alert")).toHaveTextContent(
      "The values do not fit what the plugin declares: a secret must be between 1 and 16384 bytes",
    )
    expect(input).toHaveValue("")
    expect(row.getByText("Not set")).toBeInTheDocument()
    expect(document.body.innerHTML).not.toContain(SECRET)
  })

  it("does not send an empty secret", async () => {
    const { user, client } = renderMarketplace()
    const dialog = await openConfigure(user)
    const row = within(dialog.getByRole("listitem", { name: "API key" }))
    await user.click(row.getByRole("button", { name: "Set API key" }))
    await user.click(row.getByRole("button", { name: "Store" }))
    expect(row.getByRole("alert")).toHaveTextContent("Enter the secret.")
    expect(client.setCredential).not.toHaveBeenCalled()
  })

  it("cancelling clears what was typed", async () => {
    const { user, client } = renderMarketplace()
    const dialog = await openConfigure(user)
    const row = within(dialog.getByRole("listitem", { name: "API key" }))
    await user.click(row.getByRole("button", { name: "Set API key" }))
    const input = row.getByLabelText("API key for Hello Weather")
    await user.type(input, SECRET)
    await user.click(row.getByRole("button", { name: "Cancel" }))
    expect(input).toHaveValue("")
    expect(row.queryByLabelText("API key for Hello Weather")).not.toBeInTheDocument()
    expect(client.setCredential).not.toHaveBeenCalled()
    expect(document.body.innerHTML).not.toContain(SECRET)
  })

  it("removes a stored secret after confirmation", async () => {
    const { user, client } = renderMarketplace({ helloCredentials: ["apiKey"] })
    const dialog = await openConfigure(user)
    const row = within(dialog.getByRole("listitem", { name: "API key" }))
    expect(row.getByText("Set")).toBeInTheDocument()
    expect(row.getByText(/Stored /)).toBeInTheDocument()
    await user.click(row.getByRole("button", { name: "Remove API key" }))
    const confirm = within(row.getByRole("group", { name: "Remove API key?" }))
    await user.click(confirm.getByRole("button", { name: "Remove" }))
    expect(await row.findByText("Not set")).toBeInTheDocument()
    expect(client.deleteCredential).toHaveBeenCalledWith(HELLO, "apiKey")
    expect(client.stored(HELLO).credentials).toEqual([])
    expect(toast.success).toHaveBeenCalledWith("API key removed")
  })

  it("keeps a secret when removal is not confirmed", async () => {
    const { user, client } = renderMarketplace({ helloCredentials: ["apiKey"] })
    const dialog = await openConfigure(user)
    const row = within(dialog.getByRole("listitem", { name: "API key" }))
    await user.click(row.getByRole("button", { name: "Remove API key" }))
    await user.click(row.getByRole("button", { name: "Keep it" }))
    expect(client.deleteCredential).not.toHaveBeenCalled()
    expect(row.getByText("Set")).toBeInTheDocument()
  })
})

describe("uninstall retention", () => {
  async function openUninstall(user: User) {
    await openInstalled(user)
    await user.click(installedItem("Hello Weather").getByRole("button", { name: "Uninstall" }))
    return within(screen.getByRole("alertdialog", { name: "Uninstall Hello Weather?" }))
  }

  it("deletes settings and credentials by default", async () => {
    const { user, client } = renderMarketplace({
      helloSettings: { units: "fahrenheit" },
      helloCredentials: ["apiKey"],
    })
    const confirm = await openUninstall(user)
    expect(confirm.getByRole("checkbox", { name: "Keep its settings" })).not.toBeChecked()
    expect(confirm.getByRole("checkbox", { name: "Keep its credentials" })).not.toBeChecked()
    await user.click(confirm.getByRole("button", { name: "Uninstall" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather uninstalled")
    })
    // The original request form: no keep at all.
    expect(client.uninstallPlugin).toHaveBeenCalledWith(HELLO)
    expect(client.stored(HELLO)).toEqual({ settings: {}, credentials: [] })
  })

  it("keeps what the user chose for a reinstall", async () => {
    const { user, client } = renderMarketplace({
      helloSettings: { units: "fahrenheit" },
      helloCredentials: ["apiKey"],
    })
    const confirm = await openUninstall(user)
    await user.click(confirm.getByRole("checkbox", { name: "Keep its credentials" }))
    await user.click(confirm.getByRole("button", { name: "Uninstall" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith(
        "Hello Weather uninstalled; its credentials are kept for a reinstall",
      )
    })
    expect(client.uninstallPlugin).toHaveBeenCalledWith(HELLO, { credentials: true })
    expect(client.stored(HELLO)).toEqual({ settings: {}, credentials: ["apiKey"] })
  })

  it("starts from the default each time it opens", async () => {
    const { user } = renderMarketplace()
    let confirm = await openUninstall(user)
    await user.click(confirm.getByRole("checkbox", { name: "Keep its settings" }))
    await user.click(confirm.getByRole("button", { name: "Cancel" }))
    await user.click(installedItem("Hello Weather").getByRole("button", { name: "Uninstall" }))
    confirm = within(screen.getByRole("alertdialog", { name: "Uninstall Hello Weather?" }))
    expect(confirm.getByRole("checkbox", { name: "Keep its settings" })).not.toBeChecked()
  })
})

describe("plugin details", () => {
  it("shows storage usage and quota when the agent sends them", async () => {
    const { user } = renderMarketplace({
      helloStorage: { usedBytes: 1536, keys: 3, quotaBytes: 262_144, maxKeys: 1000 },
    })
    await openInstalled(user)
    expect(
      installedItem("Hello Weather").getByText("Storage: 1.5 KiB of 256 KiB used (3 of 1000 keys)"),
    ).toBeInTheDocument()
    await user.click(installedItem("Hello Weather").getByRole("button", { name: "Details" }))
    const storage = await screen.findByRole("region", { name: "Storage" })
    expect(storage).toHaveTextContent("1.5 KiB of 256 KiB used (3 of 1000 keys)")
  })

  it("shows nothing about storage for an agent without the field", async () => {
    const { user } = renderMarketplace()
    await openInstalled(user)
    expect(installedItem("Hello Weather").queryByText(/Storage:/)).not.toBeInTheDocument()
  })

  it("names a plugin installed here before with what plugin_known has", async () => {
    const { user } = renderMarketplace({
      known: [
        {
          id: LIGHTS,
          name: "Home Lights",
          publisher: "knit",
          namespace: "plugins.knit.homeLights",
          lastVersion: "0.2.0",
          installed: false,
          firstSeenAt: 1_764_547_200,
          lastSeenAt: 1_764_547_200,
        },
      ],
    })
    await user.click(await screen.findByRole("button", { name: /Home Lights/ }))
    expect(
      await screen.findByText(/Installed here before as Home Lights, last version 0\.2\.0/),
    ).toBeInTheDocument()
  })
})

describe("classification", () => {
  it("tells core functionality, installed optional and available plugins apart", async () => {
    renderMarketplace()
    const overview = await screen.findByRole("region", { name: "What DeckPilot includes" })
    expect(overview).toHaveTextContent(
      "Core functionalityBuilt into DeckPilot, not plugins: Docker, General, Git, HTTP, Notifications, System.",
    )
    // No system plugin ships with DeckPilot yet.
    expect(overview).not.toHaveTextContent("Bundled system plugins")
    expect(overview).toHaveTextContent("Installed optional plugins1 installed by you")
    expect(overview).toHaveTextContent("Available plugins3 in the registry, not installed.")
    const cards = within(screen.getByRole("list", { name: "Plugins" })).getAllByRole("listitem")
    const hello = cards.find((c) => c.textContent.includes("Hello Weather"))
    const lights = cards.find((c) => c.textContent.includes("Home Lights"))
    expect(hello).toHaveTextContent("Optional plugin")
    expect(lights).toHaveTextContent("Available plugin")
  })
})

describe("form helpers", () => {
  const settings: PluginSettings = {
    plugin: HELLO,
    fields: [
      {
        key: "n",
        label: "N",
        kind: { type: "integer", min: 1 },
        required: false,
        default: 3,
        portability: "portable",
      },
      { key: "p", label: "P", kind: { type: "project" }, required: false, portability: "portable" },
    ],
    values: { n: 4 },
    effective: { n: 4 },
  }

  it("sends changes, resets and nothing for kinds it cannot edit", () => {
    expect(settingsChanges(settings, { n: "4", p: null })).toEqual({ values: {}, errors: {} })
    expect(settingsChanges(settings, { n: "", p: null }).values).toEqual({ n: null })
    expect(settingsChanges(settings, { n: null, p: "x" }).values).toEqual({ n: null })
    expect(settingsChanges(settings, { n: "1.5", p: null }).errors).toEqual({
      n: "Enter a whole number.",
    })
  })

  it("puts each problem of a refusal under its field", () => {
    expect(
      settingsProblems(
        "the settings of acme.x do not match their declaration: parameter `n`: 0 is out of range; unknown parameter `zz`; `p` expects text",
        settings.fields,
      ),
    ).toEqual({
      fields: { n: "0 is out of range", p: "Expects text" },
      form: ["unknown parameter `zz`"],
    })
    expect(settingsProblems("the store is locked", settings.fields)).toEqual({
      fields: {},
      form: ["the store is locked"],
    })
  })

  it("checks a secret without repeating it", () => {
    expect(secretProblem("")).toBe("Enter the secret.")
    expect(secretProblem("a\nb")).toBe("A secret cannot contain line breaks.")
    expect(secretProblem("x".repeat(16 * 1024 + 1))).toBe("The secret is longer than 16 KiB.")
    expect(secretProblem(SECRET)).toBeNull()
  })
})
