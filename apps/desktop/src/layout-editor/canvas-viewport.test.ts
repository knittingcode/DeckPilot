import { describe, expect, it } from "vitest"

import { deviceGeometry, deviceSize } from "@/components/device-geometry"
import { DEVICE_MODELS } from "@/test/fixtures"

import {
  ACTUAL_KEY,
  FIT_MAX_KEY,
  PAN_MARGIN,
  VIEW_STORAGE_KEY,
  ZOOM_LIMITS,
  canvasSize,
  clampPan,
  clampZoom,
  TOOLBAR_OFFSET,
  deviceRect,
  dividerBar,
  dividerHalf,
  fitAvoiding,
  intersects,
  toolbarLift,
  toolbarRect,
  fitInsets,
  fitView,
  fitZoom,
  fromSaved,
  inRange,
  keySizeAt,
  loadView,
  panBy,
  previewPixels,
  revealDelta,
  saveView,
  snap,
  stepZoom,
  toSaved,
  visibleRange,
  wheelPan,
  wheelZoomFactor,
  zoomAt,
  zoomPercent,
} from "./canvas-viewport"

const viewport = { width: 900, height: 360 }
const insets = fitInsets(48)
const geometry = (name: keyof typeof DEVICE_MODELS) => deviceGeometry(DEVICE_MODELS[name])

function memoryStorage(): Storage {
  const data = new Map<string, string>()
  return {
    get length() {
      return data.size
    },
    clear: () => {
      data.clear()
    },
    getItem: (key) => data.get(key) ?? null,
    key: (index) => [...data.keys()][index] ?? null,
    removeItem: (key) => {
      data.delete(key)
    },
    setItem: (key, value) => {
      data.set(key, value)
    },
  }
}

describe("canvas zoom", () => {
  it("is 72 px keys at 100 %, within 10 % … 400 %", () => {
    expect(ACTUAL_KEY).toBe(72)
    expect(keySizeAt(1)).toBe(72)
    expect(keySizeAt(2)).toBe(144)
    expect(keySizeAt(0.01)).toBe(Math.round(72 * ZOOM_LIMITS.min))
    expect(keySizeAt(100)).toBe(72 * ZOOM_LIMITS.max)
    expect(zoomPercent(1.234)).toBe(123)
  })

  it("guards against NaN, Infinity and non-positive zooms", () => {
    for (const bad of [Number.NaN, Infinity, -Infinity, 0, -2]) {
      expect(clampZoom(bad)).toBe(1)
      expect(Number.isFinite(keySizeAt(bad))).toBe(true)
    }
    const view = zoomAt(
      { zoom: 1, x: 10, y: 10 },
      Number.NaN,
      { x: NaN, y: 1 },
      geometry("mk2"),
      viewport,
    )
    expect([view.zoom, view.x, view.y].every(Number.isFinite)).toBe(true)
    expect(Number.isFinite(wheelZoomFactor(Number.NaN))).toBe(true)
    const pan = panBy({ zoom: 1, x: 0, y: 0 }, Infinity, Number.NaN, geometry("mk2"), viewport)
    expect([pan.x, pan.y].every(Number.isFinite)).toBe(true)
  })

  it("steps through the presets and stops at the limits", () => {
    expect(stepZoom(1, 1)).toBe(1.25)
    expect(stepZoom(1, -1)).toBe(0.9)
    expect(stepZoom(1.1, 1)).toBe(1.25)
    expect(stepZoom(1.1, -1)).toBe(1)
    expect(stepZoom(4, 1)).toBe(4)
    expect(stepZoom(0.1, -1)).toBe(0.1)
  })

  it("zooms by a wheel notch or smoothly by a pinch, in either direction", () => {
    expect(wheelZoomFactor(-100)).toBeGreaterThan(1.3)
    expect(wheelZoomFactor(100)).toBeLessThan(0.75)
    expect(wheelZoomFactor(-2)).toBeCloseTo(1.016, 2)
    expect(wheelZoomFactor(0)).toBe(1)
  })

  it("keeps the point under the cursor in place", () => {
    const g = geometry("xl")
    const start = { zoom: 1, x: 40, y: 20 }
    const cursor = { x: 300, y: 120 }
    for (const next of [0.5, 1.5, 3]) {
      const view = zoomAt(start, next, cursor, g, { width: 4000, height: 4000 })
      // The device point under the cursor, in keys, before and after.
      const before = (cursor.x - start.x) / keySizeAt(start.zoom)
      const after = (cursor.x - view.x) / keySizeAt(view.zoom)
      expect(after).toBeCloseTo(before, 5)
      expect((cursor.y - view.y) / keySizeAt(view.zoom)).toBeCloseTo(
        (cursor.y - start.y) / keySizeAt(start.zoom),
        5,
      )
    }
  })
})

