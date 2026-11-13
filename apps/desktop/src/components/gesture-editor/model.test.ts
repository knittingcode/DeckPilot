import { describe, expect, it } from "vitest"

import {
  clearControls,
  copyControls,
  duplicateControls,
  moveControl,
  pasteControls,
} from "@/layout-editor/editing"
import { bindingsFor, defaultTrigger } from "@/lib/layout-model"
import type { ControlId, DeviceLayout, DeviceModel } from "@/lib/types"

import {
  clickTiming,
  controlIssues,
  convertToGestures,
  gestureBinding,
  keyMode,
  keyTimings,
  needsConversion,
  setGesture,
  setKeyTiming,
  timingError,
} from "./model"

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "verified",
  verification: [],
}

const key = (index: number): ControlId => ({ kind: "key", index })

function layout(page: Partial<DeviceLayout["pages"][number]> = {}): DeviceLayout {
  return {
    id: "l",
    name: "Main",
    model: mk2.id,
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings: [], ...page }],
  }
}

const shop = { type: "action", action: "shop" } as const
const mail = { type: "action", action: "mail" } as const
const triggers = (l: DeviceLayout, control: ControlId) =>
  bindingsFor(l, "home", control).map((b) => b.trigger.type)

describe("gesture model", () => {
  it("adds and removes each gesture on an empty key", () => {
    let l = layout()
    expect(keyMode(l, "home", key(0))).toBe("empty")
    // A single click alone stays the immediate press (D2).
    l = setGesture(l, "home", key(0), "click", shop)
    expect(triggers(l, key(0))).toEqual(["press"])
    expect(keyMode(l, "home", key(0))).toBe("raw")

    // A key without a press starts in gesture mode.
    l = setGesture(l, "home", key(1), "doubleClick", shop)
    l = setGesture(l, "home", key(1), "longPress", mail)
    l = setGesture(l, "home", key(1), "click", mail)
    expect(triggers(l, key(1))).toEqual(["doubleClick", "longPress", "click"])
    expect(keyMode(l, "home", key(1))).toBe("gesture")

    for (const gesture of ["doubleClick", "longPress", "click"] as const) {
      l = setGesture(l, "home", key(1), gesture, null)
    }
    expect(triggers(l, key(1))).toEqual([])
    l = setGesture(l, "home", key(0), "click", null)
    expect(l.pages[0]?.bindings).toEqual([])
  })

  it("converts a raw press to a click, keeping its target, label and look", () => {
    const l = layout({
      bindings: [
        {
          control: key(0),
          trigger: { type: "press" },
          target: { ...shop, overrides: { message: { type: "text", value: "hi" } } },
          label: "Shop",
          appearance: { background: "#112233" },
        },
        { control: key(0), trigger: { type: "release" }, target: mail },
        { control: key(1), trigger: { type: "press" }, target: mail },
      ],
    })
    expect(needsConversion(l, "home", key(0), "longPress")).toBe(true)
    expect(needsConversion(l, "home", key(0), "click")).toBe(false)
    expect(needsConversion(l, "home", key(2), "longPress")).toBe(false)
    // A stateful target needs gestures, also on the single click.
    const workSession = { type: "stateful", machine: "workSession" } as const
    expect(needsConversion(l, "home", key(0), "click", workSession)).toBe(true)
    expect(triggers(setGesture(l, "home", key(2), "click", workSession), key(2))).toEqual(["click"])

    const { layout: converted, removed } = convertToGestures(l, "home", key(0))
    expect(removed.map((b) => b.trigger.type)).toEqual(["release"])
    expect(gestureBinding(converted, "home", key(0), "click")).toEqual({
      ...l.pages[0]?.bindings[0],
      trigger: { type: "click" },
    })
    // Other keys stay raw.
    expect(triggers(converted, key(1))).toEqual(["press"])
  })

  it("keeps overrides while the action stays and drops them when it changes", () => {
    const overrides = { message: { type: "reference", value: "shared.team.name" } } as const
    const l = layout({
      bindings: [{ control: key(0), trigger: { type: "click" }, target: { ...shop, overrides } }],
    })
    const same = setGesture(l, "home", key(0), "click", shop)
    expect(gestureBinding(same, "home", key(0), "click")?.target).toEqual({ ...shop, overrides })
    const other = setGesture(l, "home", key(0), "click", mail)
    expect(gestureBinding(other, "home", key(0), "click")?.target).toEqual(mail)
  })

  it("says when a single click fires (D1)", () => {
    let l = layout({ bindings: [{ control: key(0), trigger: { type: "press" }, target: shop }] })
    expect(clickTiming(l, "home", key(0))).toBe("immediate")
    l = convertToGestures(l, "home", key(0)).layout
    expect(clickTiming(l, "home", key(0))).toBe("immediate")
    l = setGesture(l, "home", key(0), "longPress", mail)
    expect(clickTiming(l, "home", key(0))).toBe("onRelease")
    l = setGesture(l, "home", key(0), "doubleClick", mail)
    expect(clickTiming(l, "home", key(0))).toBe("afterWindow")
  })

  it("stores only timing overrides and validates their ranges", () => {
    const l = layout()
    const set = setKeyTiming(l, "home", key(2), "longPressMs", 1200)
    expect(set.pages[0]?.gestures).toEqual([{ control: key(2), longPressMs: 1200 }])
    const both = setKeyTiming(set, "home", key(2), "doubleClickMs", 450)
    expect(keyTimings(both, "home", key(2))).toEqual({
      control: key(2),
      doubleClickMs: 450,
      longPressMs: 1200,
    })
    const cleared = setKeyTiming(
      setKeyTiming(both, "home", key(2), "longPressMs", undefined),
      "home",
      key(2),
      "doubleClickMs",
      undefined,
    )
    expect(cleared.pages[0]?.gestures).toEqual([])
    // A page that never had timings is left exactly as it was.
    expect(setKeyTiming(l, "home", key(2), "longPressMs", undefined)).toEqual(l)

    expect(timingError("doubleClickMs", "")).toBeNull()
    expect(timingError("doubleClickMs", "150")).toBeNull()
    expect(timingError("doubleClickMs", "1000")).toBeNull()
    expect(timingError("doubleClickMs", "149")).toMatch(/between 150 and 1000 ms/)
    expect(timingError("longPressMs", "5001")).toMatch(/between 300 and 5000 ms/)
    expect(timingError("longPressMs", "700.5")).toMatch(/whole number/)
  })

  it("picks the backend's messages about one control", () => {
    const messages = [
      "page `Home`: key 0 mixes press or release with click, double click or long press",
      "page `Home`: key 10: the long-press time must be between 300 and 5000 ms, not 9000 ms",
      "page `Home`: the gesture timings of key 1 are set more than once",
      "page `Other`: key 0 mixes press or release with click, double click or long press",
    ]
    const home = { name: "Home" }
    expect(controlIssues(messages, home, key(0))).toEqual([
      "Key 1 mixes press or release with click, double click or long press",
    ])
    expect(controlIssues(messages, home, key(1))).toEqual([
      "the gesture timings of Key 2 are set more than once",
    ])
    expect(controlIssues(messages, home, key(10))).toHaveLength(1)
  })
})

