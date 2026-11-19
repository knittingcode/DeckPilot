// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// The main window as a whole: the toolbar, the rail, the device editor as
// the default view, and the settings as a view of the same window.

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { App } from "@/App"
import { NAVIGATE_EVENT } from "@/lib/api"
import type { DeviceLayout, ProfileView, SecurityView } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { agentConnection, agentStatus, deviceInfo, deviceModel, profiles } from "@/test/fixtures"
import { emitBackend, resetBackend } from "@/test/tauri"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => {
  const mock = (await import("@/test/api-mock")).apiModule
  const real = await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...real, ...mock }
})
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const unlocked = {
  database: "encrypted",
  protection: "systemManaged",
  unlocked: true,
  confirmationRequired: false,
  lockReason: null,
  waitingForKeyStore: false,
  databaseMissing: false,
  screenLockDetection: true,
  problem: null,
  keyStore: "macOS Keychain",
  verification: "none",
  policy: null,
  agentRunning: true,
  appVersion: "0.7.0",
  legacyCopies: [],
  backupDir: "/data/backups",
} as unknown as SecurityView

const mk2 = deviceModel()
const plus = deviceModel({
  id: "elgato.streamdeck.plus",
  name: "Stream Deck +",
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: true },
    touch: { zones: 4, image: null, gestures: ["tap"] },
  },
})

function layoutOf(id: string, model: string, name: string): DeviceLayout {
  return { id, name, model, homePage: "home", pages: [{ id: "home", name: "Home", bindings: [] }] }
}

function profileView(id: string, name: string): ProfileView {
  return {
    activeProjectId: null,
    issues: [],
    profile: {
      id,
      name,
      description: "",
      actions: [],
      workflows: [],
      projects: [],
      layouts: [
        layoutOf(`${id}-mk2`, mk2.id, `${name} MK.2`),
        layoutOf(`${id}-plus`, plus.id, `${name} Plus`),
      ],
      activeLayouts: {},
    },
  } as unknown as ProfileView
}

beforeAll(installDomStubs)

beforeEach(() => {
  resetApi()
  resetBackend()
  resetToasts()
  localStorage.clear()
  api.securityStatus.mockResolvedValue(unlocked)
  api.listProfiles.mockResolvedValue(
    profiles([
      { id: "work", name: "Work" },
      { id: "home", name: "Home" },
    ]),
  )
  api.listModules.mockResolvedValue([])
  api.agentStatus.mockResolvedValue(
    agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ id: { model: plus.id, serial: "PLUS-1" }, modelName: plus.name })],
      }),
    }),
  )
  api.updateState.mockRejectedValue(new Error("no updater"))
  api.listDeviceModels.mockResolvedValue([mk2, plus])
  api.getProfile.mockImplementation((id: string) =>
    Promise.resolve(profileView(id, id === "work" ? "Work" : "Home")),
  )
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.pageTree.mockResolvedValue({ pages: [], warnings: [] })
  api.listActionTypes.mockResolvedValue([])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.listExecutions.mockResolvedValue([])
  api.openWindow.mockResolvedValue(null)
  api.showMainWindow.mockResolvedValue(true)
})

afterEach(cleanup)

/** The toolbar's settings button (the settings' breadcrumb is also a "Settings" button). */
function gearOf(toolbar: HTMLElement) {
  return within(within(toolbar).getByRole("group", { name: "App" })).getByRole("button", {
    name: "Settings",
  })
}

async function renderShell() {
  const user = userEvent.setup()
  render(<App />)
  const toolbar = await screen.findByRole("banner", { name: "Toolbar" })
  // The device editor is the default view.
  await screen.findByRole("group", { name: /controls$/ })
  return { user, toolbar }
}

