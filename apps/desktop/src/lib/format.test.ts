import { describe, expect, it } from "vitest"

import { describeLayout, hex } from "./format"

describe("describeLayout", () => {
  it("describes every layout status", () => {
    expect(describeLayout({ type: "missing" })).toBe("No layout for this model")
    expect(describeLayout({ type: "noProfile" })).toBe("No active profile")
    expect(describeLayout({ type: "compatible", layout: "1", name: "Main" })).toBe("Layout “Main”")
    expect(describeLayout({ type: "invalid", layout: "1", name: "Main", issues: ["x"] })).toBe(
      "Layout “Main” has problems",
    )
  })
})

describe("hex", () => {
  it("formats USB identifiers", () => {
    expect(hex(0x0fd9)).toBe("0x0fd9")
    expect(hex(0x80)).toBe("0x0080")
  })
})
