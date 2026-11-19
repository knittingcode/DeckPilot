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
import type { PluginNavigationEntry } from "@/plugins/types"
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
// Plugin views reuse the app's pages; what they show is tested elsewhere.
vi.mock("@/pages/variables-table", () => ({
  VariablesTable: ({ namespace }: { namespace?: string }) => <p>variables under {namespace}</p>,
}))

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
  api.listSharedLayouts.mockResolvedValue([])
  api.profileLayouts.mockResolvedValue([])
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
    // One navigation: the toolbar stays, the gear shows where we are, the
    // rail leads back; no back, forward or "Back to DeckPilot" anywhere.
    expect(screen.getByRole("banner", { name: "Toolbar" })).toBe(toolbar)
    expect(gearOf(toolbar)).toHaveAttribute("aria-pressed", "true")
    for (const name of ["Back to DeckPilot", "Back", "Forward"]) {
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument()
    }
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

  it("returns to the editor from the settings, the Marketplace and other pages, as it was", async () => {
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

    // The rail's Device Editor returns, with the device, profile and the
    // unsaved change as they were.
    await user.click(screen.getByRole("button", { name: "Device Editor" }))
    expect(await screen.findByRole("heading", { name: "Device Editor" })).toBeInTheDocument()
    expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Work Plus edited")
    expect(screen.getByRole("button", { name: /^Device: Stream Deck \+/ })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /^Profile: Work/ })).toBeInTheDocument()
    expect(screen.queryByRole("generic", { name: "Settings" })).not.toBeInTheDocument()
    expect(gearOf(toolbar)).toHaveAttribute("aria-pressed", "false")

    // From another workspace page, too.
    await user.click(screen.getByRole("button", { name: "Workflows" }))
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Workflows" })).toHaveAttribute(
        "aria-current",
        "page",
      )
    })
    await user.click(screen.getByRole("button", { name: "Device Editor" }))
    expect(await screen.findByRole("heading", { name: "Device Editor" })).toBeInTheDocument()

    // The gear reopens the section shown last.
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

  it("is used with the keyboard alone: shortcut, sections, and the rail back to the editor", async () => {
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
    // The rail's Device Editor is a focusable button too.
    screen.getByRole("button", { name: "Device Editor" }).focus()
    await user.keyboard("{Enter}")
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

  it("keeps the workspace pages in a rail that is always collapsed to icons", async () => {
    localStorage.setItem("deckpilot.rail", "expanded") // from an earlier version
    const { user } = await renderShell()
    const rail = screen.getByRole("generic", { name: "Workspace" }).closest("[data-state]")
    expect(rail).toHaveAttribute("data-state", "collapsed")
    expect(rail).toHaveAttribute("data-collapsible", "icon")
    // The earlier version's choice is neither honoured nor kept.
    expect(localStorage.getItem("deckpilot.rail")).toBeNull()
    // No toggle, and ⌘B / Ctrl+B do nothing.
    expect(screen.queryByRole("button", { name: /Toggle Sidebar/i })).not.toBeInTheDocument()
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "b", metaKey: true }))
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "b", ctrlKey: true }))
    })
    expect(rail).toHaveAttribute("data-state", "collapsed")

    // Every page has its name as label (and tooltip); the current one is marked.
    for (const name of ["Device Editor", "Dashboard", "Action Library", "Workflows"]) {
      expect(screen.getAllByRole("button", { name }).length).toBeGreaterThan(0)
    }
    const editor = screen.getByRole("button", { name: "Device Editor" })
    expect(editor).toHaveAttribute("aria-current", "page")
    expect(editor).toHaveAttribute("data-active", "true")
    await user.hover(editor)
    expect(await screen.findByRole("tooltip", { name: "Device Editor" })).toBeInTheDocument()

    // The same rail beside the settings, with nothing marked.
    await user.click(gearOf(screen.getByRole("banner", { name: "Toolbar" })))
    await screen.findByRole("generic", { name: "Settings" })
    expect(rail).toHaveAttribute("data-state", "collapsed")
    expect(screen.getByRole("button", { name: "Device Editor" })).not.toHaveAttribute(
      "aria-current",
    )
    // Keyboard: the rail's buttons are reachable and work with Enter.
    screen.getByRole("button", { name: "Workflows" }).focus()
    await user.keyboard("{Enter}")
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Workflows" })).toHaveAttribute(
        "aria-current",
        "page",
      )
    })
    expect(rail).toHaveAttribute("data-state", "collapsed")
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

/** The toolbar's selectors, by their accessible names; empty when hidden. */
function selectorsOf(toolbar: HTMLElement): string[] {
  const leading = toolbar.querySelector<HTMLElement>("[data-slot='titlebar-leading']")
  return leading
    ? within(leading)
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label") ?? "")
    : []
}

