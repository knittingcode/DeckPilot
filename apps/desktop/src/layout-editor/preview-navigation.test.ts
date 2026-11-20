import { beforeEach, describe, expect, it } from "vitest"

import type { ActionType, DeviceLayout, EditorNavigation } from "@/lib/types"

import {
  adjacentPage,
  canGoBack,
  checkedNavigation,
  controlMove,
  peekLayoutHistory,
  popLayoutHistory,
  pushLayoutHistory,
  resetLayoutHistory,
} from "./preview-navigation"

const text = { type: "text", multiline: false } as const

function type(editorNavigation: unknown, parameters: ActionType["parameters"] = []): ActionType {
  return {
    id: "x.y",
    name: "X",
    description: "",
    category: "X",
    requiresProject: false,
    parameters,
    editorNavigation: editorNavigation as EditorNavigation,
  }
}

const layout: DeviceLayout = {
  id: "l",
  name: "L",
  model: "m",
  homePage: "a",
  explicitTree: true,
  pages: [
    {
      id: "a",
      name: "A",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "click" },
          target: { type: "openPage", page: "f" },
        },
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "doubleClick" },
          target: { type: "back" },
        },
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "press" },
          target: { type: "workflow", workflow: "w" },
        },
      ],
    },
    { id: "f", name: "F", parent: "a", bindings: [] },
    { id: "b", name: "B", bindings: [] },
    { id: "c", name: "C", bindings: [] },
  ],
}

describe("checkedNavigation", () => {
  it("accepts the allowlist with the parameters it needs", () => {
    expect(checkedNavigation(type({ operation: "page.next" }))).toEqual({ operation: "page.next" })
    const switching = { operation: "layout.switch", targetParameter: "l", pageParameter: "p" }
    expect(
      checkedNavigation(
        type(switching, [
          { key: "l", label: "L", kind: text, required: true },
          { key: "p", label: "P", kind: text, required: false },
        ]),
      ),
    ).toEqual(switching)
  })

  it("refuses anything else", () => {
    expect(checkedNavigation(undefined)).toBeNull()
    expect(checkedNavigation(type(undefined))).toBeNull()
    expect(checkedNavigation(type({ operation: "run.anything" }))).toBeNull()
    // A target missing, extra, or not a text parameter of the action.
    expect(checkedNavigation(type({ operation: "folder.open" }))).toBeNull()
    expect(checkedNavigation(type({ operation: "page.next", targetParameter: "x" }))).toBeNull()
    expect(
      checkedNavigation(
        type({ operation: "folder.open", targetParameter: "n" }, [
          { key: "n", label: "N", kind: { type: "integer" }, required: true },
        ]),
      ),
    ).toBeNull()
    // A page parameter outside layout.switch.
    expect(
      checkedNavigation(
        type({ operation: "folder.open", targetParameter: "t", pageParameter: "t" }, [
          { key: "t", label: "T", kind: text, required: true },
        ]),
      ),
    ).toBeNull()
  })
})

describe("controlMove", () => {
  it("takes the press, then the click, then the first binding that navigates", () => {
    expect(controlMove(layout, "a", { kind: "key", index: 0 }, [], () => undefined)).toEqual({
      type: "enter",
      page: "f",
    })
    // A workflow does not navigate.
    expect(controlMove(layout, "a", { kind: "key", index: 1 }, [], () => undefined)).toBeNull()
  })

  it("reads the automatic Back key from its face only when the control has no binding", () => {
    const automatic = { operation: "folder.back", automatic: true } as const
    expect(
      controlMove(layout, "f", { kind: "key", index: 0 }, [], () => undefined, automatic),
    ).toEqual({ type: "back" })
    expect(controlMove(layout, "f", { kind: "key", index: 0 }, [], () => undefined)).toBeNull()
  })
})

describe("adjacentPage and Back", () => {
  it("moves along the top-level pages, wrapping only when asked", () => {
    expect(adjacentPage(layout, "a", "next", false)).toBe("b")
    expect(adjacentPage(layout, "f", "next", false)).toBe("b")
    expect(adjacentPage(layout, "c", "next", false)).toBeUndefined()
    expect(adjacentPage(layout, "c", "next", true)).toBe("a")
    expect(adjacentPage(layout, "a", "previous", true)).toBe("c")
  })

  it("has nowhere to go back to at the top", () => {
    expect(canGoBack(layout, "a", [])).toBe(false)
    expect(canGoBack(layout, "b", [])).toBe(false)
    expect(canGoBack(layout, "f", [])).toBe(true)
    expect(canGoBack(layout, "b", ["a"])).toBe(true)
  })
})

describe("the editor's layout history", () => {
  beforeEach(resetLayoutHistory)

  it("is kept per profile and model", () => {
    pushLayoutHistory("p", "m", "one")
    pushLayoutHistory("p", "m", "two")
    expect(peekLayoutHistory("q", "m")).toBeUndefined()
    expect(popLayoutHistory("p", "m")).toBe("two")
    expect(peekLayoutHistory("p", "m")).toBe("one")
  })
})