describe("drawn device size", () => {
  it("follows the device geometry, in whole-pixel steps like DeviceCanvas", () => {
    for (const name of ["mk2", "xl", "mini", "plus", "wide"] as const) {
      const g = geometry(name)
      const keys = deviceSize(g)
      const drawn = canvasSize(g, 72)
      // Within a few pixels of the proportional size.
      expect(Math.abs(drawn.width - keys.width * 72)).toBeLessThan(g.columns + 4)
      expect(Math.abs(drawn.height - keys.height * 72)).toBeLessThan(8)
    }
    // MK.2: 5 keys of 72 with 4 gaps of 14, and 6 of edge room on each side.
    expect(canvasSize(geometry("mk2"), 72).width).toBe(5 * 72 + 4 * 14 + 2 * 6)
  })
})

describe("fit to screen", () => {
  it("fits MK.2, XL, Mini and Plus inside the viewport less the insets, centred", () => {
    for (const name of ["mk2", "xl", "mini", "plus"] as const) {
      const g = geometry(name)
      const view = fitView(g, viewport, insets)
      const size = canvasSize(g, keySizeAt(view.zoom))
      expect(view.x).toBeGreaterThanOrEqual(insets.left)
      expect(view.y).toBeGreaterThanOrEqual(insets.top)
      expect(view.x + size.width).toBeLessThanOrEqual(viewport.width - insets.right)
      expect(view.y + size.height).toBeLessThanOrEqual(viewport.height - insets.bottom)
      // Centred in the free space (whole pixels).
      const free = viewport.width - insets.left - insets.right - size.width
      expect(Math.abs(view.x - insets.left - free / 2)).toBeLessThanOrEqual(1)
    }
  })

  it("is limited by the height for tall devices and by the width for wide ones", () => {
    // The same space: Plus (strip and dials) gets smaller keys than MK.2.
    expect(fitZoom(geometry("plus"), viewport, insets)).toBeLessThan(
      fitZoom(geometry("mk2"), viewport, insets),
    )
    const narrow = { width: 420, height: 600 }
    expect(fitZoom(geometry("xl"), narrow, insets)).toBeLessThan(
      fitZoom(geometry("mini"), narrow, insets),
    )
  })

  it("does not grow a small device past the earlier preview's largest key", () => {
    const huge = { width: 3000, height: 2000 }
    expect(keySizeAt(fitZoom(geometry("mini"), huge, insets))).toBe(FIT_MAX_KEY)
  })

  it("makes a 50 × 4 device small, within the zoom limits", () => {
    const g = geometry("wide")
    const zoom = fitZoom(g, viewport, insets)
    expect(zoom).toBeLessThan(0.3)
    expect(zoom).toBeGreaterThanOrEqual(ZOOM_LIMITS.min)
    const view = fitView(g, viewport, insets)
    expect(view.x + canvasSize(g, keySizeAt(zoom)).width).toBeLessThanOrEqual(viewport.width)
  })

  it("keeps clear of a reserved control area at every size and geometry", () => {
    const sizes = [
      { width: 320, height: 220 },
      { width: 600, height: 260 },
      { width: 900, height: 360 },
      { width: 1400, height: 520 },
    ]
    for (const name of ["mini", "mk2", "xl", "plus", "wide"] as const) {
      const g = geometry(name)
      for (const size of sizes) {
        for (const reserved of [0, 76]) {
          const view = fitView(g, size, fitInsets(reserved))
          const drawn = canvasSize(g, keySizeAt(view.zoom))
          const where = `${name} in ${size.width}×${size.height}, ${reserved} reserved`
          expect(view.y + drawn.height, where).toBeLessThanOrEqual(size.height - reserved)
          expect(view.y, where).toBeGreaterThanOrEqual(0)
          // Only the 10 % limit can make a device wider than its space
          // (the 50 × 4 device in a narrow window): then it is panned.
          if (view.zoom > ZOOM_LIMITS.min) {
            expect(view.x, where).toBeGreaterThanOrEqual(0)
            expect(view.x + drawn.width, where).toBeLessThanOrEqual(size.width)
          }
        }
      }
    }
  })

  it("does not break without a size (hidden, jsdom)", () => {
    expect(fitZoom(geometry("mk2"), { width: 0, height: 0 }, insets)).toBe(1)
    expect(fitZoom(geometry("mk2"), { width: NaN, height: 100 }, insets)).toBe(1)
  })
})

