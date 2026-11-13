import { describe, expect, it } from "vitest"

import {
  addPage,
  ancestors,
  bindingsFor,
  controlsOf,
  defaultTrigger,
  deletePage,
  hasWayBack,
  pageParents,
  removeBinding,
  setBinding,
  supportedTriggers,
} from "./layout-model"
import type { ControlBinding, DeviceLayout, DeviceModel } from "./types"

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: {
    keys: { rows: 3, columns: 5, image: { width: 72, height: 72, format: "jpeg" } },
    encoders: null,
    touch: null,
  },
  support: "unverified",
  verification: [],
}

const xl: DeviceModel = {
  ...mk2,
  id: "elgato.streamdeck.xl",
  capabilities: { ...mk2.capabilities, keys: { rows: 4, columns: 8, image: null } },
}

const plus: DeviceModel = {
  ...mk2,
  id: "elgato.streamdeck.plus",
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: true },
    touch: { zones: 4, image: null, gestures: ["tap", "longPress", "swipe"] },
  },
  support: "descriptorOnly",
  verification: [],
}

const layout = (): DeviceLayout => ({
  id: "l",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  pages: [{ id: "home", name: "Home", bindings: [] }],
})

const bind = (index: number, action: string): ControlBinding => ({
  control: { kind: "key", index },
  trigger: { type: "press" },
  target: { type: "action", action },
})

describe("controlsOf", () => {
  it("derives grids from capabilities", () => {
    expect(controlsOf(mk2).keys).toHaveLength(15)
    expect(controlsOf(mk2).columns).toBe(5)
    expect(controlsOf(xl).keys).toHaveLength(32)
    expect(controlsOf(xl).keys[31]).toMatchObject({ row: 3, column: 7 })
    const p = controlsOf(plus)
    expect(p.keys).toHaveLength(8)
    expect(p.encoders).toHaveLength(4)
    expect(p.touchZones).toHaveLength(4)
  })

  it("offers only triggers the hardware can produce", () => {
    expect(supportedTriggers(mk2, { kind: "key", index: 0 }).map((t) => t.type)).toEqual([
      "press",
      "release",
    ])
    expect(supportedTriggers(plus, { kind: "encoder", index: 0 })).toHaveLength(4)
    expect(supportedTriggers(plus, { kind: "touchZone", index: 0 })).toHaveLength(3)
  })
})

describe("bindings", () => {
  it("replaces the binding for the same control and trigger", () => {
    let l = setBinding(layout(), "home", bind(0, "a"))
    l = setBinding(l, "home", bind(0, "b"))
    l = setBinding(l, "home", bind(1, "c"))
    expect(bindingsFor(l, "home", { kind: "key", index: 0 })).toEqual([bind(0, "b")])
    l = removeBinding(l, "home", { kind: "key", index: 0 }, { type: "press" })
    expect(bindingsFor(l, "home", { kind: "key", index: 0 })).toEqual([])
    expect(l.pages[0]?.bindings).toHaveLength(1)
  })

  it("picks a free trigger for a dropped action", () => {
    const l = setBinding(layout(), "home", bind(0, "a"))
    expect(defaultTrigger(mk2, l, "home", { kind: "key", index: 0 })).toEqual({ type: "release" })
    expect(defaultTrigger(mk2, l, "home", { kind: "key", index: 1 })).toEqual({ type: "press" })
  })

  it("deleting a folder removes the keys that open it, never the home page", () => {
    let l = addPage(layout(), "folder", "Tools")
    l = setBinding(l, "home", {
      control: { kind: "key", index: 14 },
      trigger: { type: "press" },
      target: { type: "openPage", page: "folder" },
    })
    l = deletePage(l, "folder")
    expect(l.pages).toHaveLength(1)
    expect(l.pages[0]?.bindings).toEqual([])
    expect(deletePage(l, "home").pages).toHaveLength(1)
  })

  it("keeps pages stored inside a deleted folder in its folder", () => {
    const l: DeviceLayout = {
      ...layout(),
      explicitTree: true,
      pages: [
        { id: "home", name: "Home", bindings: [] },
        { id: "a", name: "A", parent: "home", bindings: [] },
        { id: "b", name: "B", parent: "a", bindings: [] },
        { id: "c", name: "C", bindings: [] },
        { id: "d", name: "D", parent: "c", bindings: [] },
      ],
    }
    let next = deletePage(l, "a")
    expect(next.pages.find((p) => p.id === "b")?.parent).toBe("home")
    next = deletePage(next, "c")
    // Its folder was the top level; no parent pointing at a missing page.
    expect(next.pages.find((p) => p.id === "d")?.parent).toBeUndefined()
    expect(JSON.parse(JSON.stringify(next.pages))).toEqual([
      { id: "home", name: "Home", bindings: [] },
      { id: "b", name: "B", parent: "home", bindings: [] },
      { id: "d", name: "D", bindings: [] },
    ])
  })
})