describe("context-aware toolbar", { timeout: 20_000 }, () => {
  it("has no Device Editor title in the toolbar, and keeps its commands", async () => {
    const { toolbar } = await renderShell()
    expect(within(toolbar).queryByRole("heading")).not.toBeInTheDocument()
    expect(within(toolbar).queryByText("Device Editor")).not.toBeInTheDocument()
    expect(within(toolbar).queryByText(/Assign actions to the controls/)).not.toBeInTheDocument()
    expect(toolbar).toHaveAttribute("data-context", "device+profile")
    // The order: room for the window controls, the selectors, the page's
    // (empty) context, the app's commands.
    const slots = [...toolbar.children].map(
      (child) => child.getAttribute("data-slot") ?? child.getAttribute("role") ?? "context",
    )
    expect(slots).toEqual([
      "titlebar-controls-inset",
      "titlebar-leading",
      "context",
      "titlebar-trailing",
    ])
    const app = within(toolbar).getByRole("group", { name: "App" })
    expect(
      within(app)
        .getAllByRole("button")
        .map((b) => b.getAttribute("aria-label")),
    ).toEqual(["Notifications", "Settings"])
    // The page still has a heading for assistive technology.
    expect(screen.getByRole("heading", { name: "Device Editor" })).toHaveClass("sr-only")
  })

  it("shows the device and profile selectors in the editor, not in the settings", async () => {
    const { user, toolbar } = await renderShell()
    expect(selectorsOf(toolbar)).toEqual([
      "Device: Stream Deck +, connected",
      "Profile: Work, active on the device",
    ])
    await user.click(gearOf(toolbar))
    const nav = await screen.findByRole("generic", { name: "Settings" })
    await user.click(within(nav).getByRole("button", { name: "Profiles" }))
    await screen.findByText("All profiles", undefined, { timeout: 5000 })
    // No selectors and no status of them; no empty room where they were.
    expect(selectorsOf(toolbar)).toEqual([])
    expect(toolbar.querySelector("[data-slot='titlebar-leading']")).toBeNull()
    expect(within(toolbar).queryByRole("button", { name: /^Device:/ })).toBeNull()
    expect(within(toolbar).queryByRole("button", { name: /^Profile:/ })).toBeNull()
    expect(toolbar).toHaveAttribute("data-context", "none")
    // The location starts right after the room for the window controls.
    const inset = toolbar.querySelector("[data-slot='titlebar-controls-inset']")
    const location = within(toolbar).getByRole("list", { name: "Location" })
    expect(inset?.nextElementSibling).toContainElement(location)
    expect(within(location).getByRole("button", { name: "Settings" })).toBeInTheDocument()
    // The page's title is on the page, not again in the toolbar.
    expect(within(toolbar).queryByRole("heading")).not.toBeInTheDocument()
    expect(within(toolbar).queryByText("Profiles")).toBeNull()
    // Notifications, settings and the rail stay.
    expect(within(toolbar).getByRole("button", { name: "Notifications" })).toBeInTheDocument()
    expect(gearOf(toolbar)).toHaveAttribute("aria-pressed", "true")
    expect(screen.getByRole("button", { name: "Device Editor" })).toBeInTheDocument()
    // Back in the editor: the same device and profile, chosen again by nobody.
    await user.click(screen.getByRole("button", { name: "Device Editor" }))
    await waitFor(() => {
      expect(selectorsOf(toolbar)).toEqual([
        "Device: Stream Deck +, connected",
        "Profile: Work, active on the device",
      ])
    })
  })

  it("shows only the profile for the profile's pages, nothing on the dashboard", async () => {
    const { user, toolbar } = await renderShell()
    await user.click(screen.getByRole("button", { name: "Workflows" }))
    await waitFor(() => {
      expect(toolbar).toHaveAttribute("data-context", "profile")
    })
    expect(selectorsOf(toolbar)).toEqual(["Profile: Work, active on the device"])
    await user.click(screen.getByRole("button", { name: "Dashboard" }))
    await waitFor(() => {
      expect(toolbar).toHaveAttribute("data-context", "none")
    })
    expect(selectorsOf(toolbar)).toEqual([])
  })

  it("keeps the device, the profile, the layout's page and the selected key across views", async () => {
    const { user, toolbar } = await renderShell()
    await user.click(screen.getByRole("button", { name: /^Device:/ }))
    await user.click(screen.getByRole("menuitem", { name: /Stream Deck MK\.2/ }))
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Work MK.2")
    })
    await user.click(screen.getByRole("button", { name: "Key 3" }))
    expect(screen.getByRole("button", { name: "Key 3" })).toHaveAttribute("aria-pressed", "true")
    const agentCalls = api.agentStatus.mock.calls.length

    // Settings, then another workspace page, then back.
    await user.click(gearOf(toolbar))
    await screen.findByRole("generic", { name: "Settings" })
    await user.click(screen.getByRole("button", { name: "Workflows" }))
    await waitFor(() => {
      expect(toolbar).toHaveAttribute("data-context", "profile")
    })
    await user.click(screen.getByRole("button", { name: "Device Editor" }))
    await screen.findByRole("group", { name: /controls$/ })
    expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Work MK.2")
    expect(screen.getByRole("button", { name: "Key 3" })).toHaveAttribute("aria-pressed", "true")
    expect(selectorsOf(toolbar)).toEqual([
      "Device: Stream Deck MK.2, not connected",
      "Profile: Work, active on the device",
    ])
    // Hiding the selectors connected nothing again.
    expect(api.agentStatus.mock.calls.length).toBeLessThanOrEqual(agentCalls + 1)
  })
})

