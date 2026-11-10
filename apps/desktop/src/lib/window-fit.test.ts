// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { coalescedReporter, naturalSize } from "./window-fit"

describe("coalescedReporter", () => {
  beforeEach(() => {
    vi.useFakeTimers()
    // Hidden windows get no animation frames; sizing must not need them.
    vi.stubGlobal("requestAnimationFrame", () => {
      throw new Error("animation frames are not available in hidden windows")
    })
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it("coalesces bursts and skips sub-pixel changes", () => {
    let size = { width: 400, height: 300 }
    const report = vi.fn()
    const reporter = coalescedReporter(() => size, report)
    // A burst of observer callbacks becomes one report.
    for (let i = 0; i < 10; i++) reporter.schedule()
    vi.advanceTimersByTime(100)
    expect(report).toHaveBeenCalledTimes(1)
    expect(report).toHaveBeenLastCalledWith({ width: 400, height: 300 })

    // Resizing the window makes the observer fire again with the same
    // content size: no report, so no feedback loop.
    size = { width: 400.4, height: 300.6 }
    reporter.schedule()
    vi.advanceTimersByTime(100)
    expect(report).toHaveBeenCalledTimes(1)

    // A validation error makes the content taller.
    size = { width: 400, height: 352 }
    reporter.schedule()
    vi.advanceTimersByTime(100)
    expect(report).toHaveBeenCalledTimes(2)

    reporter.dispose()
    size = { width: 500, height: 500 }
    reporter.schedule()
    vi.advanceTimersByTime(100)
    expect(report).toHaveBeenCalledTimes(2)
  })

  it("ignores empty measurements", () => {
    const report = vi.fn()
    const reporter = coalescedReporter(() => ({ width: 0, height: 0 }), report)
    reporter.schedule()
    vi.advanceTimersByTime(100)
    expect(report).not.toHaveBeenCalled()
  })
})

describe("naturalSize", () => {
  it("is the shell's border box, rounded up", () => {
    const root = document.createElement("section")
    root.getBoundingClientRect = () => ({ width: 380.2, height: 238.4 }) as DOMRect
    expect(naturalSize(root)).toEqual({ width: 381, height: 239 })
    // Content wider or taller than the box (overflow) still counts.
    Object.defineProperty(root, "scrollHeight", { value: 512 })
    expect(naturalSize(root).height).toBe(512)
  })
})
