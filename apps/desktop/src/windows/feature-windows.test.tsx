// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { ImportPreview, SecurityView } from "@/lib/types"
import { type UpdateStateName, updateState } from "@/test/fixtures"

import { LogsWindow } from "./logs-window"
import { ExportWindow, ImportWindow } from "./portability-windows"
import { UpdateWindow } from "./update-window"
import { WindowContext } from "./window-context"

const api = {
  listProfiles: vi.fn(),
  securityStatus: vi.fn(),
  exportConfiguration: vi.fn(),
  authenticate: vi.fn(),
  previewImport: vi.fn(),
  applyImport: vi.fn(),
  cancelImport: vi.fn(),
  showMainWindow: vi.fn(),
  readAgentLog: vi.fn(),
  updateState: vi.fn(),
  checkForUpdates: vi.fn(),
  downloadUpdate: vi.fn(),
  installUpdate: vi.fn(),
}
const dialog = { save: vi.fn(), open: vi.fn() }
const emitTo = vi.fn()

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => String(error),
  isAuthenticationRequired: (error: unknown) => String(error) === "authentication required",
  NAVIGATE_EVENT: "deckpilot://navigate",
  UPDATE_STATE_EVENT: "update-state",
}))
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (...args: unknown[]) => dialog.save(...args) as unknown,
  open: (...args: unknown[]) => dialog.open(...args) as unknown,
}))
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => undefined),
  emitTo: (...args: unknown[]) => emitTo(...args) as unknown,
}))

const close = vi.fn()

// jsdom implements neither; the components only need them to exist.
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
)
Object.defineProperty(Element.prototype, "scrollIntoView", { value: vi.fn(), writable: true })

function inWindow(children: ReactNode) {
  return render(
    <WindowContext
      value={{ spec: { id: "test", title: "Test", size: "medium", blocking: false }, close }}
    >
      {children}
    </WindowContext>,
  )
}

const security = { verification: "password" } as SecurityView

beforeEach(() => {
  for (const mock of [...Object.values(api), dialog.save, dialog.open, emitTo, close]) {
    mock.mockReset()
    mock.mockResolvedValue(null)
  }
})
afterEach(cleanup)

describe("export window", () => {
  it("authenticates, then exports with a separate export password", async () => {
    const user = userEvent.setup()
    api.listProfiles.mockResolvedValue({
      profiles: [
        { id: "p1", name: "Work", updatedAt: 0 },
        { id: "p2", name: "Home", updatedAt: 0 },
      ],
      activeProfileId: null,
    })
    api.securityStatus.mockResolvedValue(security)
    dialog.save.mockResolvedValue("/tmp/out.deckpilot")
    api.exportConfiguration
      .mockRejectedValueOnce("authentication required")
      .mockResolvedValue({ path: "/tmp/out.deckpilot", profiles: 1, modules: 2, bytes: 10 })
    inWindow(<ExportWindow />)
    await user.click(await screen.findByLabelText("Home"))
    await user.type(screen.getByLabelText("Password"), "export pw 1")
    await user.type(screen.getByLabelText("Repeat password"), "export pw 1")
    await user.click(screen.getByRole("button", { name: "Choose file and export" }))
    // The database password confirms the sensitive operation…
    await user.type(
      await screen.findByLabelText("Password", { selector: "#authenticate-password" }),
      "db password{Enter}",
    )
    expect(api.authenticate).toHaveBeenCalledWith("db password")
    expect(await screen.findByText("Configuration exported")).toBeInTheDocument()
    // …and the export password protects the file.
    expect(api.exportConfiguration).toHaveBeenLastCalledWith({
      path: "/tmp/out.deckpilot",
      password: "export pw 1",
      confirmation: "export pw 1",
      profiles: ["p1"],
      includeModuleSettings: true,
      includeAppSettings: true,
    })
  })

  it("does nothing when the save dialog is cancelled", async () => {
    const user = userEvent.setup()
    api.listProfiles.mockResolvedValue({
      profiles: [{ id: "p1", name: "Work", updatedAt: 0 }],
      activeProfileId: null,
    })
    api.securityStatus.mockResolvedValue(security)
    inWindow(<ExportWindow />)
    await user.type(await screen.findByLabelText("Password"), "export pw 1")
    await user.type(screen.getByLabelText("Repeat password"), "export pw 1")
    await user.click(screen.getByRole("button", { name: "Choose file and export" }))
    expect(api.exportConfiguration).not.toHaveBeenCalled()
  })
})

