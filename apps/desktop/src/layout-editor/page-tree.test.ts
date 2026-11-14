import { describe, expect, it } from "vitest"

import type { DeviceLayout, PageNode, PageTree } from "@/lib/types"

import { reorderedSibling, subtree, treeRows } from "./page-tree"

const node = (id: string, parent: string | null): PageNode => ({
  id,
  name: id,
  parent,
  stored: false,
  depth: 0,
  automaticBack: null,
})

// Layout order: a, home, b, c (inside a), d (inside a), e (inside c).
const tree: PageTree = {
  pages: [
    node("a", null),
    node("home", null),
    node("b", null),
    node("c", "a"),
    node("d", "a"),
    node("e", "c"),
  ],
  warnings: [],
}
const layout = {
  id: "l",
  name: "Main",
  model: "m",
  homePage: "home",
  pages: tree.pages.map((p) => ({ id: p.id, name: p.name, bindings: [] })),
} satisfies DeviceLayout

const shape = (rows: ReturnType<typeof treeRows>): unknown =>
  rows.map((r) => (r.children.length ? [r.node.id, shape(r.children)] : r.node.id))

describe("page tree", () => {
  it("nests pages under their folders with home first", () => {
    expect(shape(treeRows(tree, "home"))).toEqual(["home", ["a", [["c", ["e"]], "d"]], "b"])
  })

  it("survives a cycle in the data", () => {
    const cyclic: PageTree = {
      pages: [node("home", null), node("x", "y"), node("y", "x")],
      warnings: [],
    }
    expect(shape(treeRows(cyclic, "home"))).toEqual(["home"])
  })

  it("finds everything inside a page", () => {
    expect([...subtree(tree, "a")].sort()).toEqual(["a", "c", "d", "e"])
    expect([...subtree(tree, "b")]).toEqual(["b"])
  })

  it("swaps a page with its sibling in the layout order", () => {
    expect(reorderedSibling(layout, tree, "b", -1)).toEqual(["b", "home", "a", "c", "d", "e"])
    expect(reorderedSibling(layout, tree, "d", -1)).toEqual(["a", "home", "b", "d", "c", "e"])
    expect(reorderedSibling(layout, tree, "c", 1)).toEqual(["a", "home", "b", "d", "c", "e"])
    // No sibling in that direction; home is never a sibling.
    expect(reorderedSibling(layout, tree, "a", -1)).toBeNull()
    expect(reorderedSibling(layout, tree, "e", 1)).toBeNull()
  })
})
