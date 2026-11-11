import { describe, expect, it } from "vitest"

import type { ControlBinding, ControlId, DeviceLayout, DeviceModel } from "@/lib/types"

import {
  applyAppearance,
  appearanceOf,
  contrastHint,
  contrastRatio,
  effectiveUsage,
  iconFileProblem,
  iconName,
  iconSizeProblem,
  iconUsage,
  normalizeAppearance,
  parseHex,
  sharedAppearance,
  shownBinding,
  withIconFit,
} from "./appearance"
import { copyControls, moveControl, pasteControls } from "./editing"

const key = (index: number): ControlId => ({ kind: "key", index })
const dial: ControlId = { kind: "encoder", index: 0 }

const plus: DeviceModel = {
  id: "plus",
  vendor: "Elgato",
  name: "Stream Deck +",
  usb: null,
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: true },
    touch: null,
  },
  support: "unverified",
}

const bind = (control: ControlId, change: Partial<ControlBinding> = {}): ControlBinding => ({
  control,
  trigger: { type: "press" },
  target: { type: "action", action: "a" },
  ...change,
})

function layout(bindings: ControlBinding[]): DeviceLayout {
  return {
    id: "l",
    name: "L",
    model: plus.id,
    homePage: "home",
    pages: [
      { id: "home", name: "Home", bindings },
      { id: "other", name: "Other", bindings: [] },
    ],
  }
}

describe("shown binding", () => {
  it("prefers the press binding, like the agent", () => {
    const release = bind(key(0), {
      trigger: { type: "release" },
      appearance: { fontSize: "small" },
    })
    const press = bind(key(0), { appearance: { fontSize: "large" } })
    const l = layout([release, press])
    expect(shownBinding(l, "home", key(0))).toBe(press)
    expect(appearanceOf(l, "home", key(0))).toEqual({ fontSize: "large" })
    expect(shownBinding(layout([release]), "home", key(0))).toBe(release)
    expect(appearanceOf(l, "home", key(5))).toEqual({})
  })
})

describe("applying an appearance", () => {
  it("applies a patch to every selected key and skips keys that cannot carry it", () => {
    const l = layout([
      bind(key(0)),
      bind(key(1), { appearance: { textColor: "#ffffff" } }),
      bind(dial),
    ])
    const result = applyAppearance(l, "home", [key(0), key(1), key(2), dial], {
      background: "#FF0000",
    })
    expect(result.changed).toEqual([key(0), key(1)])
    expect(result.skipped).toEqual([key(2), dial])
    expect(appearanceOf(result.layout, "home", key(0))).toEqual({ background: "#ff0000" })
    expect(appearanceOf(result.layout, "home", key(1))).toEqual({
      background: "#ff0000",
      textColor: "#ffffff",
    })
    // The dial keeps no appearance; the input is not changed.
    expect(shownBinding(result.layout, "home", dial)?.appearance).toBeUndefined()
    expect(l.pages[0]?.bindings[0]?.appearance).toBeUndefined()
  })

  it("resets fields with null and stores nothing for a default look", () => {
    const l = layout([bind(key(0), { appearance: { background: "#000000", showTitle: false } })])
    const reset = applyAppearance(l, "home", [key(0)], { background: null, showTitle: true })
    expect(shownBinding(reset.layout, "home", key(0))).not.toHaveProperty("appearance")
  })

  it("changes the icon fit of keys with icons only", () => {
    const l = layout([
      bind(key(0), { appearance: { icon: { id: "i1" } } }),
      bind(key(1), { appearance: { icon: { id: "i2", fit: "fill" } } }),
      bind(key(2)),
    ])
    const result = applyAppearance(l, "home", [key(0), key(1), key(2)], withIconFit("crop"))
    expect(appearanceOf(result.layout, "home", key(0)).icon).toEqual({ id: "i1", fit: "crop" })
    expect(appearanceOf(result.layout, "home", key(1)).icon).toEqual({ id: "i2", fit: "crop" })
    expect(appearanceOf(result.layout, "home", key(2))).toEqual({})
    // "fit" is the default and not stored.
    const back = applyAppearance(result.layout, "home", [key(0)], withIconFit("fit"))
    expect(appearanceOf(back.layout, "home", key(0)).icon).toEqual({ id: "i1" })
  })

  it("normalizes appearances", () => {
    expect(normalizeAppearance({})).toBeNull()
    expect(normalizeAppearance({ showTitle: true, background: null })).toBeNull()
    expect(normalizeAppearance({ textColor: "#ABCDEF", showTitle: false })).toEqual({
      textColor: "#abcdef",
      showTitle: false,
    })
  })

  it("reports shared and mixed values of a selection", () => {
    const l = layout([
      bind(key(0), { appearance: { background: "#111111", fontSize: "large", icon: { id: "i" } } }),
      bind(key(1), { appearance: { background: "#111111", fontSize: "small" } }),
      bind(dial),
    ])
    expect(sharedAppearance(l, "home", [key(0), key(1), dial])).toEqual({
      background: "#111111",
      textColor: undefined,
      fontSize: "mixed",
      titleAlign: undefined,
      showTitle: true,
      iconId: "mixed",
      iconFit: "mixed",
    })
  })

  it("counts icon usage across pages", () => {
    const l = layout([
      bind(key(0), { appearance: { icon: { id: "i" } } }),
      bind(key(1), { appearance: { icon: { id: "i" } } }),
    ])
    expect(iconUsage(l).get("i")).toBe(2)
    expect(iconUsage(l).get("other")).toBeUndefined()
  })

  it("counts usage with the unsaved draft instead of the saved layout", () => {
    const saved = layout([bind(key(0), { appearance: { icon: { id: "i" } } })])
    const draft = layout([bind(key(0)), bind(key(1), { appearance: { icon: { id: "j" } } })])
    // Stored: 1 key of this layout and 2 of other layouts show "i".
    expect(effectiveUsage(3, "i", saved, draft)).toBe(2)
    expect(effectiveUsage(1, "i", saved, draft)).toBe(0)
    expect(effectiveUsage(0, "j", saved, draft)).toBe(1)
  })
})

