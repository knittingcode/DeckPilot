// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest"

import { defaultPanels, loadPanels, loadViewport, savePanels, saveViewport } from "./view-state"

describe("editor view state", () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it("remembers the viewport per workflow", () => {
    expect(loadViewport("w1")).toBeNull()
    saveViewport("w1", { x: 10, y: -20, zoom: 0.8 })
    expect(loadViewport("w1")).toEqual({ x: 10, y: -20, zoom: 0.8 })
    expect(loadViewport("w2")).toBeNull()
  })

  it("ignores stored values that are not a viewport", () => {
    localStorage.setItem("deckpilot.workflow-viewport.w1", JSON.stringify({ x: 1, zoom: 0 }))
    expect(loadViewport("w1")).toBeNull()
    localStorage.setItem("deckpilot.workflow-viewport.w1", "{not json")
    expect(loadViewport("w1")).toBeNull()
  })

  it("starts compact in narrow windows and remembers the choice", () => {
    expect(defaultPanels(1100)).toEqual({ catalog: false, inspector: true })
    expect(defaultPanels(1600)).toEqual({ catalog: true, inspector: true })
    expect(loadPanels(1100).catalog).toBe(false)
    savePanels({ catalog: true, inspector: false })
    expect(loadPanels(1100)).toEqual({ catalog: true, inspector: false })
  })
})
