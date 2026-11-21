import { describe, expect, it } from "vitest"

import type { BindingTarget, DeviceLayout, Workflow } from "@/lib/types"

import {
  decodeTarget,
  encodeTarget,
  readOnlyTarget,
  targetGroups,
  unavailableTarget,
} from "./targets"

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

describe("workflow targets without the Workflows plugin", () => {
  const layout = {
    id: "l",
    name: "L",
    model: "m",
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings: [] }],
  } as DeviceLayout
  const workflows = [
    { id: "graph", name: "Graph", steps: [], graph: { version: 1, nodes: [], edges: [] } },
    { id: "multi", name: "Multi Action", steps: [] },
  ] as unknown as Workflow[]

  it("keeps graph workflows listed, read-only, and Multi Actions available", () => {
    const off = targetGroups({ actions: [], workflows }, layout, "home", () => null, false)
    expect(unavailableTarget(off, "workflow:graph")).toBe("the Workflows plugin is off")
    expect(readOnlyTarget(off, "workflow:graph")).toBe(true)
    expect(unavailableTarget(off, "workflow:multi")).toBeUndefined()
    expect(readOnlyTarget(off, "workflow:multi")).toBe(false)
    const on = targetGroups({ actions: [], workflows }, layout, "home")
    expect(unavailableTarget(on, "workflow:graph")).toBeUndefined()
    expect(readOnlyTarget(on, "workflow:graph")).toBe(false)
  })
})
