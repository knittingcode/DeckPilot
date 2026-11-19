// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { LibraryDock } from "./library-dock"
import {
  LIBRARY_COLLAPSED_KEY,
  LIBRARY_OVERLAY_BELOW,
  LIBRARY_WIDTH,
  LIBRARY_WIDTH_KEY,
  clampLibraryWidth,
  loadLibraryWidth,
} from "./library-sidebar"

/** A window `width` pixels wide, as far as media queries go. */
function windowWidth(width: number) {
  window.matchMedia = vi.fn((query: string) => {
    const max = /max-width: (\d+)px/.exec(query)
    return {
      matches: max ? width <= Number(max[1]) : false,
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList
  })
}

function renderDock() {
  const user = userEvent.setup()
  render(
    <LibraryDock>
      <input aria-label="Search the library" />
    </LibraryDock>,
  )
  return user
}

beforeEach(() => {
  localStorage.clear()
  windowWidth(1440)
  Element.prototype.setPointerCapture = () => undefined
})
afterEach(cleanup)

describe("library sidebar", () => {
  it("keeps its width within limits", () => {
    expect(clampLibraryWidth(10)).toBe(LIBRARY_WIDTH.min)
    expect(clampLibraryWidth(9999)).toBe(LIBRARY_WIDTH.max)
    expect(clampLibraryWidth(Number.NaN)).toBe(LIBRARY_WIDTH.default)
    localStorage.setItem(LIBRARY_WIDTH_KEY, "nonsense")
    expect(loadLibraryWidth()).toBe(LIBRARY_WIDTH.default)
  })

  it("is resized with the keyboard and the pointer, and remembers the width", async () => {
    const user = renderDock()
    const library = screen.getByRole("complementary", { name: "Action Library" })
    expect(library).toHaveStyle({ width: `${LIBRARY_WIDTH.default}px` })
    const handle = screen.getByRole("separator", { name: "Resize the library" })
    handle.focus()
    // The handle is on the left edge: left widens.
    await user.keyboard("{ArrowLeft}")
    expect(handle).toHaveAttribute("aria-valuenow", String(LIBRARY_WIDTH.default + 16))
    expect(localStorage.getItem(LIBRARY_WIDTH_KEY)).toBe(String(LIBRARY_WIDTH.default + 16))
    await user.keyboard("{End}")
    expect(library).toHaveStyle({ width: `${LIBRARY_WIDTH.max}px` })

    fireEvent.pointerDown(handle, { button: 0, clientX: 800, pointerId: 1 })
    fireEvent.pointerMove(handle, { clientX: 900, pointerId: 1 })
    fireEvent.pointerUp(handle, { clientX: 900, pointerId: 1 })
    expect(localStorage.getItem(LIBRARY_WIDTH_KEY)).toBe(String(LIBRARY_WIDTH.max - 100))

    cleanup()
    renderDock()
    expect(screen.getByRole("complementary", { name: "Action Library" })).toHaveStyle({
      width: `${LIBRARY_WIDTH.max - 100}px`,
    })
  })

  it("collapses and expands, and remembers it", async () => {
    const user = renderDock()
    await user.click(screen.getByRole("button", { name: "Collapse the library" }))
    expect(screen.queryByLabelText("Search the library")).not.toBeInTheDocument()
    expect(localStorage.getItem(LIBRARY_COLLAPSED_KEY)).toBe("true")

    cleanup()
    const again = renderDock()
    expect(screen.getByRole("complementary", { name: "Action Library" })).toHaveAttribute(
      "data-layout",
      "collapsed",
    )
    await again.click(screen.getByRole("button", { name: "Show the library" }))
    expect(screen.getByLabelText("Search the library")).toBeInTheDocument()
    expect(localStorage.getItem(LIBRARY_COLLAPSED_KEY)).toBe("false")
  })

  it("opens as an overlay in narrow windows", async () => {
    windowWidth(LIBRARY_OVERLAY_BELOW - 100)
    const user = renderDock()
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument()
    const toggle = screen.getByRole("button", { name: "Library" })
    expect(toggle).toHaveAttribute("aria-expanded", "false")
    await user.click(toggle)
    const overlay = screen.getByRole("complementary", { name: "Action Library" })
    expect(overlay).toHaveAttribute("data-layout", "overlay")
    expect(screen.getByLabelText("Search the library")).toBeInTheDocument()
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument()
    await user.click(toggle)
    await user.click(screen.getByRole("button", { name: "Close the library" }))
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument()
  })

  it("stays beside the editor from the breakpoint on", () => {
    windowWidth(LIBRARY_OVERLAY_BELOW)
    renderDock()
    expect(screen.getByRole("complementary", { name: "Action Library" })).toHaveAttribute(
      "data-layout",
      "sidebar",
    )
  })
})
