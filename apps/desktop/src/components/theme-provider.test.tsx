// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { THEME_STORAGE_KEY, useTheme } from "@/lib/theme"
import { THEME_EVENT, WINDOW_INSTANCE } from "@/lib/window-sync"
import { installDomStubs } from "@/test/dom"
import { emitBackend, eventModule, resetBackend } from "@/test/tauri"

import { ThemeProvider } from "./theme-provider"

vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

function Probe() {
  const { preference, resolved, setPreference } = useTheme()
  return (
    <>
      <p>
        {preference}/{resolved}
      </p>
      <button
        type="button"
        onClick={() => {
          setPreference("dark")
        }}
      >
        Dark
      </button>
    </>
  )
}

beforeAll(installDomStubs)
beforeEach(() => {
  localStorage.clear()
  resetBackend()
  eventModule.emit.mockClear()
  document.documentElement.classList.remove("dark")
})
afterEach(() => {
  cleanup()
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
})

describe("theme across windows", () => {
  it("follows a choice another window stored", () => {
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    )
    expect(screen.getByText("system/light")).toBeInTheDocument()
    act(() => {
      localStorage.setItem(THEME_STORAGE_KEY, "dark")
      window.dispatchEvent(
        new StorageEvent("storage", { key: THEME_STORAGE_KEY, newValue: "dark" }),
      )
    })
    expect(screen.getByText("dark/dark")).toBeInTheDocument()
    expect(document.documentElement).toHaveClass("dark")
    // Other keys and invalid values change nothing.
    act(() => {
      window.dispatchEvent(new StorageEvent("storage", { key: "other", newValue: "light" }))
      window.dispatchEvent(
        new StorageEvent("storage", { key: THEME_STORAGE_KEY, newValue: "purple" }),
      )
    })
    expect(screen.getByText("dark/dark")).toBeInTheDocument()
  })

  it("announces its own choice and follows other windows' announcements", async () => {
    // Inside Tauri: the app event reaches every window.
    Object.assign(window, { __TAURI_INTERNALS__: {} })
    const user = userEvent.setup()
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    )
    await user.click(screen.getByRole("button", { name: "Dark" }))
    expect(eventModule.emit).toHaveBeenCalledWith(THEME_EVENT, {
      preference: "dark",
      source: WINDOW_INSTANCE,
    })
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark")
    await act(async () => {
      await Promise.resolve()
    })
    // Its own announcement is ignored; another window's is followed.
    act(() => {
      emitBackend(THEME_EVENT, { preference: "light", source: WINDOW_INSTANCE })
    })
    expect(screen.getByText("dark/dark")).toBeInTheDocument()
    act(() => {
      emitBackend(THEME_EVENT, { preference: "light", source: "settings-window" })
    })
    expect(screen.getByText("light/light")).toBeInTheDocument()
    expect(document.documentElement).not.toHaveClass("dark")
  })
})
