import { describe, expect, it } from "vitest"

import { canRedo, canUndo, commit, history, redo, undo } from "@/lib/history"

import {
  assignTarget,
  clearControls,
  controlRows,
  copyControls,
  dropOnControl,
  duplicateControls,
  emptySelection,
  extendSelection,
  folderOf,
  moveControl,
  neighbour,
  orderedControls,
  parentPage,
  parseControlKey,
  pasteControls,
  select,
  selectOnly,
  selectedInOrder,
  toggleSelected,
} from "./editing"
import { bindingsFor } from "@/lib/layout-model"
import type { ControlBinding, ControlId, DeviceLayout, DeviceModel } from "@/lib/types"

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: {
    keys: { rows: 3, columns: 5, image: null },
    encoders: null,
    touch: null,
  },
  support: "unverified",
  verification: [],
}

const plus: DeviceModel = {
  ...mk2,
  id: "elgato.streamdeck.plus",
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: true },
    touch: { zones: 2, image: null, gestures: ["tap"] },
  },
}

const dialsOnly: DeviceModel = {
  ...mk2,
  id: "test.dials",
  capabilities: { keys: null, encoders: { count: 3, pressable: false }, touch: null },
}

const key = (index: number): ControlId => ({ kind: "key", index })
const dial = (index: number): ControlId => ({ kind: "encoder", index })
const zone = (index: number): ControlId => ({ kind: "touchZone", index })

const press = (
  control: ControlId,
  action: string,
  label: string | null = null,
): ControlBinding => ({
  control,
  trigger: { type: "press" },
  target: { type: "action", action },
  label,
})

function layout(home: ControlBinding[], folder: ControlBinding[] = []): DeviceLayout {
  return {
    id: "l1",
    name: "Main",
    model: mk2.id,
    homePage: "home",
    pages: [
      { id: "home", name: "Home", bindings: home },
      { id: "folder", name: "Folder", bindings: folder },
    ],
  }
}

function must<T>(value: T | null | undefined): T {
  if (value === null || value === undefined) throw new Error("expected a value")
  return value
}

const actionsOn = (l: DeviceLayout, pageId: string, control: ControlId) =>
  bindingsFor(l, pageId, control).map((b) =>
    b.target.type === "action" ? b.target.action : b.target.type,
  )

describe("control order and navigation", () => {
  it("orders keys, touch zones and dials as drawn", () => {
    expect(orderedControls(mk2)).toHaveLength(15)
    expect(orderedControls(plus).map((c) => c.kind)).toEqual([
      ...Array<string>(8).fill("key"),
      "touchZone",
      "touchZone",
      "encoder",
      "encoder",
      "encoder",
      "encoder",
    ])
    expect(orderedControls(dialsOnly)).toEqual([dial(0), dial(1), dial(2)])
    expect(controlRows(plus).map((r) => r.length)).toEqual([4, 4, 2, 4])
  })

  it("moves within the key grid and stops at edges", () => {
    expect(neighbour(mk2, key(0), "right")).toEqual(key(1))
    expect(neighbour(mk2, key(0), "left")).toEqual(key(0))
    expect(neighbour(mk2, key(0), "up")).toEqual(key(0))
    expect(neighbour(mk2, key(2), "down")).toEqual(key(7))
    expect(neighbour(mk2, key(14), "down")).toEqual(key(14))
    expect(neighbour(mk2, key(4), "right")).toEqual(key(4))
  })

  it("moves between rows of different length by position", () => {
    // Key 8 (bottom right) is above the right touch zone, which is above dial 4.
    expect(neighbour(plus, key(7), "down")).toEqual(zone(1))
    expect(neighbour(plus, key(4), "down")).toEqual(zone(0))
    expect(neighbour(plus, zone(1), "down")).toEqual(dial(3))
    expect(neighbour(plus, dial(3), "up")).toEqual(zone(1))
    expect(neighbour(plus, zone(0), "up")).toEqual(key(5))
    expect(neighbour(dialsOnly, dial(1), "right")).toEqual(dial(2))
  })
})