describe("the canvas/inspector divider", () => {
  it("puts the bar's centre on the line, centred across, half above and half below", () => {
    for (const bar of [
      { width: 260, height: 38 },
      { width: 420, height: 37 },
    ]) {
      const placed = dividerBar(bar, 900, 400)
      expect(400 - placed.top).toBeCloseTo(placed.bottom - 400)
      expect((placed.top + placed.bottom) / 2).toBe(400)
      expect(placed.left).toBeCloseTo(900 - placed.right)
    }
    expect(dividerHalf(38)).toBe(19)
    expect(dividerHalf(37)).toBe(19)
    expect(dividerHalf(Number.NaN)).toBe(0)
  })

  it("keeps the keys above the bar after Fit, for every geometry and size", () => {
    for (const name of ["mini", "mk2", "xl", "plus", "wide"] as const) {
      const g = geometry(name)
      for (const size of [
        { width: 480, height: 240 },
        { width: 900, height: 360 },
        { width: 1600, height: 520 },
      ]) {
        const bar = dividerBar({ width: 300, height: 38 }, size.width, size.height)
        const view = fitView(g, size, fitInsets(dividerHalf(38)))
        const drawn = canvasSize(g, keySizeAt(view.zoom))
        // The device ends above the bar's upper half (the viewport ends at the line).
        expect(view.y + drawn.height, `${name} ${size.width}×${size.height}`).toBeLessThanOrEqual(
          bar.top,
        )
      }
    }
  })
})