describe("clipboard and moves keep the appearance", () => {
  it("copies, pastes and moves the look with the bindings", () => {
    const look = { background: "#123456", icon: { id: "i", fit: "crop" as const } }
    const l = layout([bind(key(0), { label: "Go", appearance: look })])
    const clip = copyControls(plus, l, "home", [key(0)])
    if (!clip) throw new Error("nothing copied")
    const pasted = pasteControls(plus, l, "other", clip, [key(3)])
    expect(appearanceOf(pasted.layout, "other", key(3))).toEqual(look)
    const moved = moveControl(plus, l, "home", key(0), key(1))
    expect(moved && appearanceOf(moved, "home", key(1))).toEqual(look)
  })
})

describe("colours", () => {
  it("parses hex colours", () => {
    expect(parseHex("#AbCdEf")).toBe("#abcdef")
    expect(parseHex("abcdef")).toBe("#abcdef")
    expect(parseHex("#fa0")).toBe("#ffaa00")
    expect(parseHex("#12345")).toBeNull()
    expect(parseHex("red")).toBeNull()
  })

  it("computes contrast and hints", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21)
    expect(contrastRatio("#777777", "#777777")).toBe(1)
    expect(contrastHint("#000000", "#ffffff").level).toBe("good")
    expect(contrastHint("#ffffff", "#949494").level).toBe("low")
    expect(contrastHint("#ffffff", null)).toMatchObject({ level: "poor" })
    expect(contrastHint(null, null).level).toBe("good")
  })
})

describe("icon files", () => {
  it("checks type, size and the icon limit before uploading", () => {
    const file = (name: string, type: string, size = 1000) => ({ name, type, size })
    expect(iconFileProblem(file("a.png", "image/png"), 0)).toBeNull()
    expect(iconFileProblem(file("a.JPG", ""), 0)).toBeNull()
    expect(iconFileProblem(file("a.gif", "image/gif"), 0)).toBe("Choose a PNG or JPEG image.")
    expect(iconFileProblem(file("a.png", "image/png", 2 * 1024 * 1024 + 1), 0)).toMatch(/2 MiB/)
    expect(iconFileProblem(file("a.png", "image/png", 0), 0)).toBe("The file is empty.")
    expect(iconFileProblem(file("a.png", "image/png"), 64)).toMatch(/at most 64 icons/)
    expect(iconSizeProblem(4096, 10)).toBeNull()
    expect(iconSizeProblem(4097, 10)).toMatch(/4096/)
  })

  it("names icons after the file", () => {
    expect(iconName("rocket.png")).toBe("rocket")
    expect(iconName(".png")).toBe("Icon")
    expect(iconName(`${"x".repeat(80)}.jpg`)).toHaveLength(64)
  })
})
