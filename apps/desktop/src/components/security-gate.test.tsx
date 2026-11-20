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
  startNewConfiguration: vi.fn(),
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
    waitingForKeyStore: false,
    databaseMissing: false,
    screenLockDetection: true,
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
    // Shown from an effect after the content renders.
    await waitFor(() => {
      expect(api.showMainWindow).toHaveBeenCalled()
    })
    expect(api.openWindow).not.toHaveBeenCalled()
  })
})

describe("SecurityGate locked screen", () => {
  it("names the action in full and opens the unlock window again", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(view({}))
    render(
      <SecurityGate>
        <p>Configuration studio</p>
      </SecurityGate>,
    )
    const button = await screen.findByRole("button", { name: "Unlock DeckPilot" })
    expect(screen.getByRole("heading", { name: "DeckPilot is locked" })).toBeInTheDocument()
    expect(screen.queryByText("Configuration studio")).not.toBeInTheDocument()
    api.openWindow.mockClear()
    await user.click(button)
    expect(api.openWindow).toHaveBeenCalledWith("security.unlock")
  })
})

describe("password prompt layout", () => {
  it("uses the shared header and the compact footer", async () => {
    api.securityStatus.mockResolvedValue(view({ protection: "password", verification: "password" }))
    const { container } = render(<UnlockWindow />)
    await screen.findByLabelText("Password")
    const footer = container.querySelector('[data-modal-part="footer"]')
    expect(footer).toHaveClass("py-3")
    expect(footer).not.toHaveClass("py-4")
    // The shared header: the tile at the top, the heading at least as tall
    // as the tile so a title and a one-line description fit beside it.
    expect(container.querySelector('[data-modal-part="header"]')).toHaveClass("items-start")
    expect(container.querySelector('[data-modal-part="heading"]')).toHaveClass(
      "min-h-(--modal-icon)",
      "justify-center",
    )
    // One reserved line for the inline error, nothing else below the field.
    expect(container.querySelector('[data-modal-part="body"]')?.textContent).toBe("Password")
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
    expect(await screen.findByRole("alert")).toHaveTextContent("Incorrect password. Try again.")
    expect(screen.getByLabelText("Password")).toHaveValue("")
    expect(api.finishSecurityPrompt).not.toHaveBeenCalled()
    await user.type(screen.getByLabelText("Password"), "right one{Enter}")
    await waitFor(() => {
      expect(api.finishSecurityPrompt).toHaveBeenCalledTimes(1)
    })
    expect(api.unlock).toHaveBeenLastCalledWith("right one")
  })

  describe("password prompt", () => {
    async function prompt() {
      api.securityStatus.mockResolvedValue(
        view({ protection: "password", verification: "password" }),
      )
      render(<UnlockWindow />)
      return screen.findByLabelText("Password")
    }

    it("shows a wrong password inline, linked to the field, and keeps the focus there", async () => {
      const user = userEvent.setup()
      api.unlock.mockRejectedValue("the password is not correct")
      const field = await prompt()
      expect(field).toHaveFocus()
      expect(field).not.toHaveAttribute("aria-invalid")
      await user.type(field, "wrong{Enter}")
      const alert = await screen.findByRole("alert")
      expect(alert).toHaveTextContent("Incorrect password. Try again.")
      await waitFor(() => {
        expect(field).toHaveFocus()
      })
      expect(field).toHaveAttribute("aria-invalid", "true")
      expect(field).toHaveAttribute("aria-describedby", alert.id)
      expect(field).toHaveAccessibleDescription("Incorrect password. Try again.")
      // No boxed alert from the shell next to the inline message.
      expect(screen.getAllByRole("alert")).toHaveLength(1)
      // Repeated failures announce the message again.
      await user.type(field, "again{Enter}")
      await waitFor(() => {
        expect(api.unlock).toHaveBeenCalledTimes(2)
      })
      expect(await screen.findByRole("alert")).toHaveTextContent("Incorrect password")
      expect(api.finishSecurityPrompt).not.toHaveBeenCalled()
    })

    it("tells an unreachable agent apart from a wrong password and keeps the password", async () => {
      const user = userEvent.setup()
      api.unlock.mockRejectedValueOnce("the DeckPilot agent is not running")
      const field = await prompt()
      await user.type(field, "secret{Enter}")
      expect(await screen.findByRole("alert")).toHaveTextContent(/can’t reach its background agent/)
      expect(field).toHaveValue("secret")
    })

    it("never shows internal storage errors", async () => {
      const user = userEvent.setup()
      api.unlock.mockRejectedValueOnce(
        "database error: file is not a database (/Users/x/deckpilot.db)",
      )
      const field = await prompt()
      await user.type(field, "secret{Enter}")
      const alert = await screen.findByRole("alert")
      expect(alert).toHaveTextContent("Your configuration can’t be opened right now.")
      expect(alert).not.toHaveTextContent(/deckpilot\.db|database error/)
    })

    it("does not submit an empty password", async () => {
      const user = userEvent.setup()
      const field = await prompt()
      expect(screen.getByRole("button", { name: "Unlock" })).toBeDisabled()
      await user.type(field, "{Enter}")
      expect(api.unlock).not.toHaveBeenCalled()
      expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    })

    it("submits once while the password is checked", async () => {
      const user = userEvent.setup()
      let resolve: (value: null) => void = () => undefined
      api.unlock.mockReturnValueOnce(
        new Promise<null>((r) => {
          resolve = r
        }),
      )
      const field = await prompt()
      await user.type(field, "secret{Enter}")
      expect(field).toBeDisabled()
      expect(screen.getByRole("button", { name: /Checking/ })).toBeDisabled()
      await user.keyboard("{Enter}")
      expect(api.unlock).toHaveBeenCalledTimes(1)
      resolve(null)
      await waitFor(() => {
        expect(api.finishSecurityPrompt).toHaveBeenCalledTimes(1)
      })
    })

    it("masks the password by default and shows it on request", async () => {
      const user = userEvent.setup()
      const field = await prompt()
      await user.type(field, "secret")
      expect(field).toHaveAttribute("type", "password")
      await user.click(screen.getByRole("button", { name: "Show password" }))
      expect(field).toHaveAttribute("type", "text")
      expect(field).toHaveAttribute("spellcheck", "false")
      await user.click(screen.getByRole("button", { name: "Hide password" }))
      expect(field).toHaveAttribute("type", "password")
      expect(api.unlock).not.toHaveBeenCalled()
    })
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

describe("credential store states", () => {
  it("waits for the credential store and finishes once the agent has the key", async () => {
    api.securityStatus
      .mockResolvedValueOnce(view({ waitingForKeyStore: true }))
      .mockResolvedValue(view({ unlocked: true }))
    render(<UnlockWindow />)
    expect(await screen.findByText("Waiting for the macOS Keychain")).toBeInTheDocument()
    await waitFor(
      () => {
        expect(api.finishSecurityPrompt).toHaveBeenCalled()
      },
      { timeout: 3000 },
    )
    expect(api.systemUnlock).not.toHaveBeenCalled()
  })

  it("never creates a new configuration for a missing database without confirmation", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(
      view({ database: "missing", databaseMissing: true, problem: "missing" }),
    )
    api.systemUnlock.mockRejectedValue("the configuration database is missing")
    render(<SetupWindow />)
    expect(await screen.findByText("Your configuration is missing")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Use a password instead" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Try again" }))
    expect(await screen.findByRole("alert")).toHaveTextContent("missing")
    await user.click(screen.getByRole("button", { name: "Start a new configuration…" }))
    expect(api.startNewConfiguration).not.toHaveBeenCalled()
    api.startNewConfiguration.mockResolvedValue(view({ unlocked: true }))
    await user.click(screen.getByRole("button", { name: "Start a new configuration" }))
    expect(api.startNewConfiguration).toHaveBeenCalledTimes(1)
    await waitFor(() => {
      expect(api.finishSecurityPrompt).toHaveBeenCalled()
    })
  })

  it("asks for Touch ID after an update only when the user clicks", async () => {
    const user = userEvent.setup()
    api.securityStatus.mockResolvedValue(view({ confirmationRequired: true }))
    render(<ConfirmWindow />)
    const button = await screen.findByRole("button", { name: "Unlock with Touch ID" })
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(api.confirmIdentity).not.toHaveBeenCalled()
    await user.click(button)
    expect(api.confirmIdentity).toHaveBeenCalledWith(null)
  })
})
