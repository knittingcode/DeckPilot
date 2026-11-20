// The page tree of a layout as the page tree panel shows it, from the
// backend's `page_tree`.

import type { DeviceLayout, PageNode, PageTree } from "@/lib/types"

export interface Row {
  node: PageNode
  children: Row[]
}

/** The nodes as a tree: home first, then the other top-level pages, in layout order. */
export function treeRows(tree: PageTree, homePage: string): Row[] {
  const byParent = new Map<string | null, PageNode[]>()
  for (const node of tree.pages) {
    const list = byParent.get(node.parent) ?? []
    list.push(node)
    byParent.set(node.parent, list)
  }
  const build = (node: PageNode, seen: Set<string>): Row => {
    seen.add(node.id)
    return {
      node,
      children: (byParent.get(node.id) ?? [])
        .filter((child) => !seen.has(child.id))
        .map((child) => build(child, seen)),
    }
  }
  const roots = [...(byParent.get(null) ?? [])].sort(
    (a, b) => Number(b.id === homePage) - Number(a.id === homePage),
  )
  const seen = new Set<string>()
  return roots.map((root) => build(root, seen))
}

/** `page` and every page inside it. */
export function subtree(tree: PageTree, page: string): Set<string> {
  const inside = new Set([page])
  let grew = true
  while (grew) {
    grew = false
    for (const node of tree.pages) {
      if (node.parent && inside.has(node.parent) && !inside.has(node.id)) {
        inside.add(node.id)
        grew = true
      }
    }
  }
  return inside
}

/** The page order after swapping `page` with its previous (-1) or next (+1) sibling. */
export function reorderedSibling(
  layout: DeviceLayout,
  tree: PageTree,
  page: string,
  direction: -1 | 1,
): string[] | null {
  const parent = tree.pages.find((n) => n.id === page)?.parent ?? null
  const siblings = layout.pages
    .map((p) => p.id)
    .filter((id) => id !== layout.homePage)
    .filter((id) => (tree.pages.find((n) => n.id === id)?.parent ?? null) === parent)
  const index = siblings.indexOf(page)
  const other = siblings[index + direction]
  if (index < 0 || other === undefined) return null
  const order = layout.pages.map((p) => p.id)
  const a = order.indexOf(page)
  const b = order.indexOf(other)
  order[a] = other
  order[b] = page
  return order
}

/** Where a dragged page goes against the row it is over. */
export type DropPosition = "before" | "after" | "inside"

/** What a drop does: a new page order (the same folder) or a move to another folder. */
export type PlannedDrop =
  { kind: "reorder"; order: string[] } | { kind: "move"; parent: string | null }

/**
 * Which part of a row the pointer is over: its top and bottom quarters
 * place the page beside it, the middle puts it inside.
 */
export function dropPosition(y: number, top: number, height: number): DropPosition {
  const at = height > 0 ? (y - top) / height : 0.5
  return at < 0.25 ? "before" : at > 0.75 ? "after" : "inside"
}

/**
 * What dropping `page` at `position` of `target` does, or `null` when it
 * is not a valid move: the home page never moves and nothing goes before
 * or after it, a page does not go into itself or a page inside it, and a
 * drop that changes nothing does nothing.
 */
export function plannedDrop(
  layout: DeviceLayout,
  tree: PageTree,
  page: string,
  target: string,
  position: DropPosition,
): PlannedDrop | null {
  if (page === layout.homePage || page === target) return null
  if (subtree(tree, page).has(target)) return null
  const parentOf = (id: string) => tree.pages.find((n) => n.id === id)?.parent ?? null
  const from = parentOf(page)
  if (position === "inside") return from === target ? null : { kind: "move", parent: target }
  if (target === layout.homePage) return null
  const to = parentOf(target)
  if (to !== from) return { kind: "move", parent: to }
  const before = layout.pages.map((p) => p.id)
  const order = before.filter((id) => id !== page)
  order.splice(order.indexOf(target) + (position === "after" ? 1 : 0), 0, page)
  return order.every((id, index) => id === before[index]) ? null : { kind: "reorder", order }
}
