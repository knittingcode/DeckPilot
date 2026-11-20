// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { DndContext, PointerSensor, useSensor, useSensors } from "@dnd-kit/core"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { DeviceCanvas } from "@/components/device-canvas"
import { deviceGeometry } from "@/components/device-geometry"
import { i18n } from "@/i18n"
import type { ControlId, DeviceModel } from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { DEVICE_MODELS } from "@/test/fixtures"

import type { SelectMode } from "./editing"
import {
  ARROW_PAN,
  VIEW_STORAGE_KEY,
  fitInsets,
  fitView,
  keySizeAt,
  stepZoom,
  zoomPercent,
} from "./canvas-viewport"
import { DeviceViewport, SETTLE_MS } from "./device-viewport"

const SIZE = { width: 900, height: 360 }
const descriptors = {
  clientWidth: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth"),
  clientHeight: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight"),
}

beforeAll(() => {
  installDomStubs()
  // jsdom does not lay out: the canvas viewport gets a size of its own.
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get(this: HTMLElement) {
      return this.dataset.slot === "device-viewport" ? SIZE.width : 0
    },
  })
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(this: HTMLElement) {
      return this.dataset.slot === "device-viewport" ? SIZE.height : 0
    },
  })
})
afterAll(() => {
  for (const [name, descriptor] of Object.entries(descriptors))
    if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor)
})
beforeEach(() => {
  localStorage.clear()
})
afterEach(() => {
  cleanup()
})

function Harness({
  model,
  onSelect,
  onOpen,
  onSettled,
}: {
  model: DeviceModel
  onSelect: (control: ControlId, mode: SelectMode) => void
  onOpen: (control: ControlId) => void
  onSettled: (keySize: number) => void
}) {
  const [selection, setSelection] = useState<ControlId[]>([])
  // As the editor's: a drag starts after 4 px, so a click stays a click.
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }))
  return (
    <DndContext sensors={sensors}>
      <DeviceViewport
        model={model}
        onKeySizeSettled={onSettled}
        footer={<button type="button">Page navigation</button>}
        renderCanvas={(keySize, visible) => (
          <DeviceCanvas
            model={model}
            keySize={keySize}
            visible={visible}
            selection={selection}
            focused={selection[0] ?? null}
            onSelect={(control, mode) => {
              onSelect(control, mode)
              setSelection([control])
            }}
            onOpen={onOpen}
            droppable
            draggable
          />
        )}
      />
    </DndContext>
  )
}

function setup(model: DeviceModel = DEVICE_MODELS.mk2) {
  const onSelect = vi.fn()
  const onOpen = vi.fn()
  const onSettled = vi.fn()
  const user = userEvent.setup()
  const result = render(
    <Harness model={model} onSelect={onSelect} onOpen={onOpen} onSettled={onSettled} />,
  )
  const viewport = document.querySelector("[data-slot=device-viewport]") as HTMLElement
  const stage = document.querySelector("[data-slot=device-stage]") as HTMLElement
  const grid = document.querySelector("[data-slot=device-grid]") as HTMLElement
  return { user, onSelect, onOpen, onSettled, viewport, stage, grid, ...result }
}

/** The translation of the device. */
function position(grid: HTMLElement) {
  const match = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(grid.style.transform)
  if (!match) throw new Error(`no translation in "${grid.style.transform}"`)
  return { x: Number(match[1]), y: Number(match[2]) }
}
const zoomOf = (viewport: HTMLElement) => Number(viewport.dataset.zoom)
const key = (name: string) => screen.getByRole("button", { name })

