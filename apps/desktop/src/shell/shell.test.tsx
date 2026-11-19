// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// The main window as a whole: the toolbar, the rail, the device editor as
// the default view, and the settings in their own window.

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

  it("opens the settings window from the toolbar, the keyboard and other windows", async () => {
    const { user } = await renderShell()
    await user.click(screen.getByRole("button", { name: "Settings" }))
    expect(api.openWindow).toHaveBeenLastCalledWith("app.settings", undefined)

    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", ctrlKey: true }))
      window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", metaKey: true }))
    })
    expect(api.openWindow).toHaveBeenCalledTimes(2)

    // "Manage profiles…" leads to Settings › Profiles.
    await user.click(screen.getByRole("button", { name: /^Profile:/ }))
    await user.click(screen.getByRole("menuitem", { name: "Manage profiles…" }))
    expect(api.openWindow).toHaveBeenLastCalledWith("app.settings", "profiles")

    // Another window asks for a settings page.
    act(() => {
      emitBackend(NAVIGATE_EVENT, "settings:security")
    })
    expect(api.openWindow).toHaveBeenLastCalledWith("app.settings", "security")
    // The editor stays open meanwhile.
    expect(screen.getByRole("heading", { name: "Device Editor" })).toBeInTheDocument()
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