describe("selection", () => {
  it("selects, toggles and extends", () => {
    let s = selectOnly(key(1))
    s = toggleSelected(s, key(3))
    expect(s.controls).toEqual([key(1), key(3)])
    expect(s.focus).toEqual(key(3))
    s = toggleSelected(s, key(3))
    expect(s.controls).toEqual([key(1)])
    expect(s.focus).toEqual(key(1))
    s = toggleSelected(s, key(1))
    expect(s).toEqual(emptySelection)
  })

  it("extends a range from the anchor across control kinds", () => {
    const s = extendSelection(plus, selectOnly(key(6)), zone(0))
    expect(s.controls).toEqual([key(6), key(7), zone(0)])
    expect(s.anchor).toEqual(key(6))
    const back = select(plus, s, key(5), "extend")
    expect(back.controls).toEqual([key(5), key(6)])
    expect(selectedInOrder(plus, toggleSelected(selectOnly(key(3)), key(1)))).toEqual([
      key(1),
      key(3),
    ])
  })
})

describe("clipboard", () => {
  it("copies and pastes keeping relative positions", () => {
    const l = layout([press(key(0), "a"), press(key(2), "b", "B")])
    const clip = copyControls(mk2, l, "home", [key(2), key(0)])
    expect(clip?.entries.map((e) => e.offset)).toEqual([0, 2])
    const result = pasteControls(mk2, l, "folder", must(clip), [key(5)])
    expect(result.controls).toEqual([key(5), key(7)])
    expect(actionsOn(result.layout, "folder", key(5))).toEqual(["a"])
    expect(bindingsFor(result.layout, "folder", key(7))[0]?.label).toBe("B")
    expect(result.skipped).toBe(0)
  })

  it("fills every selected control from a single copied control", () => {
    const l = layout([press(key(0), "a"), press(key(1), "old")])
    const clip = must(copyControls(mk2, l, "home", [key(0)]))
    const result = pasteControls(mk2, l, "home", clip, [key(1), key(3)])
    expect(actionsOn(result.layout, "home", key(1))).toEqual(["a"])
    expect(actionsOn(result.layout, "home", key(3))).toEqual(["a"])
  })

  it("pasting an empty control clears the target", () => {
    const l = layout([press(key(1), "a")])
    const clip = must(copyControls(mk2, l, "home", [key(0)]))
    expect(actionsOn(pasteControls(mk2, l, "home", clip, [key(1)]).layout, "home", key(1))).toEqual(
      [],
    )
  })

  it("skips unsupported triggers, positions past the end and self-opening folders", () => {
    const l: DeviceLayout = {
      ...layout([
        press(key(7), "a"),
        { control: dial(0), trigger: { type: "rotateClockwise" }, target: { type: "back" } },
        {
          control: key(0),
          trigger: { type: "press" },
          target: { type: "openPage", page: "folder" },
        },
      ]),
      model: plus.id,
    }
    const turn = must(copyControls(plus, l, "home", [dial(0)]))
    const onKey = pasteControls(plus, l, "home", turn, [key(1)])
    expect(onKey.skipped).toBe(1)
    expect(actionsOn(onKey.layout, "home", key(1))).toEqual([])

    const tail = must(copyControls(plus, l, "home", [key(7), dial(0)]))
    expect(pasteControls(plus, l, "home", tail, [dial(3)]).skipped).toBe(1)

    const opener = must(copyControls(plus, l, "home", [key(0)]))
    const inFolder = pasteControls(plus, l, "folder", opener, [key(0)])
    expect(inFolder.skipped).toBe(1)
    expect(actionsOn(inFolder.layout, "folder", key(0))).toEqual([])
  })

  it("copies nothing without controls", () => {
    expect(copyControls(mk2, layout([]), "home", [])).toBeNull()
  })
})

