// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "vitest"

import tauriConfig from "../../src-tauri/tauri.conf.json?raw"
import { LIBRARY_BESIDE_FROM, LibraryDock } from "./library-dock"
import {
  LIBRARY_COMPATIBLE_ONLY_KEY,
  OBSOLETE_LIBRARY_KEYS,
  removeObsoleteLibraryKeys,
} from "./library-sidebar"

function renderDock() {
  render(
    <LibraryDock>
      <input aria-label="Search the library" />
    </LibraryDock>,
  )
  return screen.getByRole("complementary", { name: "Action Library" })
}

beforeEach(() => {
  localStorage.clear()
})
afterEach(cleanup)

describe("library dock", () => {
  it("is always shown, with no way to collapse or resize it", () => {
    const library = renderDock()
    expect(within(library).getByLabelText("Search the library")).toBeVisible()
    expect(within(library).queryByRole("button")).not.toBeInTheDocument()
    expect(screen.queryByRole("separator")).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /collapse|show the library|library/i })).toBeNull()
    expect(library).not.toHaveAttribute("aria-expanded")
  })

  it("has the fixed width of the design token beside the editor", () => {
    const library = renderDock()
    // No inline width (nothing remembered or dragged): the token decides.
    expect(library.style.width).toBe("")
    expect(library.className).toContain("min-[860px]:w-(--library-width)")
    expect(library.className).toContain("shrink-0")
  })

  it("sits under the editor, full width, in windows narrower than the minimum", () => {
    const library = renderDock()
    expect(LIBRARY_BESIDE_FROM).toBe(860)
    expect(library.className).toContain("max-[859px]:w-full")
    expect(library.className).toContain("max-[859px]:h-(--library-stacked-height)")
    // The main window cannot be made narrower than that.
    const config = JSON.parse(tauriConfig) as {
      app: { windows: { label: string; minWidth: number; minHeight: number }[] }
    }
    const main = config.app.windows.find((w) => w.label === "main")
    expect(main?.minWidth).toBe(LIBRARY_BESIDE_FROM)
    expect(main?.minHeight).toBeGreaterThanOrEqual(560)
  })

  it("ignores and removes the width and collapsed state earlier versions kept", () => {
    localStorage.setItem("deckpilot.editor.libraryWidth", "120")
    localStorage.setItem("deckpilot.editor.libraryCollapsed", "true")
    localStorage.setItem(LIBRARY_COMPATIBLE_ONLY_KEY, "true")
    const library = renderDock()
    expect(library.style.width).toBe("")
    expect(within(library).getByLabelText("Search the library")).toBeVisible()
    removeObsoleteLibraryKeys()
    for (const key of OBSOLETE_LIBRARY_KEYS) expect(localStorage.getItem(key)).toBeNull()
    // The library's other preference stays.
    expect(localStorage.getItem(LIBRARY_COMPATIBLE_ONLY_KEY)).toBe("true")
  })
})
