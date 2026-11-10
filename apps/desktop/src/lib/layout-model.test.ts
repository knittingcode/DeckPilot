import { describe, expect, it } from "vitest"

import {
  addPage,
  bindingsFor,
  controlsOf,
  defaultTrigger,
  deletePage,
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
})
