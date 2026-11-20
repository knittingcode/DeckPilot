// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { initLanguage } from "@/i18n/language"
import { LANGUAGE_STORAGE_KEY } from "@/i18n/preference"
import type { SecurityView } from "@/lib/types"

import { UnlockWindow } from "./security-windows"

const api = {
  securityStatus: vi.fn(),
  unlock: vi.fn(),
  finishSecurityPrompt: vi.fn(),
}

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => String(error),
}))

const locked: SecurityView = {
  database: "encrypted",
  protection: "password",
  unlocked: false,
  confirmationRequired: false,
  lockReason: null,
  waitingForKeyStore: false,
  databaseMissing: false,
  screenLockDetection: true,
  problem: null,
  keyStore: "macOS Keychain",
  verification: "password",
  policy: null,
  agentRunning: true,
  appVersion: "0.9.0",
  legacyCopies: [],
  backupDir: "/data/backups",
}

beforeEach(() => {
  api.securityStatus.mockResolvedValue(locked)
  api.unlock.mockRejectedValue("the password is not correct")
})

afterEach(async () => {
  cleanup()
  localStorage.clear()
  await act(async () => {
    await i18n.changeLanguage("en")
  })
})

describe("unlock window language", () => {
  it("starts in Polish when the language is Polish", async () => {
    // As main.tsx does before the window's first render.
    localStorage.setItem(LANGUAGE_STORAGE_KEY, "pl")
    await initLanguage()
    const user = userEvent.setup()
    render(<UnlockWindow />)

    // Loaded: the password field replaces the loading state.
    const field = await screen.findByLabelText("Hasło")
    const dialog = screen.getByRole("alertdialog", { name: "DeckPilot jest zablokowany" })
    expect(dialog).toHaveAccessibleDescription("Wprowadź hasło, aby odblokować DeckPilot.")
    expect(screen.getByRole("button", { name: "Odblokuj" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Pokaż hasło" })).toBeInTheDocument()

    await user.type(field, "złe hasło{Enter}")
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nieprawidłowe hasło. Spróbuj ponownie.",
    )
  })

  it("follows the system's languages by default", async () => {
    vi.spyOn(navigator, "languages", "get").mockReturnValue(["pl-PL"])
    await initLanguage()
    render(<UnlockWindow />)
    await screen.findByLabelText("Hasło")
    expect(
      screen.getByRole("alertdialog", { name: "DeckPilot jest zablokowany" }),
    ).toBeInTheDocument()
    vi.restoreAllMocks()
  })
})