const preview: ImportPreview = {
  exportedAt: 1_700_000_000,
  appVersion: "0.4.0",
  profiles: [
    {
      id: "p1",
      name: "Work",
      actions: 2,
      workflows: 1,
      projects: 1,
      layouts: 1,
      conflict: { type: "sameId", name: "Work" },
      modules: [],
    },
  ],
  modules: [],
  unknownActionTypes: [],
  missingResources: [{ key: "shop.repository", kind: "directory", profiles: ["Work"] }],
  warnings: [],
  localProfiles: [["p1", "Work"]],
  settings: { updates: { mode: "notifyOnly", automaticChecks: true, checkIntervalHours: 24 } },
  variables: { new: [], existing: [], incompatible: [], values: 0, globalValues: 0 },
}

describe("import window", () => {
  it("previews, resolves conflicts and applies in one step", async () => {
    const user = userEvent.setup()
    dialog.open.mockResolvedValue("/tmp/in.deckpilot")
    api.previewImport
      .mockRejectedValueOnce("the password is wrong or the file was modified")
      .mockResolvedValue({ token: 7, preview })
    api.applyImport.mockResolvedValue({
      imported: [["p2", "Work (imported)"]],
      removed: [],
      skipped: [],
      renamed: ["Work (imported)"],
      modules: [],
      settings: ["updates"],
      variables: [],
      variableValues: 0,
      skippedValues: [],
    })
    inWindow(<ImportWindow />)
    await user.click(screen.getByRole("button", { name: "Choose file…" }))
    await user.type(await screen.findByLabelText("Export password"), "wrong{Enter}")
    expect(await screen.findByRole("alert")).toHaveTextContent("password is wrong")
    expect(screen.getByLabelText("Export password")).toHaveValue("")
    await user.type(screen.getByLabelText("Export password"), "right{Enter}")
    expect(await screen.findByText("Review the import")).toBeInTheDocument()
    expect(screen.getByText("Already here as “Work”")).toBeInTheDocument()
    expect(api.applyImport).not.toHaveBeenCalled()
    await user.click(screen.getByLabelText("Apply application settings (updates)"))
    await user.click(screen.getByRole("button", { name: "Import" }))
    expect(api.applyImport).toHaveBeenCalledWith(7, {
      mode: "merge",
      profiles: ["p1"],
      conflicts: {},
      moduleSettings: false,
      appSettings: true,
    })
    expect(await screen.findByText("Import complete")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Map resources now" }))
    expect(emitTo).toHaveBeenCalledWith("main", "deckpilot://navigate", "projects/projects")
  })

  it("shows the shared variables an import brings and the values it skipped", async () => {
    const user = userEvent.setup()
    dialog.open.mockResolvedValue("/tmp/in.deckpilot")
    api.previewImport.mockResolvedValue({
      token: 8,
      preview: {
        ...preview,
        missingResources: [],
        settings: {},
        variables: {
          new: ["paths.root"],
          existing: ["deploy.target"],
          incompatible: ["build.jobs"],
          values: 3,
          globalValues: 1,
        },
      },
    })
    api.applyImport.mockResolvedValue({
      imported: [["p1", "Work"]],
      removed: [],
      skipped: [],
      renamed: [],
      modules: [],
      settings: [],
      variables: ["paths.root"],
      variableValues: 2,
      skippedValues: ["`build.jobs`: `build.jobs` holds integer values, not string"],
    })
    inWindow(<ImportWindow />)
    await user.click(screen.getByRole("button", { name: "Choose file…" }))
    await user.type(await screen.findByLabelText("Export password"), "right{Enter}")
    const section = await screen.findByLabelText("Shared variables")
    expect(section).toHaveTextContent("New: paths.root.")
    expect(section).toHaveTextContent("Already here, kept as they are: deploy.target.")
    expect(section).toHaveTextContent(
      "3 values for profiles and projects, 1 global value with the application settings.",
    )
    expect(screen.getByText("Some shared variables have another type here")).toBeInTheDocument()
    // Global values come with the application settings, even without update settings.
    await user.click(
      screen.getByLabelText("Apply application settings (global shared variable values)"),
    )
    await user.click(screen.getByRole("button", { name: "Import" }))
    expect(api.applyImport).toHaveBeenCalledWith(8, expect.objectContaining({ appSettings: true }))
    expect(await screen.findByText("Import complete")).toBeInTheDocument()
    expect(
      screen.getByText("Shared variables: paths.root created, 2 values set."),
    ).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Values not imported" })).toHaveTextContent(
      "`build.jobs`: `build.jobs` holds integer values, not string",
    )
  })

  it("cancels a previewed import when closed", async () => {
    const user = userEvent.setup()
    dialog.open.mockResolvedValue("/tmp/in.deckpilot")
    api.previewImport.mockResolvedValue({ token: 7, preview })
    inWindow(<ImportWindow />)
    await user.click(screen.getByRole("button", { name: "Choose file…" }))
    await user.type(await screen.findByLabelText("Export password"), "right{Enter}")
    await user.click(await screen.findByRole("button", { name: "Cancel" }))
    expect(api.cancelImport).toHaveBeenCalled()
    expect(close).toHaveBeenCalled()
  })
})

