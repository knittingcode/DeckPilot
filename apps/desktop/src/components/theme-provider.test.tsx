// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { LEGACY_THEME_STORAGE_KEY, THEME } from "@/lib/theme"

import { ThemeProvider } from "./theme-provider"

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  document.documentElement.className = ""
  document.documentElement.style.colorScheme = ""
  localStorage.clear()
})

describe("ThemeProvider (permanently dark)", () => {
  let listeners: (() => void)[] = []
  beforeEach(() => {
    listeners = []
    // An operating system in light mode, which then changes.
    vi.stubGlobal("matchMedia", (query: string) => ({
      media: query,
      matches: false,
      addEventListener: (_: string, listener: () => void) => listeners.push(listener),
      removeEventListener: () => undefined,
    }))
  })

  it("is dark from the first render, whatever the system appearance", () => {
    render(
      <ThemeProvider>
        <p>content</p>
      </ThemeProvider>,
    )
    expect(screen.getByText("content")).toBeInTheDocument()
    expect(document.documentElement).toHaveClass("dark")
    expect(document.documentElement.style.colorScheme).toBe("dark")
    expect(THEME).toBe("dark")
  })

  it("never listens to the operating system's appearance", () => {
    render(<ThemeProvider>x</ThemeProvider>)
    expect(listeners).toHaveLength(0)
    for (const listener of listeners) listener()
    expect(document.documentElement).toHaveClass("dark")
  })

  it("ignores and removes the light/dark/system choice of earlier versions", () => {
    localStorage.setItem(LEGACY_THEME_STORAGE_KEY, "light")
    render(<ThemeProvider>x</ThemeProvider>)
    expect(document.documentElement).toHaveClass("dark")
    expect(localStorage.getItem(LEGACY_THEME_STORAGE_KEY)).toBeNull()
  })
})