describe("page tree", () => {
  const open = (index: number, page: string): ControlBinding => ({
    control: { kind: "key", index },
    trigger: { type: "press" },
    target: { type: "openPage", page },
  })
  const back = (index: number): ControlBinding => ({
    control: { kind: "key", index },
    trigger: { type: "press" },
    target: { type: "back" },
  })
  const tree = (pages: DeviceLayout["pages"], explicitTree?: boolean): DeviceLayout => ({
    ...layout(),
    pages,
    ...(explicitTree === undefined ? {} : { explicitTree }),
  })

  it("infers folders breadth-first from the home page", () => {
    // "deep" is opened from "tools" and from "home" (later in the list):
    // the walk from home reaches it first through home.
    const l = tree([
      { id: "tools", name: "Tools", bindings: [open(0, "deep")] },
      { id: "home", name: "Home", bindings: [open(0, "tools"), open(1, "deep")] },
      { id: "deep", name: "Deep", bindings: [open(0, "tools")] },
      { id: "lost", name: "Lost", bindings: [] },
    ])
    expect(Object.fromEntries(pageParents(l))).toEqual({ tools: "home", deep: "home" })
    expect(ancestors(l, "tools")).toEqual(["home"])
    expect(ancestors(l, "lost")).toEqual([])
  })

  it("prefers stored parents and infers nothing for an explicit tree", () => {
    const pages = [
      { id: "home", name: "Home", bindings: [open(0, "a"), open(1, "b")] },
      { id: "a", name: "A", bindings: [] },
      { id: "b", name: "B", parent: "a", bindings: [] },
    ]
    expect(Object.fromEntries(pageParents(tree(pages)))).toEqual({ a: "home", b: "a" })
    expect(ancestors(tree(pages), "b")).toEqual(["a", "home"])
    expect(Object.fromEntries(pageParents(tree(pages, true)))).toEqual({ b: "a" })
  })

  it("ignores stored parents that are missing, on home or cyclic", () => {
    const l = tree(
      [
        { id: "home", name: "Home", parent: "a", bindings: [] },
        { id: "a", name: "A", parent: "b", bindings: [] },
        { id: "b", name: "B", parent: "a", bindings: [] },
        { id: "c", name: "C", parent: "gone", bindings: [] },
      ],
      true,
    )
    expect(Object.fromEntries(pageParents(l))).toEqual({ a: "b" })
    expect(ancestors(l, "b")).toEqual([])
  })

  it("knows whether a page in a folder can be left", () => {
    const l = tree([
      { id: "home", name: "Home", bindings: [open(0, "a")] },
      { id: "a", name: "A", bindings: [open(0, "b")] },
      { id: "b", name: "B", bindings: [open(0, "home")] },
      { id: "c", name: "C", parent: "a", bindings: [back(3)] },
      { id: "d", name: "D", parent: "a", bindings: [] },
      { id: "lost", name: "Lost", bindings: [] },
    ])
    expect(hasWayBack(l, "home")).toBe(true)
    expect(hasWayBack(l, "a")).toBe(false)
    // Opening one of its folders counts as a way back.
    expect(hasWayBack(l, "b")).toBe(true)
    expect(hasWayBack(l, "c")).toBe(true)
    expect(hasWayBack(l, "d")).toBe(false)
    expect(hasWayBack(l, "lost")).toBe(true)
  })
})