describe("editing keeps gestures", () => {
  const gestureKey = layout({
    bindings: [
      { control: key(0), trigger: { type: "click" }, target: shop },
      { control: key(0), trigger: { type: "longPress" }, target: mail },
    ],
    gestures: [{ control: key(0), longPressMs: 1500 }],
  })

  it("assigns new targets to a gesture key as gestures, never as raw edges", () => {
    expect(defaultTrigger(mk2, gestureKey, "home", key(0))).toEqual({ type: "doubleClick" })
    expect(defaultTrigger(mk2, layout(), "home", key(0))).toEqual({ type: "press" })
  })

  it("copies, pastes, duplicates and moves bindings with their timings", () => {
    const clipboard = copyControls(mk2, gestureKey, "home", [key(0)])
    if (!clipboard) throw new Error("nothing copied")
    const pasted = pasteControls(mk2, gestureKey, "home", clipboard, [key(4)])
    expect(pasted.skipped).toBe(0)
    expect(triggers(pasted.layout, key(4))).toEqual(["click", "longPress"])
    expect(keyTimings(pasted.layout, "home", key(4))?.longPressMs).toBe(1500)

    const duplicated = duplicateControls(mk2, gestureKey, "home", [key(0)])
    expect(duplicated.controls).toEqual([key(1)])
    expect(keyTimings(duplicated.layout, "home", key(1))?.longPressMs).toBe(1500)

    const moved = moveControl(mk2, gestureKey, "home", key(0), key(7))
    if (!moved) throw new Error("move refused")
    expect(triggers(moved, key(7))).toEqual(["click", "longPress"])
    expect(moved.pages[0]?.gestures).toEqual([{ control: key(7), longPressMs: 1500 }])

    const cleared = clearControls(gestureKey, "home", [key(0)])
    expect(cleared.pages[0]?.bindings).toEqual([])
    expect(cleared.pages[0]?.gestures).toEqual([])
  })
})
