// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { coalescedReporter, naturalSize } from "./window-fit"

describe("coalescedReporter", () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) =>
      setTimeout(() => {
        callback(0)
      }, 16),
    )
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
  it("uses the full body height, not the visible part", () => {
    document.body.innerHTML = `
      <section data-modal-root>
        <header data-modal-part="header"></header>
        <div data-modal-part="body"></div>
        <footer data-modal-part="footer"></footer>
      </section>`
    const root = document.querySelector<HTMLElement>("[data-modal-root]")
    if (!root) throw new Error("fixture missing")
    const set = (selector: string, property: string, value: number) => {
      Object.defineProperty(root.querySelector(selector), property, { value, configurable: true })
    }
    set('[data-modal-part="header"]', "offsetHeight", 72)
    // The body scrolls: 200 px visible, 900 px of content.
    set('[data-modal-part="body"]', "offsetHeight", 200)
    set('[data-modal-part="body"]', "scrollHeight", 900)
    set('[data-modal-part="footer"]', "offsetHeight", 64)
    root.getBoundingClientRect = () => ({ width: 380.2 }) as DOMRect
    expect(naturalSize(root)).toEqual({ width: 381, height: 1036 })
  })
})
