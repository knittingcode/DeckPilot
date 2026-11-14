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
