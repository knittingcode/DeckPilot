import {
  ArrowLeft,
  ChevronLeft,
  ChevronRight,
  File,
  Folder,
  Home,
  ListTree,
  Plus,
} from "lucide-react"
import type { ReactNode } from "react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { ancestors, page as findPage, topLevelPages } from "@/lib/layout-model"
import type { DeviceLayout } from "@/lib/types"

import { newPageName, pageNavigation, pagePath } from "./page-navigation"

const pageName = (name: string | undefined) => name || "Untitled"

interface PageNavProps {
  draft: DeviceLayout
  pageId: string
  /** Pages opened before the current one, oldest first; empty when opened directly. */
  trail: string[]
  onOpen: (pageId: string) => void
  onBack: () => void
  onAdd: (name: string) => void
  /** Opens the page settings and the page tree. */
  onManage: () => void
  /** A line under the navigation, e.g. the state of live values. */
  status?: ReactNode
}

/**
 * The page navigation under the device preview: previous and next top-level
 * page, the page shown (a menu of every page), a new page, and the pages'
 * settings and tree. Inside a folder, where it is and the way back.
 */
export function PageNav({
  draft,
  pageId,
  trail,
  onOpen,
  onBack,
  onAdd,
  onManage,
  status,
}: PageNavProps) {
  const { number, count, previous, next } = pageNavigation(draft, pageId)
  const current = findPage(draft, pageId)
  const name = pageName(current?.name)
  const top = new Set(topLevelPages(draft))
  // A page inside a folder, or reached through one, has somewhere to return to.
  const nested = trail.length > 0 || ancestors(draft, pageId).length > 0
  const path = nested ? pagePath(draft, pageId, trail) : []
  const nameOf = (id: string | undefined) => pageName(id ? findPage(draft, id)?.name : undefined)

  return (
    <div data-slot="page-nav" className="grid justify-items-center gap-1.5">
      <div
        role="toolbar"
        aria-label="Page navigation"
        className="flex items-center gap-1 rounded-lg border bg-card p-0.5 shadow-xs"
      >
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Previous page"
          title={previous ? `Previous page: ${nameOf(previous)}` : "This is the first page"}
          disabled={!previous}
          onClick={() => {
            if (previous) onOpen(previous)
          }}
        >
          <ChevronLeft />
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              className="h-8 min-w-12 px-2 font-medium tabular-nums"
              title={`${name}, page ${number} of ${count}. Go to another page`}
            >
              <span aria-hidden>
                {number}
                <span className="text-muted-foreground"> / {count}</span>
              </span>
              <span className="sr-only">
                Page {number} of {count}
              </span>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="center" className="max-h-80 min-w-52">
            <DropdownMenuLabel className="text-xs text-muted-foreground">
              Go to page
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuRadioGroup value={pageId} onValueChange={onOpen}>
              {draft.pages.map((p) => {
                const depth = ancestors(draft, p.id).length
                const Icon = p.id === draft.homePage ? Home : top.has(p.id) ? File : Folder
                return (
                  <DropdownMenuRadioItem
                    key={p.id}
                    value={p.id}
                    style={{ paddingLeft: `${2 + depth * 0.75}rem` }}
                  >
                    <Icon className="text-muted-foreground" aria-hidden />
                    <span className="truncate">{pageName(p.name)}</span>
                  </DropdownMenuRadioItem>
                )
              })}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Next page"
          title={next ? `Next page: ${nameOf(next)}` : "This is the last page"}
          disabled={!next}
          onClick={() => {
            if (next) onOpen(next)
          }}
        >
          <ChevronRight />
        </Button>
        <span className="mx-0.5 h-5 w-px bg-border" aria-hidden />
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Add page"
          title="Add a page after the last one and show it"
          onClick={() => {
            onAdd(newPageName(draft))
          }}
        >
          <Plus />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Manage pages"
          title="Pages: rename, delete, reorder, move or copy"
          onClick={onManage}
        >
          <ListTree />
        </Button>
      </div>

      {nested ? (
        <nav
          className="flex max-w-full min-w-0 items-center gap-1.5 text-xs"
          aria-label="Page location"
        >
          <Button variant="outline" size="xs" onClick={onBack}>
            <ArrowLeft /> Back
          </Button>
          <ol className="flex min-w-0 flex-wrap items-center gap-1 text-muted-foreground">
            {path.map((p, index) => (
              <li key={`${p.id}-${index}`} className="flex min-w-0 items-center gap-1">
                {index > 0 && <ChevronRight className="size-3 shrink-0" aria-hidden />}
                {index === path.length - 1 ? (
                  <span
                    className="truncate font-medium text-foreground"
                    aria-current="page"
                    data-slot="page-name"
                  >
                    {pageName(p.name)}
                  </span>
                ) : (
                  <button
                    type="button"
                    className="truncate rounded-sm hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    onClick={() => {
                      onOpen(p.id)
                    }}
                  >
                    {pageName(p.name)}
                  </button>
                )}
              </li>
            ))}
          </ol>
        </nav>
      ) : (
        <p className="max-w-full truncate text-xs font-medium" data-slot="page-name">
          {name}
        </p>
      )}
      {status}
    </div>
  )
}
