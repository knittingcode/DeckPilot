import { describe, expect, it } from "vitest"

import {
  assignTarget,
  createFolder,
  dropOnControl,
  duplicateControls,
  moveControl,
  pasteControls,
  copyControls,
} from "@/layout-editor/editing"

import {
  autoExitOf,
  backPosition,
  checkAutoExitSeconds,
  isSystemBack,
  legacyAutomaticBack,
  moveBackToReservedKey,
  reservedBackControl,
  setAutoExit,
  systemBackControl,
  withoutSystemBack,
} from "./folder-back"
import { bindingsFor } from "./layout-model"
import type { ControlBinding, ControlId, DeviceLayout, DeviceModel } from "./types"

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: {
    keys: { rows: 3, columns: 5, image: { width: 72, height: 72 } as never },
    encoders: null,
    touch: null,
  },
  support: "verified",
  verification: [],
}
// Keys without a display (a pedal): no reserved key.
const pedal: DeviceModel = {
  ...mk2,
  id: "test.pedal",
  capabilities: { keys: { rows: 1, columns: 3, image: null }, encoders: null, touch: null },
}
const dials: DeviceModel = {
  ...mk2,
  id: "test.dials",
  capabilities: { keys: null, encoders: { count: 2, pressable: true }, touch: null },
}

const key = (index: number): ControlId => ({ kind: "key", index })
const press = (control: ControlId, action: string): ControlBinding => ({
  control,
  trigger: { type: "press" },
  target: { type: "action", action },
  label: null,
})
const back = (control: ControlId): ControlBinding => ({
  control,
  trigger: { type: "press" },
  target: { type: "back" },
  label: null,
})

/** Home opens Tools (reserved), Tools opens Docker (reserved, nested). */
function layout(tools: ControlBinding[] = [], position: "reserved" | "legacy" | null = "reserved") {
  const value: DeviceLayout = {
    id: "l",
    name: "Main",
    model: mk2.id,
    homePage: "home",
    explicitTree: true,
    pages: [
      {
        id: "home",
        name: "Home",
        bindings: [{ ...press(key(1), "x"), target: { type: "openPage", page: "tools" } }],
      },
      {
        id: "tools",
        name: "Tools",
        parent: "home",
        bindings: tools,
        ...(position ? { back: { position } } : {}),
      },
      {
        id: "docker",
        name: "Docker",
        parent: "tools",
        bindings: [],
        back: { position: "reserved" },
      },
    ],
  }
  return value
}

describe("the reserved Back key", () => {
  it("is the first key with a display, never a dial or a key without one", () => {
    expect(reservedBackControl(mk2)).toEqual(key(0))
    expect(reservedBackControl(pedal)).toBeNull()
    expect(reservedBackControl(dials)).toBeNull()
  })

  it("is on every page of a folder, nested ones too, and never on the root page", () => {
    const l = layout()
    expect(systemBackControl(mk2, l, "home")).toBeNull()
    expect(backPosition(l, "home")).toBeNull()
    expect(systemBackControl(mk2, l, "tools")).toEqual(key(0))
    expect(systemBackControl(mk2, l, "docker")).toEqual(key(0))
    expect(isSystemBack(mk2, l, "docker", key(0))).toBe(true)
    expect(isSystemBack(mk2, l, "docker", key(1))).toBe(false)
    expect(systemBackControl(pedal, l, "tools")).toBeNull()
  })

  it("is not reserved on legacy folders or folders without the setting", () => {
    expect(backPosition(layout([], "legacy"), "tools")).toBe("legacy")
    expect(backPosition(layout([], null), "tools")).toBe("legacy")
    expect(systemBackControl(mk2, layout([], null), "tools")).toBeNull()
    // Legacy keeps the automatic Back on the first free key.
    const legacy = layout([press(key(0), "a")], "legacy")
    expect(legacyAutomaticBack(mk2, legacy, "tools")).toEqual(key(1))
    expect(legacyAutomaticBack(mk2, layout(), "tools")).toBeNull()
  })

  it("is left out of what bulk edits change", () => {
    expect(withoutSystemBack(mk2, layout(), "tools", [key(0), key(2)])).toEqual([key(2)])
    expect(withoutSystemBack(mk2, layout(), "home", [key(0), key(2)])).toEqual([key(0), key(2)])
  })
})