describe("the zoom toolbar", () => {
  const bar = { width: 150, height: 38 }
  const tool = { width: 170, height: 36 }

  it("sits in the bottom-right corner, as far from the right as from the divider", () => {
    for (const size of [
      { width: 600, height: 220 },
      { width: 900, height: 360 },
      { width: 1600, height: 520 },
    ]) {
      const rect = toolbarRect(tool, size, bar)
      expect(size.width - rect.right).toBe(TOOLBAR_OFFSET)
      expect(size.height - rect.bottom).toBe(TOOLBAR_OFFSET)
      // The viewport ends at the divider: the page navigation's upper half
      // is the strip of `half` above it, centred.
      const nav = dividerBar(bar, size.width, size.height)
      expect(intersects(rect, nav)).toBe(false)
      expect((nav.top + nav.bottom) / 2).toBe(size.height)
      // Nothing of it below the divider (the inspector).
      expect(rect.bottom).toBeLessThan(size.height)
    }
  })

  it("moves up just enough only where the page navigation would be under it", () => {
    const narrow = { width: 300, height: 220 }
    const rect = toolbarRect(tool, narrow, bar)
    const nav = dividerBar(bar, narrow.width, narrow.height)
    expect(intersects(rect, nav)).toBe(false)
    expect(nav.top - rect.bottom).toBeLessThanOrEqual(TOOLBAR_OFFSET / 4 + 1)
    expect(toolbarLift(tool, { width: 900, height: 300 }, bar)).toBe(0)
    expect(toolbarLift(tool, narrow, null)).toBe(0)
  })

  it("is avoided by Fit to screen only when it would cover keys", () => {
    const half = dividerHalf(bar.height)
    for (const name of ["mini", "mk2", "xl", "plus", "wide"] as const) {
      const g = geometry(name)
      for (const size of [
        { width: 480, height: 240 },
        { width: 700, height: 300 },
        { width: 900, height: 360 },
        { width: 1600, height: 520 },
      ]) {
        const exclusion = toolbarRect(tool, size, bar)
        const insets = fitInsets(half)
        const view = fitAvoiding(g, size, insets, exclusion)
        const where = `${name} ${size.width}×${size.height}`
        if (view.zoom > ZOOM_LIMITS.min)
          expect(intersects(deviceRect(g, view), exclusion), where).toBe(false)
        expect(
          intersects(deviceRect(g, view), dividerBar(bar, size.width, size.height)),
          where,
        ).toBe(false)
        const plain = fitView(g, size, insets)
        if (!intersects(deviceRect(g, plain), exclusion)) expect(view, where).toEqual(plain)
        else expect(view.zoom, where).toBeLessThanOrEqual(plain.zoom)
      }
    }
  })
})

describe("pan bounds", () => {
  it("keeps at least a margin of the device in view", () => {
    const g = geometry("mk2")
    const size = canvasSize(g, 72)
    const far = clampPan({ zoom: 1, x: 10_000, y: -10_000 }, g, viewport)
    expect(far.x).toBe(viewport.width - PAN_MARGIN)
    expect(far.y).toBe(PAN_MARGIN - size.height)
    const back = clampPan({ zoom: 1, x: -10_000, y: 10_000 }, g, viewport)
    expect(back.x + size.width).toBe(PAN_MARGIN)
    expect(back.y).toBe(viewport.height - PAN_MARGIN)
  })

  it("lets a zoomed-in 50 × 4 device be panned along its length", () => {
    const g = geometry("wide")
    const start = { zoom: 1, x: 0, y: 0 }
    const moved = panBy(start, -2000, 0, g, viewport)
    expect(moved.x).toBe(-2000)
    const end = panBy(moved, -100_000, 0, g, viewport)
    expect(end.x + canvasSize(g, 72).width).toBe(PAN_MARGIN)
  })

  it("brings a focused key into view", () => {
    expect(revealDelta({ left: 20, top: 20, right: 90, bottom: 90 }, viewport)).toEqual({
      dx: 0,
      dy: 0,
    })
    expect(revealDelta({ left: 950, top: 20, right: 1020, bottom: 90 }, viewport).dx).toBe(
      viewport.width - 12 - 1020,
    )
    expect(revealDelta({ left: -200, top: 20, right: -130, bottom: 90 }, viewport).dx).toBe(212)
    // Above the page navigation at the bottom.
    expect(revealDelta({ left: 10, top: 300, right: 80, bottom: 350 }, viewport, 12, 60).dy).toBe(
      viewport.height - 60 - 12 - 350,
    )
  })
})

describe("wheel", () => {
  it("pans against the scroll, lines in pixels, Shift sideways", () => {
    expect(wheelPan({ deltaX: 5, deltaY: 10, deltaMode: 0, shiftKey: false }, 400)).toEqual({
      dx: -5,
      dy: -10,
    })
    expect(wheelPan({ deltaX: 0, deltaY: 3, deltaMode: 1, shiftKey: false }, 400).dy).toBe(-48)
    expect(wheelPan({ deltaX: 0, deltaY: 30, deltaMode: 0, shiftKey: true }, 400)).toEqual({
      dx: -30,
      dy: -0,
    })
  })
})