describe("main window shell", { timeout: 20_000 }, () => {
  it("opens with the device editor, the library at the right and the inspector below", async () => {
    await renderShell()
    expect(screen.getByRole("heading", { name: "Device Editor" })).toBeInTheDocument()
    const library = screen.getByRole("complementary", { name: "Action Library" })
    expect(within(library).getByLabelText("Search the library")).toBeInTheDocument()
    const main = document.querySelector("[data-slot='device-editor-main']")
    const canvas = screen.getByRole("group", { name: /controls$/ })
    expect(main).toContainElement(canvas)
    // The inspector follows the device in the editor's main column.
    const inspector = within(main as HTMLElement).getByText("Control")
    expect(canvas.compareDocumentPosition(inspector) & Node.DOCUMENT_POSITION_FOLLOWING).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    )
    // The library is beside the main column, after it: on the right.
    expect(main?.nextElementSibling).toBe(library)
  })

  it("orders the toolbar: selectors, page, then notifications and settings", async () => {
    const { toolbar } = await renderShell()
    const leading = toolbar.querySelector("[data-slot='titlebar-leading']") as HTMLElement
    const buttons = within(leading).getAllByRole("button")
    expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual([
      "Device: Stream Deck +, connected",
      "Profile: Work, active on the device",
    ])
    const app = within(toolbar).getByRole("group", { name: "App" })
    expect(
      within(app)
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label")),
    ).toEqual(["Notifications", "Settings"])
    // No plugin entries: no plugin navigation and no separator.
    expect(within(toolbar).queryByRole("navigation", { name: "Plugins" })).not.toBeInTheDocument()
    expect(toolbar.querySelector("[data-slot='plugin-navigation-separator']")).toBeNull()
    // The trailing group is the toolbar's last.
    expect(toolbar.lastElementChild).toContainElement(app)
  })

  it("chooses the device for the editor: connected ones and models not connected", async () => {
    const { user } = await renderShell()
    expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Work Plus")
    await user.click(screen.getByRole("button", { name: /^Device:/ }))
    const menu = screen.getByRole("menu")
    expect(within(menu).getByText("Connected · PLUS-1")).toBeInTheDocument()
    const mk2Item = within(menu).getByRole("menuitem", { name: /Stream Deck MK\.2/ })
    expect(mk2Item).toHaveTextContent("Not connected")
    await user.click(mk2Item)
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Work MK.2")
    })
    expect(
      screen.getByRole("button", { name: "Device: Stream Deck MK.2, not connected" }),
    ).toBeInTheDocument()
    expect(localStorage.getItem("deckpilot.editor.model")).toBe(mk2.id)
  })

  it("switches the profile from the toolbar, but not over unsaved changes", async () => {
    const { user } = await renderShell()
    await user.click(screen.getByRole("button", { name: /^Profile:/ }))
    await user.click(screen.getByRole("menuitem", { name: /Home/ }))
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Home Plus")
    })
    expect(localStorage.getItem("deckpilot.editingProfile")).toBe("home")

    // An unsaved change keeps the profile.
    const name = screen.getByRole("textbox", { name: "Layout name" })
    await user.type(name, " edited")
    await user.click(screen.getByRole("button", { name: /^Profile:/ }))
    await user.click(screen.getByRole("menuitem", { name: /Work/ }))
    expect(toast.warning).toHaveBeenCalledWith("Save or revert the current layout first")
    expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Home Plus edited")
    // And the device likewise.
    await user.click(screen.getByRole("button", { name: /^Device:/ }))
    await user.click(screen.getByRole("menuitem", { name: /Stream Deck MK\.2/ }))
    expect(toast.warning).toHaveBeenCalledTimes(2)
    expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Home Plus edited")
  })

  it("opens the settings inside the main window, never in a window of their own", async () => {
    const { user, toolbar } = await renderShell()
    const gear = gearOf(toolbar)
    expect(gear).toHaveAttribute("aria-pressed", "false")
    await user.click(gear)

    const nav = await screen.findByRole("generic", { name: "Settings" })
    expect(nav).toHaveAttribute("data-slot", "sidebar")
    for (const name of ["Appearance", "Profiles", "Updates", "Devices", "Background Agent"]) {
      expect(within(nav).getByRole("button", { name })).toBeInTheDocument()
    }
    expect(within(nav).getByRole("button", { name: "Appearance" })).toHaveAttribute(
      "aria-current",
      "page",
    )
    expect(
      await screen.findByText(/System follows the light or dark/, undefined, { timeout: 5000 }),
    ).toBeInTheDocument()
    // One navigation: the toolbar stays, the gear shows where we are, and the
    // settings have no back or "Back to DeckPilot" of their own.
    expect(screen.getByRole("banner", { name: "Toolbar" })).toBe(toolbar)
    expect(gearOf(toolbar)).toHaveAttribute("aria-pressed", "true")
    expect(screen.queryByRole("button", { name: "Back to DeckPilot" })).not.toBeInTheDocument()
    expect(screen.getAllByRole("button", { name: "Back" })).toHaveLength(1)
    // The device editor is not on screen (it waits, hidden).
    expect(screen.queryByRole("heading", { name: "Device Editor" })).not.toBeInTheDocument()
    // No window was asked for, not even once.
    expect(api.openWindow).not.toHaveBeenCalled()

    // Opening them again changes nothing.
    await user.click(gearOf(toolbar))
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", ctrlKey: true }))
    })
    expect(screen.getAllByRole("generic", { name: "Settings" })).toHaveLength(1)
    expect(api.openWindow).not.toHaveBeenCalled()
  })

  it("goes back and forth between the editor and settings sections, keeping the editor as it was", async () => {
    const { user, toolbar } = await renderShell()
    // An unsaved change in the editor: proof that it is the same editor later.
    const name = screen.getByRole("textbox", { name: "Layout name" })
    await user.type(name, " edited")

    await user.click(gearOf(toolbar))
    const nav = await screen.findByRole("generic", { name: "Settings" })
    await user.click(within(nav).getByRole("button", { name: "Profiles" }))
    expect(
      await screen.findByText("All profiles", undefined, { timeout: 5000 }),
    ).toBeInTheDocument()
    expect(localStorage.getItem("deckpilot.settings.page")).toBe("profiles")

    // Back: the previous settings section; back again: the editor, as it was.
    await user.click(within(toolbar).getByRole("button", { name: "Back" }))
    expect(
      await screen.findByText(/System follows the light or dark/, undefined, { timeout: 5000 }),
    ).toBeInTheDocument()
    await user.click(within(toolbar).getByRole("button", { name: "Back" }))
    expect(await screen.findByRole("heading", { name: "Device Editor" })).toBeInTheDocument()
    expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Work Plus edited")
    expect(screen.queryByRole("generic", { name: "Settings" })).not.toBeInTheDocument()
    expect(gearOf(toolbar)).toHaveAttribute("aria-pressed", "false")

    // Forward: the settings again, at the section shown there.
    await user.click(within(toolbar).getByRole("button", { name: "Forward" }))
    expect(
      await screen.findByText(/System follows the light or dark/, undefined, { timeout: 5000 }),
    ).toBeInTheDocument()
    await user.click(within(toolbar).getByRole("button", { name: "Forward" }))
    expect(
      await screen.findByText("All profiles", undefined, { timeout: 5000 }),
    ).toBeInTheDocument()

    // The rail leads back to the editor; the gear then reopens the section
    // shown last.
    await user.click(screen.getByRole("button", { name: "Device Editor" }))
    expect(await screen.findByRole("heading", { name: "Device Editor" })).toBeInTheDocument()
    expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Work Plus edited")
    await user.click(gearOf(toolbar))
    const again = await screen.findByRole("generic", { name: "Settings" })
    expect(within(again).getByRole("button", { name: "Profiles" })).toHaveAttribute(
      "aria-current",
      "page",
    )
  })

  it("opens settings sections asked for by menus and other windows, and searches them", async () => {
    const { user } = await renderShell()
    // "Manage profiles…" leads to Settings › Profiles, in this window.
    await user.click(screen.getByRole("button", { name: /^Profile:/ }))
    await user.click(screen.getByRole("menuitem", { name: "Manage profiles…" }))
    expect(
      await screen.findByText("All profiles", undefined, { timeout: 5000 }),
    ).toBeInTheDocument()

    // Another window (or the agent, through the app) asks for a section.
    act(() => {
      emitBackend(NAVIGATE_EVENT, "settings:security")
    })
    const nav = await screen.findByRole("generic", { name: "Settings" })
    await waitFor(() => {
      expect(within(nav).getByRole("button", { name: "Security" })).toHaveAttribute(
        "aria-current",
        "page",
      )
    })
    expect(api.openWindow).not.toHaveBeenCalled()

    // The search narrows the sections, and says when nothing matches.
    await user.type(within(nav).getByRole("searchbox", { name: "Search settings" }), "upd")
    expect(within(nav).getByRole("button", { name: "Updates" })).toBeInTheDocument()
    expect(within(nav).queryByRole("button", { name: "Profiles" })).not.toBeInTheDocument()
    await user.clear(within(nav).getByRole("searchbox", { name: "Search settings" }))
    await user.type(
      within(nav).getByRole("searchbox", { name: "Search settings" }),
      "nothing like this",
    )
    expect(within(nav).getByText("No settings found")).toBeInTheDocument()
  })

  it("is used with the keyboard alone: shortcut, sections, and back to the editor", async () => {
    const { user } = await renderShell()
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", metaKey: true }))
      window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", ctrlKey: true }))
    })
    const nav = await screen.findByRole("generic", { name: "Settings" })
    // Every section is a focusable button, reached with Tab, chosen with Enter.
    const profiles = within(nav).getByRole("button", { name: "Profiles" })
    profiles.focus()
    await user.keyboard("{Enter}")
    expect(
      await screen.findByText("All profiles", undefined, { timeout: 5000 }),
    ).toBeInTheDocument()
    // The history shortcut returns to the previous view.
    // Back twice (Alt+← here; ⌘[ on macOS): Appearance, then the editor.
    for (let i = 0; i < 2; i++) {
      act(() => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true }))
      })
    }
    expect(await screen.findByRole("heading", { name: "Device Editor" })).toBeInTheDocument()
  })

  it("switches the theme from the settings without leaving the window", async () => {
    const { user, toolbar } = await renderShell()
    await user.click(gearOf(toolbar))
    const dark = await screen.findByRole("radio", { name: "Dark" }, { timeout: 5000 })
    await user.click(dark)
    expect(document.documentElement).toHaveClass("dark")
    await user.click(screen.getByRole("radio", { name: "Light" }))
    expect(document.documentElement).not.toHaveClass("dark")
    expect(api.openWindow).not.toHaveBeenCalled()
  })

  it("keeps the other workspace pages in the rail", async () => {
    const { user } = await renderShell()
    const rail = screen.getByRole("button", { name: "Toggle Sidebar" })
    await user.click(rail)
    expect(localStorage.getItem("deckpilot.rail")).toBe("expanded")
    for (const name of ["Device Editor", "Dashboard", "Action Library", "Workflows"]) {
      expect(screen.getAllByRole("button", { name }).length).toBeGreaterThan(0)
    }
    expect(screen.getByRole("button", { name: "Device Editor" })).toHaveAttribute(
      "aria-current",
      "page",
    )
  })

  it("shows notifications from the toolbar with the number of unread ones", async () => {
    const { user, toolbar } = await renderShell()
    act(() => {
      emitBackend("agent-notification", {
        title: "Backup failed",
        severity: "error",
        desktop: false,
        statusBar: true,
      })
    })
    const bell = within(toolbar).getByRole("button", { name: "Notifications, 1 unread" })
    await user.click(bell)
    expect(
      within(screen.getByRole("list", { name: "Notifications" })).getByText("Backup failed"),
    ).toBeInTheDocument()
    await user.keyboard("{Escape}")
    expect(within(toolbar).getByRole("button", { name: "Notifications" })).toBeInTheDocument()
    // Not in the status bar any more.
    const statusBar = screen.getByRole("contentinfo", { name: "Status bar" })
    expect(within(statusBar).queryByRole("button", { name: /Notifications/ })).toBeNull()
  })
})

describe("main window with plugins", { timeout: 20_000 }, () => {
  beforeEach(() => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  })
  afterEach(() => {
    vi.unstubAllEnvs()
  })

  it("has the Marketplace between notifications and settings", async () => {
    // The flag is read when the module loads.
    vi.resetModules()
    const { App: WithPlugins } = await import("@/App")
    render(<WithPlugins />)
    const toolbar = await screen.findByRole("banner", { name: "Toolbar" })
    const app = within(toolbar).getByRole("group", { name: "App" })
    expect(
      within(app)
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label")),
    ).toEqual(["Notifications", "Marketplace", "Settings"])
  })
})
