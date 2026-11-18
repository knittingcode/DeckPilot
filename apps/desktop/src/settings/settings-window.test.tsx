// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// The settings window (`app.settings`): its pages, its own navigation and
// the way it hands pages of the main window over to it.

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { ThemeProvider } from "@/components/theme-provider"
import { TooltipProvider } from "@/components/ui/tooltip"
import { NAVIGATE_EVENT, WORKSPACE_PAGE_EVENT } from "@/lib/api"
import type { SecurityView } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { agentConnection, profiles } from "@/test/fixtures"
import { emitBackend, eventModule, resetBackend } from "@/test/tauri"

import { SettingsWindow } from "./settings-window"
import { SETTINGS_PAGE_KEY, initialSettingsPage } from "./window-page"

vi.mock("@/lib/api", async () => {
  const mock = (await import("@/test/api-mock")).apiModule
  const real = await vi.importActual<typeof import("@/lib/api")>("@/lib/api")
  return { ...real, ...mock }
})
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

function security(unlocked: boolean) {
  return {
    database: "encrypted",
    protection: "systemManaged",
    unlocked,
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
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetBackend()
  eventModule.emitTo.mockClear()
  localStorage.clear()
  window.history.replaceState(null, "", "/index.html?window=app.settings")
  api.securityStatus.mockResolvedValue(security(true))
  api.listProfiles.mockResolvedValue(profiles())
  api.listModules.mockResolvedValue([])
  api.agentStatus.mockResolvedValue(agentConnection())
  api.updateState.mockRejectedValue(new Error("no updater"))
  api.showMainWindow.mockResolvedValue(true)
  api.closeWindow.mockResolvedValue(null)
  api.getProfile.mockResolvedValue({
    activeProjectId: null,
    issues: [],
    profile: {
      id: "profile-1",
      name: "Work",
      description: "",
      actions: [],
      workflows: [],
      projects: [],
      layouts: [],
    },
  } as never)
})
afterEach(cleanup)

function renderWindow() {
  const user = userEvent.setup()
  render(
    <ThemeProvider>
      <TooltipProvider>
        <SettingsWindow />
      </TooltipProvider>
    </ThemeProvider>,
  )
  return user
}

describe("settings window", { timeout: 20_000 }, () => {
  it("opens at the page it was asked for, else where it was left", () => {
    expect(initialSettingsPage("?window=app.settings&page=security", null)).toBe("security")
    expect(initialSettingsPage("?page=settings:updates", null)).toBe("updates")
    // Pages that moved to the settings.
    expect(initialSettingsPage("?page=profiles", null)).toBe("profiles")
    const storage = { getItem: (key: string) => (key === SETTINGS_PAGE_KEY ? "storage" : null) }
    expect(initialSettingsPage("?window=app.settings", storage)).toBe("storage")
    // A workspace page is no settings page.
    expect(initialSettingsPage("?page=designer", null)).toBe("appearance")
    expect(initialSettingsPage("?page=projects/projects", null)).toBe("appearance")
  })

  it("shows the sections, switches pages and follows the main window's requests", async () => {
    window.history.replaceState(null, "", "/index.html?window=app.settings&page=appearance")
    const user = renderWindow()
    const back = await screen.findByRole("button", { name: "Back to DeckPilot" })
    const nav = back.closest("[data-slot='sidebar']") as HTMLElement
    for (const name of ["Appearance", "Profiles", "Updates", "Devices", "Background Agent"]) {
      expect(within(nav).getByRole("button", { name })).toBeInTheDocument()
    }
    expect(
      await screen.findByText(/System follows the light or dark/, undefined, { timeout: 5000 }),
    ).toBeInTheDocument()

    await user.click(within(nav).getByRole("button", { name: "Profiles" }))
    expect(
      await screen.findByText("All profiles", undefined, { timeout: 5000 }),
    ).toBeInTheDocument()
    expect(localStorage.getItem(SETTINGS_PAGE_KEY)).toBe("profiles")
    // Back and forward within the window.
    await user.click(screen.getByRole("button", { name: "Back" }))
    expect(
      await screen.findByText(/System follows the light or dark/, undefined, { timeout: 5000 }),
    ).toBeInTheDocument()

    // The main window asks for another page while the window is open.
    act(() => {
      emitBackend(WORKSPACE_PAGE_EVENT, "profiles")
    })
    expect(
      await screen.findByText("All profiles", undefined, { timeout: 5000 }),
    ).toBeInTheDocument()
  })

  it("hands pages of the main window to it and returns there", async () => {
    const user = renderWindow()
    await screen.findByRole("button", { name: "Back to DeckPilot" })
    act(() => {
      emitBackend(WORKSPACE_PAGE_EVENT, "designer")
    })
    await waitFor(() => {
      expect(eventModule.emitTo).toHaveBeenCalledWith("main", NAVIGATE_EVENT, "designer")
    })
    await waitFor(() => {
      expect(api.showMainWindow).toHaveBeenCalled()
    })
    api.showMainWindow.mockClear()
    await user.click(screen.getByRole("button", { name: "Back to DeckPilot" }))
    expect(api.showMainWindow).toHaveBeenCalled()
  })

  it("closes when the configuration is locked, without opening a prompt", async () => {
    api.securityStatus.mockResolvedValue(security(false))
    renderWindow()
    await waitFor(() => {
      expect(api.closeWindow).toHaveBeenCalled()
    })
    expect(api.openWindow).not.toHaveBeenCalled()
  })
})
