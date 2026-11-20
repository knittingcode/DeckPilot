// What the Page Navigator reads about the page tree, and the names it
// shows (page-navigator.tsx).

import { useEffect, useState } from "react"

import { i18n } from "@/i18n"
import { api, errorMessage } from "@/lib/api"
import type { DeviceLayout, PageTree } from "@/lib/types"

/** What the tree depends on: pages, their folders and the keys between them. */
function structureKey(layout: DeviceLayout): string {
  return JSON.stringify([
    layout.model,
    layout.homePage,
    layout.explicitTree ?? false,
    layout.pages.map((p) => [
      p.id,
      p.name,
      p.parent ?? null,
      p.bindings
        .filter((b) => b.target.type === "openPage" || b.target.type === "back")
        .map((b) => [b.control, b.target]),
    ]),
  ])
}

export const pageName = (name: string) => name || i18n.t("editor:pageNav.untitled")

/** The message for a page moved into `folder` (none: the top level). */
export function movedMessage(name: string, folder?: string): string {
  return folder === undefined
    ? i18n.t("editor:pageTree.moved.top", { page: name })
    : i18n.t("editor:pageTree.moved.page", { page: name, target: folder })
}

/** The page tree of a (possibly unsaved) layout, from the backend. */
export function usePageTree(layout: DeviceLayout) {
  const key = structureKey(layout)
  const [tree, setTree] = useState<{ key: string; tree: PageTree } | null>(null)
  const [problem, setProblem] = useState<string | null>(null)
  useEffect(() => {
    let current = true
    api.pageTree(layout).then(
      (next) => {
        if (!current) return
        setTree({ key, tree: next })
        setProblem(null)
      },
      (e: unknown) => {
        if (current) setProblem(errorMessage(e))
      },
    )
    return () => {
      current = false
    }
    // Only structural changes change the tree.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])
  return { tree: tree?.tree ?? null, problem }
}