describe("editing refuses the system Back", () => {
  const target = { type: "action", action: "plugin" } as const

  it("assigning skips it", () => {
    const l = layout()
    const next = assignTarget(mk2, l, "tools", [key(0), key(2)], target)
    expect(bindingsFor(next, "tools", key(0))).toEqual([])
    expect(bindingsFor(next, "tools", key(2))).toHaveLength(1)
  })

  it("dropping on it or moving to or from it is refused", () => {
    const l = layout([press(key(3), "a")])
    expect(dropOnControl(mk2, l, "tools", { type: "target", target }, key(0))).toBeNull()
    expect(dropOnControl(mk2, l, "tools", { type: "control", control: key(3) }, key(0))).toBeNull()
    expect(moveControl(mk2, l, "tools", key(3), key(0))).toBeNull()
    expect(moveControl(mk2, l, "tools", key(0), key(3))).toBeNull()
  })

  it("pasting and duplicating never fill it", () => {
    const l = layout([press(key(3), "a")])
    const clipboard = copyControls(mk2, l, "tools", [key(3)])
    if (!clipboard) throw new Error("nothing copied")
    const pasted = pasteControls(mk2, l, "tools", clipboard, [key(0)])
    expect(bindingsFor(pasted.layout, "tools", key(0))).toEqual([])
    expect(pasted.skipped).toBe(1)
    // Duplicating key 14's neighbourhood wraps nowhere; key 0 is never "free".
    const full = layout(
      Array.from({ length: 14 }, (_, i) => press(key(i + 1), `a${i}`)).filter(
        (b) => b.control.index !== 14,
      ),
    )
    const duplicated = duplicateControls(mk2, full, "tools", [key(13)])
    expect(duplicated.controls).toEqual([key(14)])
    expect(bindingsFor(duplicated.layout, "tools", key(0))).toEqual([])
  })

  it("a folder cannot be created on it; new folders get the reserved Back", () => {
    expect(createFolder(mk2, layout(), "tools", key(0), "new")).toBeNull()
    const made = createFolder(mk2, layout(), "tools", key(4), "new")
    expect(made?.pages.at(-1)).toMatchObject({
      parent: "tools",
      back: { position: "reserved" },
      autoExit: { enabled: false, seconds: 10 },
    })
  })
})

describe("Move Back to the first key", () => {
  it("moves what the first key runs, with its timings, to the first free key", () => {
    const l = layout([press(key(0), "a"), press(key(1), "b")], "legacy")
    const tools = l.pages[1]
    if (!tools) throw new Error("no folder")
    l.pages[1] = { ...tools, gestures: [{ control: key(0), longPressMs: 900 }] }
    const moved = moveBackToReservedKey(mk2, l, "tools")
    if (!moved) throw new Error("not moved")
    expect(moved.movedTo).toEqual(key(2))
    const next = moved.layout
    expect(bindingsFor(next, "tools", key(0))).toEqual([])
    expect(bindingsFor(next, "tools", key(2))).toEqual([press(key(2), "a")])
    expect(next.pages[1]?.gestures).toEqual([{ control: key(2), longPressMs: 900 }])
    expect(backPosition(next, "tools")).toBe("reserved")
  })

  it("keeps a Back binding on the first key and moves only the rest", () => {
    const kept = moveBackToReservedKey(mk2, layout([back(key(0))], null), "tools")
    expect(kept?.movedTo).toBeNull()
    expect(kept?.layout.pages[1]?.bindings).toEqual([back(key(0))])
    expect(kept?.layout.pages[1]?.back).toEqual({ position: "reserved" })
    const longPress = { ...press(key(0), "a"), trigger: { type: "longPress" as const } }
    const click = { ...back(key(0)), trigger: { type: "click" as const } }
    const mixed = moveBackToReservedKey(mk2, layout([click, longPress], "legacy"), "tools")
    expect(mixed?.movedTo).toEqual(key(1))
    expect(mixed?.layout.pages[1]?.bindings).toEqual([click, { ...longPress, control: key(1) }])
  })

  it("refuses without a free key, outside folders and without a key for Back", () => {
    const full = layout(
      Array.from({ length: 15 }, (_, i) => press(key(i), `a${i}`)),
      "legacy",
    )
    expect(moveBackToReservedKey(mk2, full, "tools")).toBeNull()
    expect(moveBackToReservedKey(mk2, layout(), "home")).toBeNull()
    expect(moveBackToReservedKey(pedal, layout([], "legacy"), "tools")).toBeNull()
  })
})

describe("Auto Exit", () => {
  it("is off with 10 seconds unless stored, and is stored with the page", () => {
    expect(autoExitOf(layout().pages[1])).toEqual({ enabled: false, seconds: 10 })
    const next = setAutoExit(layout(), "tools", { enabled: true, seconds: 30 })
    expect(next.pages[1]?.autoExit).toEqual({ enabled: true, seconds: 30 })
    expect(autoExitOf(next.pages[1])).toEqual({ enabled: true, seconds: 30 })
  })

  it("takes whole seconds from 1 to 3600", () => {
    expect(checkAutoExitSeconds("1")).toBeNull()
    expect(checkAutoExitSeconds(3600)).toBeNull()
    expect(checkAutoExitSeconds("0")).toBe("editor:folderSettings.invalid.range")
    expect(checkAutoExitSeconds("3601")).toBe("editor:folderSettings.invalid.range")
    expect(checkAutoExitSeconds("1.5")).toBe("editor:folderSettings.invalid.whole")
    expect(checkAutoExitSeconds("-3")).toBe("editor:folderSettings.invalid.whole")
    expect(checkAutoExitSeconds("")).toBe("editor:folderSettings.invalid.whole")
  })
})