describe("culling", () => {
  it("finds the rows and columns in view, with one of slack", () => {
    const g = geometry("wide")
    const range = visibleRange({ zoom: 1, x: -860, y: 0 }, g, viewport)
    // 86 px per column at 100 % (and 6 of edge room): the view starts in
    // column 9, one more of slack.
    expect(range.columns[0]).toBe(8)
    expect(range.columns[1]).toBeLessThan(25)
    expect(range.rows).toEqual([0, 3])
    expect(inRange(range, 10, 50)).toBe(true)
    expect(inRange(range, 0, 50)).toBe(false)
    expect(inRange(range, 49 + 50 * 3, 50)).toBe(false)
    expect(inRange(null, 0, 50)).toBe(true)
  })
})

describe("whole pixels", () => {
  it("snaps translations to device pixels and preview sizes to the renderer's limits", () => {
    expect(snap(10.3, 2)).toBe(10.5)
    expect(snap(10.3, 1)).toBe(10)
    expect(snap(Number.NaN, 2)).toBe(0)
    expect(previewPixels(4)).toBe(16)
    expect(previewPixels(2000)).toBe(512)
    expect(previewPixels(Number.NaN)).toBe(16)
    expect(previewPixels(131.6)).toBe(132)
  })
})

describe("remembered view", () => {
  it("is kept per device model, versioned, and survives another window size", () => {
    const store = memoryStorage()
    const g = geometry("xl")
    const view = { zoom: 1.5, x: -120, y: 30 }
    saveView("xl", toSaved(view, viewport, false), store)
    saveView("mk2", { mode: "fit" }, store)
    const stored = JSON.parse(store.getItem(VIEW_STORAGE_KEY) ?? "{}") as Record<string, unknown>
    expect(Object.keys(stored)).toEqual(["xl", "mk2"])
    const saved = loadView("xl", store)
    expect(saved?.mode).toBe("manual")
    expect(loadView("mk2", store)).toEqual({ mode: "fit" })
    expect(loadView("mini", store)).toBeNull()
    if (saved?.mode !== "manual") throw new Error("not manual")
    // Same window: the same place.
    const same = fromSaved(saved, g, viewport)
    expect(same.zoom).toBe(1.5)
    expect(same.x).toBeCloseTo(view.x)
    expect(same.y).toBeCloseTo(view.y)
    // A wider window: the same device point in the middle.
    const wider = fromSaved(saved, g, { width: 1300, height: 360 })
    expect(wider.x).toBeCloseTo(view.x + 200)
  })

  it("ignores broken or foreign values and failing storage", () => {
    const store = memoryStorage()
    store.setItem(VIEW_STORAGE_KEY, "{not json")
    expect(loadView("xl", store)).toBeNull()
    store.setItem(VIEW_STORAGE_KEY, JSON.stringify({ xl: { mode: "manual", zoom: "big" } }))
    expect(loadView("xl", store)).toBeNull()
    store.setItem(
      VIEW_STORAGE_KEY,
      JSON.stringify({ xl: { mode: "manual", zoom: 99, cx: 1, cy: 2 } }),
    )
    expect(loadView("xl", store)).toEqual({ mode: "manual", zoom: ZOOM_LIMITS.max, cx: 1, cy: 2 })
    const failing = {
      length: 0,
      clear: () => undefined,
      key: () => null,
      removeItem: () => undefined,
      getItem: () => {
        throw new Error("denied")
      },
      setItem: () => {
        throw new Error("quota")
      },
    } as Storage
    expect(loadView("xl", failing)).toBeNull()
    expect(() => {
      saveView("xl", { mode: "fit" }, failing)
    }).not.toThrow()
  })
})
