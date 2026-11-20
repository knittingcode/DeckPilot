// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { DialogProvider } from "@/components/dialogs"
import { ThemeProvider } from "@/components/theme-provider"
import { settingsPages } from "@/settings/registry"

import { DialogShowcase } from "./dialog-showcase"

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("dialog showcase", () => {
  it("is a settings page in development builds only", () => {
    // Vitest runs with import.meta.env.DEV; builds replace it with false.
    expect(settingsPages([]).some((page) => page.id === "dev-dialogs")).toBe(import.meta.env.DEV)
  })

  it("opens the real templates", async () => {
    const user = userEvent.setup()
    render(
      <ThemeProvider>
        <DialogProvider>
          <DialogShowcase />
        </DialogProvider>
      </ThemeProvider>,
    )
    await user.click(screen.getByRole("button", { name: "Fails (inline error)" }))
    await user.click(screen.getByRole("button", { name: "Delete" }))
    expect(await screen.findByRole("alert", {}, { timeout: 2000 })).toHaveTextContent(
      "default layout",
    )
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "Authenticate" }))
    const dialog = screen.getByRole("dialog", { name: "Enter your password" })
    await user.type(within(dialog).getByLabelText("Password"), "wrong{Enter}")
    expect(await within(dialog).findByRole("alert", {}, { timeout: 2000 })).toHaveTextContent(
      "Incorrect password. Try again.",
    )
  })
})