describe("main window with plugins", { timeout: 20_000 }, () => {
  beforeEach(() => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  })
  afterEach(() => {
    vi.unstubAllEnvs()
  })

  /** The app with the plugin flag, a fake plugin client and `navigation` entries. */
  async function renderWithPlugins(navigation: PluginNavigationEntry[] = []) {
    vi.resetModules()
    const { App: WithPlugins } = await import("@/App")
    const { PluginClientContext } = await import("@/plugins/api")
    const { createFakePluginClient, HELLO } = await import("@/test/plugins")
    const client = createFakePluginClient({
      helloInstalled: true,
      navigation: { [HELLO]: navigation },
    })
    const user = userEvent.setup()
    render(
      <PluginClientContext value={client}>
        <WithPlugins />
      </PluginClientContext>,
    )
    const toolbar = await screen.findByRole("banner", { name: "Toolbar" })
    await screen.findByRole("group", { name: /controls$/ })
    return { user, toolbar, client, HELLO }
  }

  const view = (id: string, context?: string): PluginNavigationEntry => ({
    id,
    label: `View ${id}`,
    view: { id: `${id}-view`, kind: "variables", ...(context === undefined ? {} : { context }) },
  })

  it("hides the selectors in the Marketplace and keeps the editor's selection", async () => {
    const { user, toolbar } = await renderWithPlugins()
    await user.click(screen.getByRole("button", { name: "Key 2" }))
    await user.click(within(toolbar).getByRole("button", { name: "Marketplace" }))
    await waitFor(() => {
      expect(toolbar).toHaveAttribute("data-context", "none")
    })
    expect(selectorsOf(toolbar)).toEqual([])
    expect(within(toolbar).getByRole("button", { name: "Marketplace" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    // Settings, then the editor again: as it was.
    await user.click(gearOf(toolbar))
    await screen.findByRole("generic", { name: "Settings" })
    expect(selectorsOf(toolbar)).toEqual([])
    await user.click(screen.getByRole("button", { name: "Device Editor" }))
    await screen.findByRole("group", { name: /controls$/ })
    expect(selectorsOf(toolbar)).toEqual([
      "Device: Stream Deck +, connected",
      "Profile: Work, active on the device",
    ])
    expect(screen.getByRole("textbox", { name: "Layout name" })).toHaveValue("Work Plus")
    expect(screen.getByRole("button", { name: "Key 2" })).toHaveAttribute("aria-pressed", "true")
  })

  it("shows what each plugin view declares, and nothing for none, absent or unknown", async () => {
    const { user, toolbar, client } = await renderWithPlugins([
      view("plain"),
      view("device", "device"),
      view("profile", "profile"),
      view("both", "device+profile"),
    ])
    const open = async (id: string) => {
      await user.click(await within(toolbar).findByRole("button", { name: `View ${id}` }))
      await screen.findByText(/variables under/)
    }
    await open("plain")
    expect(selectorsOf(toolbar)).toEqual([])
    await open("device")
    expect(selectorsOf(toolbar)).toEqual(["Device: Stream Deck +, connected"])
    await open("profile")
    expect(selectorsOf(toolbar)).toEqual(["Profile: Work, active on the device"])
    await open("both")
    expect(selectorsOf(toolbar)).toEqual([
      "Device: Stream Deck +, connected",
      "Profile: Work, active on the device",
    ])
    // Showing the selectors asked the plugin for nothing and gave it nothing:
    // the client only listed the entries and their icons.
    const asked = Object.entries(client)
      .filter(([, fn]) => vi.isMockFunction(fn) && fn.mock.calls.length > 0)
      .map(([name]) => name)
    expect(asked.filter((name) => !["listNavigation", "getNavigationIcon"].includes(name))).toEqual(
      expect.not.arrayContaining(["invoke", "runAction", "setSettings"]),
    )
  })

  it("treats a malformed context as none", async () => {
    const { user, toolbar } = await renderWithPlugins([view("odd", "everything")])
    await user.click(await within(toolbar).findByRole("button", { name: "View odd" }))
    await screen.findByText(/variables under/)
    expect(toolbar).toHaveAttribute("data-context", "none")
    expect(selectorsOf(toolbar)).toEqual([])
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
