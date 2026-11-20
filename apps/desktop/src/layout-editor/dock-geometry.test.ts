import { describe, expect, it } from "vitest"

import { MIN_SHOULDER, PANEL_WIDTH, dockGeometry, dockOutline } from "./dock-geometry"

describe("the expanded dock's geometry", () => {
  it("makes the panel the preferred width, centred under the toolbar", () => {
    expect(dockGeometry(200, 1000)).toEqual({ width: PANEL_WIDTH, offset: -108, shoulder: 108 })
  })

  it("stays within the room the central area has", () => {
    expect(dockGeometry(200, 300)).toEqual({ width: 300, offset: -50, shoulder: 50 })
  })

  it("joins straight when the shoulders would be too small", () => {
    expect(dockGeometry(200, 200 + 2 * MIN_SHOULDER - 2)).toEqual({
      width: 200,
      offset: 0,
      shoulder: 0,
    })
    // Never narrower than the toolbar, even without room.
    expect(dockGeometry(380, 100)).toEqual({ width: 380, offset: 0, shoulder: 0 })
  })

  it("keeps the shoulders symmetrical (an odd room loses a pixel)", () => {
    const g = dockGeometry(201, 331)
    expect(g.width - 201).toBe(2 * g.shoulder)
    expect(g.offset).toBe(-g.shoulder)
  })
})

describe("the dock's outline", () => {
  it("steps out at the toolbar's bottom with concave fillets", () => {
    const g = dockGeometry(200, 400)
    const d = dockOutline(g, 200, 30, 170, 8)
    // Starts on the toolbar's top edge, past its corner.
    expect(d.startsWith(`M ${100 + 0.5 + 8} 0.5`)).toBe(true)
    // Two concave fillets (sweep 0) at the shoulders, eight convex corners.
    expect(d.match(/ 0 0 0 /g)).toHaveLength(2)
    expect(d.match(/ 0 0 1 /g)).toHaveLength(6)
    // The shoulders' top edge is the panel's first pixel row.
    expect(d).toContain("H 391.5")
    expect(d.endsWith("Z")).toBe(true)
  })

  it("is one rounded rectangle for a straight join", () => {
    const d = dockOutline(dockGeometry(300, 310), 300, 30, 200, 8)
    expect(d.match(/ 0 0 0 /g)).toBeNull()
    expect(d.match(/ 0 0 1 /g)).toHaveLength(4)
  })
})
