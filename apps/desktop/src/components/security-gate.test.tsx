// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { SensitiveOperations } from "./authentication"
import { isCancelled, useSensitive } from "@/lib/sensitive"
import { SecurityGate } from "./security-gate"
import { ConfirmWindow, MigrateWindow, SetupWindow, UnlockWindow } from "@/windows/security-windows"
import type { SecurityView } from "@/lib/types"

const api = {
  securityStatus: vi.fn(),
  setupPassword: vi.fn(),
  unlock: vi.fn(),
  systemUnlock: vi.fn(),
  confirmIdentity: vi.fn(),
  authenticate: vi.fn(),
  encryptLegacyDatabase: vi.fn(),
  openWindow: vi.fn(),
  showMainWindow: vi.fn(),
  finishSecurityPrompt: vi.fn(),
}

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => String(error),
  isAuthenticationRequired: (error: unknown) => String(error) === "authentication required",
  SECURITY_CHANGED_EVENT: "deckpilot://security-changed",
}))
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }))

function view(overrides: Partial<SecurityView>): SecurityView {
  return {
    database: "encrypted",
    protection: "systemManaged",
    unlocked: false,
    confirmationRequired: false,
    lockReason: null,
    problem: null,
    keyStore: "macOS Keychain",
    verification: "touchId",
    policy: null,
    agentRunning: true,
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
  it("offers a retry or a password when the system key could not be stored", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(
      view({ database: "missing", protection: null, problem: "no Secret Service on D-Bus" }),
    )
    render(<SetupWindow />)
    expect(await screen.findByText("no Secret Service on D-Bus")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Try again" }))
    expect(api.systemUnlock).toHaveBeenCalled()
    await waitFor(() => {
      expect(api.finishSecurityPrompt).toHaveBeenCalledTimes(1)
    })

    await user.click(screen.getByRole("button", { name: "Use a password instead" }))
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
  })

  it("asks for the password with password protection, and allows another try", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(view({ protection: "password", verification: "password" }))
    api.unlock.mockRejectedValueOnce("the password is not correct").mockResolvedValueOnce(null)
    render(<UnlockWindow />)
    // Loaded: the password field replaces the loading state.
    await user.type(await screen.findByLabelText("Password"), "wrong one{Enter}")
    expect(screen.getByRole("alertdialog", { name: "DeckPilot is locked" })).toBeInTheDocument()
    expect(await screen.findByRole("alert")).toHaveTextContent("the password is not correct")
    expect(screen.getByLabelText("Password")).toHaveValue("")
    expect(api.finishSecurityPrompt).not.toHaveBeenCalled()
    await user.type(screen.getByLabelText("Password"), "right one{Enter}")
    await waitFor(() => {
      expect(api.finishSecurityPrompt).toHaveBeenCalledTimes(1)
    })
    expect(api.unlock).toHaveBeenLastCalledWith("right one")
  })

  it("unlocks a system key configuration with Touch ID after an automatic lock", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(view({ lockReason: "idle" }))
    render(<UnlockWindow />)
    expect(await screen.findByText(/without activity/)).toBeInTheDocument()
    expect(screen.queryByLabelText("Password")).not.toBeInTheDocument()
    // The system prompt only appears when the user asks for it.
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(api.systemUnlock).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Unlock with Touch ID" }))
    expect(api.systemUnlock).toHaveBeenCalled()
    await waitFor(() => {
      expect(api.finishSecurityPrompt).toHaveBeenCalled()
    })
  })

  it("explains an unavailable credential store instead of asking repeatedly", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(
      view({ lockReason: "keyUnavailable", problem: "the keychain is locked" }),
    )
    api.systemUnlock.mockRejectedValueOnce("the keychain is locked")
    render(<UnlockWindow />)
    expect(await screen.findByText(/did not provide it/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Try again" }))
    expect(await screen.findAllByText("the keychain is locked")).not.toHaveLength(0)
    expect(api.finishSecurityPrompt).not.toHaveBeenCalled()
    expect(api.systemUnlock).toHaveBeenCalledTimes(1)
  })

  it("cannot be dismissed with Escape", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(view({ protection: "password", verification: "password" }))
    render(<ConfirmWindow />)
    expect(await screen.findByLabelText("Password")).toBeInTheDocument()
    await user.keyboard("{Escape}")
    expect(screen.getByRole("alertdialog", { name: "Confirm it's you" })).toBeInTheDocument()
    expect(api.finishSecurityPrompt).not.toHaveBeenCalled()
  })

  it("explains that the unencrypted original is kept", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(view({ database: "plaintext", protection: null }))
    render(<MigrateWindow />)
    await user.click(await screen.findByRole("button", { name: "Use a password instead" }))
    expect(screen.getByText("The unencrypted original is kept")).toBeInTheDocument()
  })
})

describe("sensitive operations", () => {
  function Export({
    task,
    onResult,
  }: {
    task: () => Promise<string>
    onResult: (r: string) => void
  }) {
    const sensitive = useSensitive()
    return (
      <button
        onClick={() => {
          sensitive(task, "export your configuration").then(onResult, (e: unknown) => {
            onResult(isCancelled(e) ? "cancelled" : String(e))
          })
        }}
      >
        Export
      </button>
    )
  }

  it("authenticates with the password and retries once", async () => {
    const user = userEvent.setup()
    const task = vi.fn().mockRejectedValueOnce("authentication required").mockResolvedValue("done")
    const onResult = vi.fn()
    render(
      <SensitiveOperations verification="password">
        <Export task={task} onResult={onResult} />
      </SensitiveOperations>,
    )
    await user.click(screen.getByRole("button", { name: "Export" }))
    expect(
      await screen.findByText("Authenticate to export your configuration."),
    ).toBeInTheDocument()
    await user.type(screen.getByLabelText("Password"), "my password{Enter}")
    expect(api.authenticate).toHaveBeenCalledWith("my password")
    await waitFor(() => {
      expect(onResult).toHaveBeenCalledWith("done")
    })
    expect(task).toHaveBeenCalledTimes(2)
  })

  it("uses Touch ID without a password field, and can be cancelled", async () => {
    const user = userEvent.setup()
    const task = vi.fn().mockRejectedValue("authentication required")
    const onResult = vi.fn()
    render(
      <SensitiveOperations verification="touchId">
        <Export task={task} onResult={onResult} />
      </SensitiveOperations>,
    )
    await user.click(screen.getByRole("button", { name: "Export" }))
    expect(
      await screen.findByRole("button", { name: "Continue with Touch ID" }),
    ).toBeInTheDocument()
    expect(screen.queryByLabelText("Password")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(onResult).toHaveBeenCalledWith("cancelled")
    })
    expect(api.authenticate).not.toHaveBeenCalled()
  })

  it("passes other errors through without asking", async () => {
    const user = userEvent.setup()
    const onResult = vi.fn()
    render(
      <SensitiveOperations verification="password">
        <Export task={() => Promise.reject(new Error("disk full"))} onResult={onResult} />
      </SensitiveOperations>,
    )
    await user.click(screen.getByRole("button", { name: "Export" }))
    await waitFor(() => {
      expect(onResult).toHaveBeenCalledWith("Error: disk full")
    })
    expect(screen.queryByText(/Authenticate to/)).not.toBeInTheDocument()
  })
})
