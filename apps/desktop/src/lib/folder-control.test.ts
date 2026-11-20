// The editor's mirror of `deckpilot_core::folder_control`: the rule by
// identifiers, bulk edits that leave folder keys alone, and moving or
// discarding recovered gestures.

import { describe, expect, it } from "vitest"

import {
  assignTarget,
  copyControls,
  dropOnControl,
  opensFolderBesides,
  pasteControls,
} from "@/layout-editor/editing"

import {
  brokenFolderControl,
  discardRecovered,
  folderControlOf,
  pressFolderControl,
  recoveredOf,
  restoreRecovered,
  withoutFolderControls,
} from "./folder-control"
import type { ControlBinding, DeviceLayout, DeviceModel } from "./types"

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

const key = (index: number) => ({ kind: "key" as const, index })
const on = (index: number, type: string, target: ControlBinding["target"]): ControlBinding => ({
  control: key(index),
  trigger: { type } as ControlBinding["trigger"],
  target,
  label: null,
})
const open = (page: string) => ({ type: "openPage" as const, page })
const shop = { type: "action" as const, action: "shop" }

/** Home: key 1 opens Tools (a folder), key 2 runs Shop, key 3 opens the top-level Other. */
function layout(): DeviceLayout {
  return {
    id: "l",
    name: "Main",
    model: mk2.id,
    homePage: "home",
    explicitTree: true,
    pages: [
      {
        id: "home",
        name: "Home",
        bindings: [
          on(1, "press", open("tools")),
          on(2, "press", shop),
          on(3, "press", open("other")),
        ],
      },
      { id: "tools", name: "Tools", parent: "home", bindings: [], back: { position: "reserved" } },
      { id: "other", name: "Other", bindings: [] },
    ],
    recovered: [
      {
        page: "home",
        control: key(1),
        folder: "tools",
        bindings: [on(1, "doubleClick", shop)],
        gestures: { control: key(1), doubleClickMs: 400 },
        reason: "folderControl",
      },
    ],
  }
}

describe("folder keys", () => {
  it("are the keys that open a folder of their page, by id", () => {
    const l = layout()
    expect(folderControlOf(l, "home", key(1))).toBe("tools")
    expect(folderControlOf(l, "home", key(2))).toBeUndefined()
    // A top-level page: an ordinary page key, whatever it is called.
    const renamed = { ...l, pages: l.pages.map((p) => ({ ...p, name: "Tools" })) }
    expect(folderControlOf(renamed, "home", key(3))).toBeUndefined()
    // Dials never are.
    const dial = { kind: "encoder" as const, index: 0 }
    const withDial: DeviceLayout = {
      ...l,
      pages: l.pages.map((p) =>
        p.id === "home"
          ? { ...p, bindings: [...p.bindings, { ...on(0, "press", open("tools")), control: dial }] }
          : p,
      ),
    }
    expect(folderControlOf(withDial, "home", dial)).toBeUndefined()
    // An inferred tree: the page a key opens first is its folder.
    const inferred: DeviceLayout = {
      ...l,
      explicitTree: false,
      pages: l.pages.map((p) => ({ ...p, parent: null })),
    }
    expect(folderControlOf(inferred, "home", key(1))).toBe("tools")
    expect(withoutFolderControls(l, "home", [key(1), key(2), key(4)])).toEqual([key(2), key(4)])
  })

  it("are left out of bulk assignments, pastes and Library drops", () => {
    const l = layout()
    const assigned = assignTarget(mk2, l, "home", [key(1), key(4)], shop)
    const home = assigned.pages[0]?.bindings ?? []
    expect(home.filter((b) => b.control.index === 1)).toEqual([on(1, "press", open("tools"))])
    expect(home.filter((b) => b.control.index === 4)).toHaveLength(1)
    const clipboard = copyControls(mk2, l, "home", [key(2)])
    expect(clipboard).not.toBeNull()
    if (!clipboard) return
    const pasted = pasteControls(mk2, l, "home", clipboard, [key(1), key(4)])
    expect(pasted.skipped).toBe(1)
    expect(pasted.layout.pages[0]?.bindings.filter((b) => b.control.index === 1)).toEqual([
      on(1, "press", open("tools")),
    ])
    expect(dropOnControl(mk2, l, "home", { type: "target", target: shop }, key(1))).toBeNull()
    // A folder goes on a free key only, where it opens on the press.
    expect(opensFolderBesides(mk2, l, "home", key(2), open("tools"))).toBe(true)
    expect(
      dropOnControl(mk2, l, "home", { type: "target", target: open("tools") }, key(2)),
    ).toBeNull()
    const placed = dropOnControl(mk2, l, "home", { type: "target", target: open("tools") }, key(5))
    expect(placed?.pages[0]?.bindings.find((b) => b.control.index === 5)?.trigger).toEqual({
      type: "press",
    })
  })

  it("keep one binding on the press", () => {
    const l = layout()
    const clicked: DeviceLayout = {
      ...l,
      pages: l.pages.map((p) =>
        p.id === "home" ? { ...p, bindings: [on(1, "click", open("tools"))] } : p,
      ),
    }
    expect(pressFolderControl(clicked, "home", key(1)).pages[0]?.bindings[0]?.trigger).toEqual({
      type: "press",
    })
    const extra: DeviceLayout = {
      ...l,
      pages: l.pages.map((p) =>
        p.id === "home" ? { ...p, bindings: [...p.bindings, on(1, "longPress", shop)] } : p,
      ),
    }
    expect(brokenFolderControl(extra, "home", key(1))).toBe(true)
    expect(brokenFolderControl(l, "home", key(1))).toBe(false)
  })

  it("move recovered gestures to the first free key, or discard them", () => {
    const l = layout()
    expect(recoveredOf(l, "home", key(1))).toHaveLength(1)
    const moved = restoreRecovered(mk2, l, "home", key(1))
    expect(moved?.movedTo).toEqual(key(0))
    expect(moved?.layout.recovered).toBeUndefined()
    expect(moved?.layout.pages[0]?.bindings.at(-1)).toEqual(on(0, "doubleClick", shop))
    expect(moved?.layout.pages[0]?.gestures).toEqual([{ control: key(0), doubleClickMs: 400 }])
    // Nothing to move from another key.
    expect(restoreRecovered(mk2, l, "home", key(2))).toBeNull()
    // No free key: refused.
    const full: DeviceLayout = {
      ...l,
      pages: l.pages.map((p) =>
        p.id === "home"
          ? {
              ...p,
              bindings: Array.from({ length: 15 }, (_, i) => on(i, "press", shop)),
            }
          : p,
      ),
    }
    expect(restoreRecovered(mk2, full, "home", key(1))).toBeNull()
    const discarded = discardRecovered(l, "home", key(1))
    expect(discarded.recovered).toBeUndefined()
    expect(discarded.pages).toEqual(l.pages)
  })
})
