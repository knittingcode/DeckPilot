// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { SecurityGate } from "./security-gate"
import type { SecurityView } from "@/lib/types"

const securityStatus = vi.fn()
const setupPassword = vi.fn()
const unlock = vi.fn()
const confirmPassword = vi.fn()
const encryptLegacyDatabase = vi.fn()

vi.mock("@/lib/api", () => ({
  api: {
    securityStatus: () => securityStatus() as unknown,
    setupPassword: (...args: unknown[]) => setupPassword(...args) as unknown,
    unlock: (...args: unknown[]) => unlock(...args) as unknown,
    confirmPassword: (...args: unknown[]) => confirmPassword(...args) as unknown,
    encryptLegacyDatabase: (...args: unknown[]) => encryptLegacyDatabase(...args) as unknown,
  },
  errorMessage: (error: unknown) => String(error),
}))

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

const unlocked = view({ unlocked: true })

function renderGate() {
  render(
    <SecurityGate>
      <p>Configuration studio</p>
    </SecurityGate>,
  )
}

describe("SecurityGate", () => {
  beforeEach(() => {
    for (const mock of [
      securityStatus,
      setupPassword,
      unlock,
      confirmPassword,
      encryptLegacyDatabase,
    ]) {
      mock.mockReset()
    }
  })
  afterEach(cleanup)

  it("asks for a confirmed password on first start", async () => {
    const user = userEvent.setup()
    securityStatus.mockResolvedValue(view({ database: "missing" }))
    setupPassword.mockResolvedValue(unlocked)
    renderGate()
    expect(await screen.findByText("Welcome to DeckPilot")).toBeInTheDocument()
    const create = screen.getByRole("button", { name: /create encrypted configuration/i })

    await user.type(screen.getByLabelText("Password"), "short")
    expect(screen.getByText("Use at least 8 characters.")).toBeInTheDocument()
    expect(create).toBeDisabled()

    await user.type(screen.getByLabelText("Password"), " but longer")
    await user.type(screen.getByLabelText("Repeat password"), "short but longeR")
    expect(screen.getByText("The passwords do not match.")).toBeInTheDocument()
    expect(create).toBeDisabled()

    await user.clear(screen.getByLabelText("Repeat password"))
    await user.type(screen.getByLabelText("Repeat password"), "short but longer")
    await user.click(create)
    expect(setupPassword).toHaveBeenCalledWith("short but longer", "short but longer")
    expect(await screen.findByText("Configuration studio")).toBeInTheDocument()
  })

  it("shows a wrong password and lets the user try again", async () => {
    const user = userEvent.setup()
    securityStatus.mockResolvedValue(view({}))
    unlock.mockRejectedValueOnce("the password is not correct").mockResolvedValueOnce(unlocked)
    renderGate()
    expect(await screen.findByText("DeckPilot is locked")).toBeInTheDocument()
    await user.type(screen.getByLabelText("Password"), "wrong one")
    await user.click(screen.getByRole("button", { name: "Unlock" }))
    expect(await screen.findByRole("alert")).toHaveTextContent("the password is not correct")
    // The field is cleared for the next attempt.
    expect(screen.getByLabelText("Password")).toHaveValue("")
    await user.type(screen.getByLabelText("Password"), "right one")
    await user.click(screen.getByRole("button", { name: "Unlock" }))
    expect(await screen.findByText("Configuration studio")).toBeInTheDocument()
    expect(unlock).toHaveBeenLastCalledWith("right one")
  })

  it("asks once after an update", async () => {
    const user = userEvent.setup()
    securityStatus.mockResolvedValue(view({ confirmationRequired: true }))
    confirmPassword.mockResolvedValue(unlocked)
    renderGate()
    expect(await screen.findByText("DeckPilot 0.4.0 is installed")).toBeInTheDocument()
    await user.type(screen.getByLabelText("Password"), "my password")
    await user.click(screen.getByRole("button", { name: "Continue" }))
    await waitFor(() => {
      expect(screen.getByText("Configuration studio")).toBeInTheDocument()
    })
    expect(confirmPassword).toHaveBeenCalledWith("my password")
  })

  it("explains that the unencrypted original is kept when migrating", async () => {
    securityStatus.mockResolvedValue(view({ database: "plaintext" }))
    renderGate()
    expect(await screen.findByText("Encrypt your configuration")).toBeInTheDocument()
    expect(screen.getByText("The unencrypted original is kept")).toBeInTheDocument()
    expect(screen.queryByText("Configuration studio")).not.toBeInTheDocument()
  })
})
