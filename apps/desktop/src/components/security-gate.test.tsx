// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { SecurityGate } from "./security-gate"
import { ConfirmWindow, MigrateWindow, SetupWindow, UnlockWindow } from "@/windows/security-windows"
import type { SecurityView } from "@/lib/types"

const api = {
  securityStatus: vi.fn(),
  setupPassword: vi.fn(),
  unlock: vi.fn(),
  confirmPassword: vi.fn(),
  encryptLegacyDatabase: vi.fn(),
  openWindow: vi.fn(),
  showMainWindow: vi.fn(),
  finishSecurityPrompt: vi.fn(),
}

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => String(error),
  SECURITY_CHANGED_EVENT: "deckpilot://security-changed",
}))
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }))

function view(overrides: Partial<SecurityView>): SecurityView {
  return {
    database: "encrypted",
    unlocked: false,
    confirmationRequired: false,
    appVersion: "0.4.0",
    legacyCopies: [],
    backupDir: "/data/backups",
    ...overrides,
  }
}

beforeEach(() => {
  for (const mock of Object.values(api)) {
    mock.mockReset()
    mock.mockResolvedValue(null)
  }
})
afterEach(cleanup)

describe("SecurityGate (main window)", () => {
  it.each([
    [view({ database: "missing" }), "security.setup"],
    [view({ database: "plaintext" }), "security.migrate"],
    [view({ confirmationRequired: true }), "security.confirm"],
    [view({}), "security.unlock"],
  ])("opens the matching prompt window and keeps the main window closed", async (state, id) => {
    api.securityStatus.mockResolvedValue(state)
    render(
      <SecurityGate>
        <p>Configuration studio</p>
      </SecurityGate>,
    )
    await waitFor(() => {
      expect(api.openWindow).toHaveBeenCalledWith(id)
    })
    expect(api.showMainWindow).not.toHaveBeenCalled()
    expect(screen.queryByText("Configuration studio")).not.toBeInTheDocument()
  })

  it("shows the main window once unlocked", async () => {
    api.securityStatus.mockResolvedValue(view({ unlocked: true }))
    render(
      <SecurityGate>
        <p>Configuration studio</p>
      </SecurityGate>,
    )
    expect(await screen.findByText("Configuration studio")).toBeInTheDocument()
    expect(api.showMainWindow).toHaveBeenCalled()
    expect(api.openWindow).not.toHaveBeenCalled()
  })
})

describe("security prompt windows", () => {
  it("validates a new password before setting it up", async () => {
    const user = userEvent.setup()
    render(<SetupWindow />)
    const create = screen.getByRole("button", { name: /create encrypted configuration/i })
    expect(screen.getByLabelText("Password")).toHaveFocus()
    await user.type(screen.getByLabelText("Password"), "short")
    expect(screen.getByText("Use at least 8 characters.")).toBeInTheDocument()
    expect(create).toBeDisabled()
    await user.type(screen.getByLabelText("Password"), " but longer")
    await user.type(screen.getByLabelText("Repeat password"), "short but longeR")
    expect(screen.getByText("The passwords do not match.")).toBeInTheDocument()
    await user.clear(screen.getByLabelText("Repeat password"))
    await user.type(screen.getByLabelText("Repeat password"), "short but longer{Enter}")
    expect(api.setupPassword).toHaveBeenCalledWith("short but longer", "short but longer")
    await waitFor(() => {
      expect(api.finishSecurityPrompt).toHaveBeenCalled()
    })
  })

  it("reports a wrong password, clears the field and allows another try", async () => {
    const user = userEvent.setup()
    api.unlock.mockRejectedValueOnce("the password is not correct").mockResolvedValueOnce(null)
    render(<UnlockWindow />)
    expect(screen.getByRole("alertdialog", { name: "DeckPilot is locked" })).toBeInTheDocument()
    await user.type(screen.getByLabelText("Password"), "wrong one{Enter}")
    expect(await screen.findByRole("alert")).toHaveTextContent("the password is not correct")
    expect(screen.getByLabelText("Password")).toHaveValue("")
    expect(api.finishSecurityPrompt).not.toHaveBeenCalled()
    await user.type(screen.getByLabelText("Password"), "right one{Enter}")
    await waitFor(() => {
      expect(api.finishSecurityPrompt).toHaveBeenCalledTimes(1)
    })
    expect(api.unlock).toHaveBeenLastCalledWith("right one")
  })

  it("cannot be dismissed with Escape", async () => {
    const user = userEvent.setup()
    render(<ConfirmWindow />)
    await user.keyboard("{Escape}")
    expect(screen.getByRole("alertdialog", { name: "DeckPilot was updated" })).toBeInTheDocument()
    expect(api.finishSecurityPrompt).not.toHaveBeenCalled()
  })

  it("explains that the unencrypted original is kept", () => {
    render(<MigrateWindow />)
    expect(screen.getByText("The unencrypted original is kept")).toBeInTheDocument()
  })
})