describe("editing controls", () => {
  it("clears selected controls only", () => {
    const l = layout([press(key(0), "a"), press(key(1), "b"), press(key(2), "c")])
    const cleared = clearControls(l, "home", [key(0), key(2)])
    expect(cleared.pages[0]?.bindings.map((b) => b.control)).toEqual([key(1)])
  })

  it("assigns a target to several controls", () => {
    const l = assignTarget(mk2, layout([]), "home", [key(0), key(4)], {
      type: "workflow",
      workflow: "w",
    })
    expect(actionsOn(l, "home", key(4))).toEqual(["workflow"])
    expect(assignTarget(mk2, l, "home", [key(1)], { type: "openPage", page: "home" })).toBe(l)
  })

  it("moves to an empty control and swaps with a bound one", () => {
    const l = layout([press(key(0), "a"), press(key(1), "b")])
    const moved = must(moveControl(mk2, l, "home", key(0), key(5)))
    expect(actionsOn(moved, "home", key(0))).toEqual([])
    expect(actionsOn(moved, "home", key(5))).toEqual(["a"])
    const swapped = must(moveControl(mk2, l, "home", key(0), key(1)))
    expect(actionsOn(swapped, "home", key(0))).toEqual(["b"])
    expect(actionsOn(swapped, "home", key(1))).toEqual(["a"])
    expect(moveControl(mk2, l, "home", key(0), key(0))).toBe(l)
  })

  it("refuses moves the destination cannot take", () => {
    const l: DeviceLayout = {
      ...layout([
        { control: dial(0), trigger: { type: "rotateClockwise" }, target: { type: "back" } },
      ]),
      model: plus.id,
    }
    expect(moveControl(plus, l, "home", dial(0), key(0))).toBeNull()
    expect(moveControl(plus, l, "home", dial(0), dial(2))).not.toBeNull()
  })

  it("duplicates onto the next free control of the same kind", () => {
    const l = layout([press(key(0), "a"), press(key(1), "b"), press(key(3), "c")])
    const result = duplicateControls(mk2, l, "home", [key(0), key(3)])
    expect(result.controls).toEqual([key(2), key(4)])
    expect(actionsOn(result.layout, "home", key(2))).toEqual(["a"])
    expect(actionsOn(result.layout, "home", key(4))).toEqual(["c"])
    const full = layout(Array.from({ length: 15 }, (_, i) => press(key(i), `k${i}`)))
    expect(duplicateControls(mk2, full, "home", [key(3)]).controls).toEqual([])
  })
})

describe("drag and drop", () => {
  it("assigns library entries and moves controls", () => {
    const l = layout([press(key(0), "a")])
    const assigned = must(
      dropOnControl(
        mk2,
        l,
        "home",
        { type: "target", target: { type: "action", action: "n" } },
        key(3),
      ),
    )
    expect(actionsOn(assigned, "home", key(3))).toEqual(["n"])
    const moved = must(dropOnControl(mk2, l, "home", { type: "control", control: key(0) }, key(3)))
    expect(actionsOn(moved, "home", key(3))).toEqual(["a"])
    expect(
      dropOnControl(
        mk2,
        l,
        "home",
        { type: "target", target: { type: "openPage", page: "home" } },
        key(3),
      ),
    ).toBeNull()
  })

  it("parses control keys", () => {
    expect(parseControlKey("touchZone:2")).toEqual(zone(2))
    expect(parseControlKey("action:x")).toBeNull()
    expect(parseControlKey("key:-1")).toBeNull()
  })
})

describe("pages", () => {
  const opener: ControlBinding = {
    control: key(4),
    trigger: { type: "press" },
    target: { type: "openPage", page: "folder" },
  }

  it("finds folders, parents and back bindings", () => {
    const l = layout([opener], [{ ...press(key(0), "x"), target: { type: "back" } }])
    expect(folderOf(l, "home", key(4))).toBe("folder")
    expect(folderOf(l, "home", key(0))).toBeUndefined()
    expect(parentPage(l, "folder")).toBe("home")
    expect(parentPage(l, "home")).toBe("home")
    expect(parentPage(layout([]), "folder")).toBe("home")
  })
})

describe("history", () => {
  it("undoes and redoes commits", () => {
    let h = history(1)
    h = commit(h, 2)
    h = commit(h, 3)
    expect(canUndo(h)).toBe(true)
    h = undo(h)
    expect(h.present).toBe(2)
    expect(canRedo(h)).toBe(true)
    h = redo(h)
    expect(h.present).toBe(3)
    h = undo(undo(h))
    expect(h.present).toBe(1)
    expect(undo(h)).toBe(h)
    h = commit(h, 9)
    expect(canRedo(h)).toBe(false)
    expect(redo(h)).toBe(h)
  })

  it("ignores unchanged values and groups typing", () => {
    let h = history({ name: "a" })
    expect(commit(h, { name: "a" })).toBe(h)
    h = commit(h, { name: "ab" }, "name")
    h = commit(h, { name: "abc" }, "name")
    expect(h.past).toHaveLength(1)
    h = undo(h)
    expect(h.present.name).toBe("a")
  })

  it("is bounded", () => {
    let h = history(0)
    for (let i = 1; i <= 10; i++) h = commit(h, i, null, 3)
    expect(h.past).toEqual([7, 8, 9])
  })
})
