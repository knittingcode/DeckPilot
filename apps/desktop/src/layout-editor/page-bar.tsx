import { ArrowLeft, ChevronRight, Folder, FolderPlus, Home, Trash2 } from "lucide-react"
import { useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { page as findPage } from "@/lib/layout-model"
import type { DeviceLayout } from "@/lib/types"

interface PageBarProps {
  draft: DeviceLayout
  pageId: string
  /** Pages opened before the current one, oldest first. */
  trail: string[]
  onOpen: (pageId: string) => void
  onBack: () => void
  onAdd: (name: string) => void
  onRename: (name: string) => void
  onDelete: () => void
}

/** Page tabs, the page indicator with back navigation, and page settings. */
export function PageBar({
  draft,
  pageId,
  trail,
  onOpen,
  onBack,
  onAdd,
  onRename,
  onDelete,
}: PageBarProps) {
  const [newPage, setNewPage] = useState("")
  const current = findPage(draft, pageId)
  const position = draft.pages.findIndex((p) => p.id === pageId) + 1
  const isHome = pageId === draft.homePage
  const path = [...trail, pageId]
    .map((id) => findPage(draft, id))
    .filter((p): p is NonNullable<typeof p> => p !== undefined)
  if (path[0]?.id !== draft.homePage && !isHome) {
    const home = findPage(draft, draft.homePage)
    if (home) path.unshift(home)
  }

  return (
    <div className="grid gap-3">
      <div className="flex flex-wrap items-center gap-1" role="tablist" aria-label="Pages">
        {draft.pages.map((p) => (
          <Button
            key={p.id}
            role="tab"
            aria-selected={p.id === pageId}
            variant={p.id === pageId ? "secondary" : "ghost"}
            size="sm"
            onClick={() => {
              onOpen(p.id)
            }}
          >
            {p.id === draft.homePage ? <Home /> : <Folder />}
            {p.name || "Untitled"}
          </Button>
        ))}
        <form
          className="flex items-center gap-1"
          onSubmit={(e) => {
            e.preventDefault()
            if (!newPage.trim()) return
            onAdd(newPage.trim())
            setNewPage("")
          }}
        >
          <Input
            aria-label="New page name"
            className="h-8 w-32"
            placeholder="New folder"
            value={newPage}
            onChange={(e) => {
              setNewPage(e.target.value)
            }}
          />
          <Button
            type="submit"
            variant="ghost"
            size="icon"
            aria-label="Add page"
            disabled={!newPage.trim()}
          >
            <FolderPlus />
          </Button>
        </form>
      </div>

      <nav className="flex flex-wrap items-center gap-2 text-sm" aria-label="Page location">
        {!isHome && (
          <Button variant="outline" size="sm" onClick={onBack}>
            <ArrowLeft /> Back
          </Button>
        )}
        <ol className="flex flex-wrap items-center gap-1 text-muted-foreground">
          {path.map((p, index) => (
            <li key={`${p.id}-${index}`} className="flex items-center gap-1">
              {index > 0 && <ChevronRight className="size-3" aria-hidden />}
              {index === path.length - 1 ? (
                <span className="font-medium text-foreground" aria-current="page">
                  {p.name || "Untitled"}
                </span>
              ) : (
                <button
                  type="button"
                  className="rounded-sm hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  onClick={() => {
                    onOpen(p.id)
                  }}
                >
                  {p.name || "Untitled"}
                </button>
              )}
            </li>
          ))}
        </ol>
        <span className="ml-auto text-xs text-muted-foreground" aria-live="polite">
          Page {position} of {draft.pages.length}
        </span>
      </nav>

      {current && !isHome && (
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <Label htmlFor="page-name">Page name</Label>
          <Input
            id="page-name"
            className="h-8 w-40"
            value={current.name}
            onChange={(e) => {
              onRename(e.target.value)
            }}
          />
          <Button variant="ghost" size="sm" onClick={onDelete}>
            <Trash2 /> Delete page
          </Button>
        </div>
      )}
    </div>
  )
}