describe("the device canvas", () => {
  it("opens fitted and centred, and asks for key images of that size at once", () => {
    const { viewport, grid, onSettled } = setup()
    const expected = fitView(deviceGeometry(DEVICE_MODELS.mk2), SIZE, fitInsets(0))
    expect(zoomOf(viewport)).toBe(zoomPercent(expected.zoom))
    expect(position(grid)).toEqual({ x: expected.x, y: expected.y })
    expect(onSettled).toHaveBeenCalledWith(keySizeAt(expected.zoom))
    // The keys are laid out at that size (no CSS scaling of the device).
    const canvas = screen.getByRole("group", { name: "Stream Deck MK.2 controls" })
    expect(canvas.style.getPropertyValue("--key")).toBe(`${keySizeAt(expected.zoom)}px`)
    expect(grid.style.transform).not.toContain("scale")
  })

  it("pans with the wheel and zooms with Ctrl+wheel around the pointer", async () => {
    const { viewport, grid } = setup()
    const start = position(grid)
    fireEvent.wheel(viewport, { deltaX: 10, deltaY: 30 })
    await waitFor(() => {
      expect(position(grid)).toEqual({ x: start.x - 10, y: start.y - 30 })
    })
    const before = { ...position(grid), key: keySizeAt(zoomOf(viewport) / 100) }
    const cursor = { clientX: 300, clientY: 150 }
    fireEvent.wheel(viewport, { deltaY: -100, ctrlKey: true, ...cursor })
    await waitFor(() => {
      expect(zoomOf(viewport)).toBeGreaterThan(100 * (before.key / 72))
    })
    const key = Number(viewport.dataset.keySize)
    const after = position(grid)
    // The device point under the pointer stayed there (to a pixel).
    expect(
      Math.abs((cursor.clientX - after.x) / key - (cursor.clientX - before.x) / before.key),
    ).toBeLessThan(0.02)
    expect(
      Math.abs((cursor.clientY - after.y) / key - (cursor.clientY - before.y) / before.key),
    ).toBeLessThan(0.02)
  })

  it("zooms with a Safari pinch (gesture events)", async () => {
    const { viewport } = setup()
    const before = zoomOf(viewport)
    act(() => {
      viewport.dispatchEvent(new Event("gesturestart", { cancelable: true }))
      viewport.dispatchEvent(
        Object.assign(new Event("gesturechange", { cancelable: true }), {
          scale: 2,
          clientX: 450,
          clientY: 180,
        }),
      )
      viewport.dispatchEvent(new Event("gestureend", { cancelable: true }))
    })
    await waitFor(() => {
      expect(Math.abs(zoomOf(viewport) - before * 2)).toBeLessThanOrEqual(1)
    })
  })

  it("zooms with + - 0 1 and pans with the arrows when the canvas is focused", async () => {
    const { viewport, grid, user } = setup()
    const fitted = zoomOf(viewport)
    viewport.focus()
    await user.keyboard("0")
    expect(zoomOf(viewport)).toBe(100)
    await user.keyboard("+")
    expect(zoomOf(viewport)).toBe(stepZoom(1, 1) * 100)
    await user.keyboard("-")
    await user.keyboard("-")
    expect(zoomOf(viewport)).toBe(stepZoom(1, -1) * 100)
    await user.keyboard("1")
    expect(zoomOf(viewport)).toBe(fitted)
    const start = position(grid)
    await user.keyboard("{ArrowRight}")
    await waitFor(() => {
      expect(position(grid).x).toBe(start.x - ARROW_PAN)
    })
    await user.keyboard("{Shift>}{ArrowDown}{/Shift}")
    await waitFor(() => {
      expect(position(grid).y).toBe(start.y - ARROW_PAN * 4)
    })
  })

  it("moves the selection, not the view, with the arrows on a key", async () => {
    const { grid, user, onSelect } = setup()
    await user.click(key("Key 1"))
    const start = position(grid)
    await user.keyboard("{ArrowRight}")
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "key", index: 1 }, "only")
    expect(key("Key 2")).toHaveFocus()
    expect(position(grid)).toEqual(start)
  })

  it("pans by dragging the background, without selecting anything", async () => {
    const { stage, grid, onSelect } = setup()
    const start = position(grid)
    fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 100, clientY: 100 })
    fireEvent.pointerMove(stage, { pointerId: 1, clientX: 160, clientY: 80 })
    fireEvent.pointerUp(stage, { pointerId: 1, clientX: 160, clientY: 80 })
    await waitFor(() => {
      expect(position(grid)).toEqual({ x: start.x + 60, y: start.y - 20 })
    })
    expect(onSelect).not.toHaveBeenCalled()
  })

  it("keeps panning and fitting when an animation frame never runs (a hidden window)", () => {
    // WebKit runs no animation frames while a window is hidden — the main
    // window starts hidden until the configuration is unlocked. A pending
    // frame that never ran used to block every later move: the device could
    // no longer be dragged or fitted back (owner report, November 2026).
    vi.stubGlobal("requestAnimationFrame", () => 1)
    vi.stubGlobal("cancelAnimationFrame", () => undefined)
    try {
      const { stage, grid, viewport } = setup()
      const start = position(grid)
      const moves: [number, number][] = [
        [60, -20],
        [-200, 150],
      ]
      for (const [dx, dy] of moves) {
        const from = position(grid)
        fireEvent.pointerDown(stage, { button: 0, pointerId: 7, clientX: 100, clientY: 100 })
        fireEvent.pointerMove(stage, { pointerId: 7, clientX: 100 + dx, clientY: 100 + dy })
        fireEvent.pointerUp(stage, { pointerId: 7, clientX: 100 + dx, clientY: 100 + dy })
        // Applied at once, 1:1, without waiting for a frame.
        expect(position(grid)).toEqual({ x: from.x + dx, y: from.y + dy })
      }
      expect(position(grid)).not.toEqual(start)
      // Fit brings it back where it opened (fitted and centred).
      fireEvent.keyDown(viewport, { key: "1" })
      expect(position(grid)).toEqual(start)
    } finally {
      vi.unstubAllGlobals()
    }
  })

  it("pans with Space+drag and the middle button over a key, which stays unselected", async () => {
    const { viewport, grid, onSelect } = setup()
    const start = position(grid)
    fireEvent.pointerEnter(viewport)
    fireEvent.keyDown(viewport, { key: " ", code: "Space" })
    expect(viewport).toHaveAttribute("data-space")
    const target = key("Key 3")
    fireEvent.pointerDown(target, { button: 0, pointerId: 2, clientX: 200, clientY: 100 })
    fireEvent.pointerMove(target, { pointerId: 2, clientX: 240, clientY: 100 })
    fireEvent.pointerUp(target, { pointerId: 2, clientX: 240, clientY: 100 })
    fireEvent.click(target)
    fireEvent.keyUp(viewport, { key: " ", code: "Space" })
    await waitFor(() => {
      expect(position(grid).x).toBe(start.x + 40)
    })
    // Middle button: also over a key.
    fireEvent.pointerDown(target, { button: 1, pointerId: 3, clientX: 200, clientY: 100 })
    fireEvent.pointerMove(target, { pointerId: 3, clientX: 200, clientY: 130 })
    fireEvent.pointerUp(target, { pointerId: 3, clientX: 200, clientY: 130 })
    await waitFor(() => {
      expect(position(grid).y).toBe(start.y + 30)
    })
    expect(onSelect).not.toHaveBeenCalled()
    expect(target).toHaveAttribute("aria-pressed", "false")
  })

  it("shows no focus ring after a pointer focus, but does for the keyboard", async () => {
    const { viewport, stage, user } = setup()
    // The ring is only for keyboard focus, never an unconditional one.
    expect(viewport.className).toContain("focus-visible:not-data-[pointer-focus]:ring-2")
    expect(viewport.className).not.toMatch(/(^|\s)(focus:|focus-visible:)?ring-2(\s|$)/)
    expect(viewport.className).toContain("outline-none")
    fireEvent.pointerDown(stage, { button: 0, pointerId: 7, clientX: 10, clientY: 10 })
    fireEvent.pointerUp(stage, { pointerId: 7, clientX: 10, clientY: 10 })
    expect(viewport).toHaveFocus()
    expect(viewport).toHaveAttribute("data-pointer-focus")
    await user.keyboard("{Shift}")
    expect(viewport).not.toHaveAttribute("data-pointer-focus")
    // Tab away and back: a keyboard focus.
    fireEvent.pointerDown(stage, { button: 0, pointerId: 8, clientX: 10, clientY: 10 })
    fireEvent.pointerUp(stage, { pointerId: 8 })
    await user.tab()
    expect(viewport).not.toHaveAttribute("data-pointer-focus")
  })

  it("ends a pan on pointercancel and window blur, with nothing stuck", async () => {
    const { viewport, stage, grid } = setup()
    fireEvent.pointerDown(stage, { button: 0, pointerId: 4, clientX: 100, clientY: 100 })
    expect(viewport).toHaveAttribute("data-panning")
    fireEvent.pointerCancel(stage, { pointerId: 4 })
    expect(viewport).not.toHaveAttribute("data-panning")
    const after = position(grid)
    fireEvent.pointerMove(stage, { pointerId: 4, clientX: 300, clientY: 300 })
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(position(grid)).toEqual(after)
    fireEvent.pointerEnter(viewport)
    fireEvent.keyDown(viewport, { key: " ", code: "Space" })
    fireEvent.pointerDown(stage, { button: 0, pointerId: 5, clientX: 100, clientY: 100 })
    act(() => {
      window.dispatchEvent(new Event("blur"))
    })
    expect(viewport).not.toHaveAttribute("data-panning")
    expect(viewport).not.toHaveAttribute("data-space")
  })

  it("leaves the context menu and right clicks of the keys alone", () => {
    const { onSelect } = setup()
    expect(fireEvent.pointerDown(key("Key 2"), { button: 2, pointerId: 9 })).toBe(true)
    expect(fireEvent.contextMenu(key("Key 2"))).toBe(true)
    expect(onSelect).not.toHaveBeenCalled()
  })

  it("keeps the page navigation and the toolbar out of the moved device's layer", () => {
    const { viewport } = setup()
    const nav = screen.getByText("Page navigation")
    const toolbar = screen.getByRole("toolbar", { name: "Zoom" })
    expect(viewport.contains(nav)).toBe(false)
    expect(viewport.contains(toolbar)).toBe(false)
    // The page navigation on the divider after the canvas region; the zoom
    // toolbar in the canvas region's corner: neither in the moved layer.
    const divider = document.querySelector("[data-slot=canvas-divider]")
    expect(divider?.contains(nav)).toBe(true)
    const region = document.querySelector("[data-slot=device-workspace]")
    expect(region?.nextElementSibling).toBe(divider)
    expect(region?.contains(nav)).toBe(false)
    expect(region?.contains(toolbar)).toBe(true)
    expect(document.querySelector("[data-slot=device-grid]")?.contains(toolbar)).toBe(false)
    // One inset token for the right and the bottom edge.
    const corner = toolbar.parentElement as HTMLElement
    expect(corner).toHaveClass(
      "absolute",
      "right-(--canvas-overlay-inset)",
      "bottom-(--canvas-overlay-inset)",
    )
    expect(corner.style.bottom).toBe("")
    // A translucent background (not opacity), opaque on hover and focus.
    expect(toolbar).toHaveClass("bg-popover/65", "hover:bg-popover", "focus-within:bg-popover")
    expect(toolbar.className).not.toMatch(/(^|\s)opacity-/)
    expect(toolbar.className).toContain("motion-safe:transition-[background-color]")
    // Half of the bar is reserved below it, for the inspector.
    expect(
      (divider?.parentElement as HTMLElement).style.getPropertyValue("--canvas-divider-half"),
    ).toBe("0px")
  })

  it("keeps selecting with a click, Shift-click and opening with a double-click", async () => {
    const { user, onSelect, onOpen } = setup()
    await user.click(key("Key 1"))
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "key", index: 0 }, "only")
    await user.keyboard("{Shift>}")
    await user.click(key("Key 4"))
    await user.keyboard("{/Shift}")
    expect(onSelect).toHaveBeenLastCalledWith({ kind: "key", index: 3 }, "extend")
    await user.dblClick(key("Key 5"))
    expect(onOpen).toHaveBeenCalledWith({ kind: "key", index: 4 })
  })

  it("pans a key focused outside the view into it", async () => {
    const { grid, user, viewport } = setup(DEVICE_MODELS.wide)
    await user.click(key("Key 1"))
    fireEvent.keyDown(viewport, { key: "0" })
    const start = position(grid)
    const far = key("Key 50")
    far.getBoundingClientRect = () =>
      ({
        left: 2000,
        right: 2072,
        top: 40,
        bottom: 112,
        width: 72,
        height: 72,
        x: 2000,
        y: 40,
      }) as DOMRect
    // Not right after a click: as from the keyboard.
    await new Promise((resolve) => setTimeout(resolve, 600))
    act(() => {
      far.focus()
    })
    await waitFor(() => {
      expect(position(grid).x).toBe(start.x - (2072 - (SIZE.width - 12)))
    })
  })

  it("draws the keys of a large grid outside the view as placeholders that stay focusable", async () => {
    const { viewport, user } = setup(DEVICE_MODELS.wide)
    viewport.focus()
    await user.keyboard("0")
    const keys = [
      ...document.querySelectorAll<HTMLElement>("[data-slot=device-keys] [data-control]"),
    ]
    expect(keys).toHaveLength(200)
    await waitFor(() => {
      // At 100 %, about 11 of the 50 columns are in view.
      const drawn = keys.filter((k) => k.childElementCount > 0)
      expect(drawn.length).toBeGreaterThan(0)
      expect(drawn.length).toBeLessThan(100)
    })
    for (const k of keys) expect(k).toHaveAccessibleName()
    expect(keys.at(-1)?.childElementCount).toBe(0)
  })

  it("asks for key images at a new size only once the zoom rests", async () => {
    const { viewport, onSettled } = setup()
    onSettled.mockClear()
    fireEvent.keyDown(viewport, { key: "0" })
    fireEvent.keyDown(viewport, { key: "+" })
    expect(onSettled).not.toHaveBeenCalled()
    await waitFor(
      () => {
        expect(onSettled).toHaveBeenCalledTimes(1)
      },
      { timeout: SETTLE_MS * 4 },
    )
    expect(onSettled).toHaveBeenCalledWith(keySizeAt(stepZoom(1, 1)))
  })

  it("remembers the view per device model", async () => {
    const first = setup()
    first.viewport.focus()
    fireEvent.keyDown(first.viewport, { key: "0" })
    fireEvent.wheel(first.viewport, { deltaX: 0, deltaY: 50 })
    await waitFor(() => {
      expect(localStorage.getItem(VIEW_STORAGE_KEY)).toContain(DEVICE_MODELS.mk2.id)
    })
    const place = position(first.grid)
    first.unmount()
    const again = setup()
    expect(zoomOf(again.viewport)).toBe(100)
    expect(position(again.grid).x).toBeCloseTo(place.x, 0)
    expect(position(again.grid).y).toBeCloseTo(place.y, 0)
    again.unmount()
    // Another model: its own view (fitted).
    const xl = setup(DEVICE_MODELS.xl)
    const fitted = fitView(deviceGeometry(DEVICE_MODELS.xl), SIZE, fitInsets(0))
    expect(zoomOf(xl.viewport)).toBe(zoomPercent(fitted.zoom))
  })

  it("has a zoom toolbar with labels, a percentage menu and limits", async () => {
    const { viewport, user } = setup()
    const toolbar = screen.getByRole("toolbar", { name: "Zoom" })
    expect(toolbar).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Zoom in" }))
    const zoomed = zoomOf(viewport)
    await user.click(screen.getByRole("button", { name: `Zoom ${zoomed}%` }))
    await user.click(await screen.findByRole("menuitem", { name: /Actual size \(100%\)/ }))
    expect(zoomOf(viewport)).toBe(100)
    await user.click(screen.getByRole("button", { name: "Zoom out" }))
    expect(zoomOf(viewport)).toBe(90)
    for (let i = 0; i < 20; i++) fireEvent.keyDown(viewport, { key: "+" })
    expect(zoomOf(viewport)).toBe(400)
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeDisabled()
    await user.click(screen.getByRole("button", { name: "Fit to screen" }))
    expect(screen.getByRole("button", { name: "Fit to screen" })).toHaveAttribute(
      "title",
      "Fit to screen (1)",
    )
    // The page navigation is not inside the moved device.
    expect(
      document
        .querySelector("[data-slot=device-grid]")
        ?.contains(screen.getByText("Page navigation")),
    ).toBe(false)
  })

  it("is translated into Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    try {
      setup()
      expect(screen.getByRole("toolbar", { name: "Powiększenie" })).toBeInTheDocument()
      expect(screen.getByRole("button", { name: "Powiększ" })).toBeInTheDocument()
      expect(screen.getByRole("button", { name: "Pomniejsz" })).toBeInTheDocument()
      expect(screen.getByRole("button", { name: "Dopasuj do ekranu" })).toBeInTheDocument()
      expect(screen.getByRole("group", { name: "Obszar urządzenia" })).toBeInTheDocument()
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    }
  })
})
