import { describe, expect, it } from "vitest"

import type { BindingTarget } from "@/lib/types"

import { decodeTarget, encodeTarget } from "./targets"

describe("binding target values", () => {
  it("round-trips every target kind, including a work session key", () => {
    const targets: BindingTarget[] = [
      { type: "action", action: "a1" },
      { type: "workflow", workflow: "w1" },
      { type: "openPage", page: "p1" },
      { type: "back" },
      { type: "stateful", machine: "workSession" },
    ]
    for (const target of targets) {
      expect(decodeTarget(encodeTarget(target))).toEqual(target)
    }
    expect(encodeTarget(undefined)).toBe("none")
  })

  it("refuses machines it does not know", () => {
    expect(decodeTarget("stateful:script")).toBeNull()
  })
})