describe("logs and update windows", () => {
  it("shows the end of the agent log and refreshes on request", async () => {
    const user = userEvent.setup()
    api.readAgentLog.mockResolvedValue({
      directory: "/data/logs",
      file: "/data/logs/agent.2026-11-10.log",
      lines: ["INFO started", "INFO configuration unlocked"],
      truncated: true,
    })
    inWindow(<LogsWindow />)
    expect(await screen.findByLabelText("Log lines")).toHaveTextContent("configuration unlocked")
    expect(screen.getByText("Showing the last 2 lines.")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Refresh" }))
    await waitFor(() => {
      expect(api.readAgentLog).toHaveBeenCalledTimes(2)
    })
  })

  it("explains builds without updates and offers the next step otherwise", async () => {
    const base = updateState("notConfigured")
    api.updateState.mockResolvedValue(base)
    const { unmount } = inWindow(<UpdateWindow />)
    expect(await screen.findByText(/no update signing key/)).toBeInTheDocument()
    unmount()

    const user = userEvent.setup()
    api.updateState.mockResolvedValue({
      ...updateState("githubDownloading"),
      phase: "downloading",
      available: { version: "0.4.1", notes: "Fixes", date: null },
      progress: { downloaded: 50, total: 100 },
    })
    inWindow(<UpdateWindow />)
    expect(await screen.findByText("50%")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Downloading/ })).toBeDisabled()
    api.updateState.mockResolvedValue({
      ...updateState("githubReady"),
      phase: "ready",
      available: { version: "0.4.1", notes: null, date: null },
    })
    cleanup()
    inWindow(<UpdateWindow />)
    await user.click(await screen.findByRole("button", { name: "Install and restart" }))
    expect(api.installUpdate).toHaveBeenCalled()
  })

  it.each<[UpdateStateName, string | null, string | null]>([
    ["apt", "Updates are delivered by APT", "sudo apt update && sudo apt upgrade"],
    ["homebrew", "Updates are delivered by Homebrew", "brew upgrade --cask deckpilot"],
    ["winget", "Updates are delivered by WinGet", "winget upgrade KnittingCode.DeckPilot"],
    ["flathub", "Updates are delivered by Flathub", null],
    ["macAppStore", "Updates are delivered by the Mac App Store", null],
    ["microsoftStore", "Updates are delivered by the Microsoft Store", null],
    ["notConfigured", null, null],
  ])("offers a %s build nothing to check, download or install", async (name, by, command) => {
    // Even with an update in its state, as the GitHub code path would leave it.
    api.updateState.mockResolvedValue(
      updateState(name, {
        phase: "ready",
        available: { version: "0.7.0", notes: "Notes", date: null },
      }),
    )
    const user = userEvent.setup()
    inWindow(<UpdateWindow />)
    expect(await screen.findByText(/^Installed: 0\.6\.0-alpha\.3/)).toBeInTheDocument()
    if (by) expect(screen.getByText(by)).toBeInTheDocument()
    if (command) expect(screen.getByLabelText("Update command")).toHaveTextContent(command)
    else expect(screen.queryByLabelText("Update command")).not.toBeInTheDocument()
    expect(screen.queryByText("Notes")).not.toBeInTheDocument()
    for (const label of [/Install/, /Download/, /Check now/]) {
      expect(screen.queryByRole("button", { name: label })).not.toBeInTheDocument()
    }
    for (const button of screen.getAllByRole("button")) {
      if (!/Copy/.test(button.textContent)) continue
      await user.click(button)
    }
    await user.click(screen.getAllByRole("button", { name: "Close" })[0] as HTMLElement)
    expect(close).toHaveBeenCalled()
    expect(api.installUpdate).not.toHaveBeenCalled()
    expect(api.downloadUpdate).not.toHaveBeenCalled()
    expect(api.checkForUpdates).not.toHaveBeenCalled()
  })

  it("shows a refused GitHub update with its reason", async () => {
    api.updateState.mockResolvedValue(updateState("githubUntrustedUrl"))
    inWindow(<UpdateWindow />)
    expect(
      await screen.findByText("Update refused: unexpected download address"),
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Check now" })).toBeInTheDocument()
  })
})
