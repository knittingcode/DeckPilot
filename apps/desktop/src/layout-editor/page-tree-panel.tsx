import { ArrowDown, ArrowUp, FolderInput } from "lucide-react"
import { useEffect, useMemo, useState } from "react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { api, errorMessage } from "@/lib/api"
import type { DeviceLayout, PageChange, PageNode, PageTree } from "@/lib/types"
import { cn } from "@/lib/utils"

import { type Row, reorderedSibling, subtree, treeRows } from "./page-tree"

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

const pageName = (name: string) => name || "Untitled"

/** The page tree of a (possibly unsaved) layout, from the backend. */
function usePageTree(layout: DeviceLayout) {
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

/**
 * The page tree of the layout with reordering, moving and copying pages
 * (with everything inside them) within the layout or to another layout.
 * These commands store the layout, so they need the draft to be saved.
 */
export function PageTreePanel({
  profileId,
  draft,
  layouts,
  dirty,
  pageId,
  onOpen,
  onStored,
}: {
  profileId: string
  draft: DeviceLayout
  /** Every layout of the profile, as stored. */
  layouts: DeviceLayout[]
  dirty: boolean
  pageId: string
  onOpen: (page: string) => void
  /**
   * A command stored layouts of the profile: `layout` is this layout as
   * stored (`null` when it did not change); show `page` afterwards.
   */
  onStored: (layout: DeviceLayout | null, page?: string) => void
}) {
  const { tree, problem } = usePageTree(draft)
  const [moving, setMoving] = useState<PageNode | null>(null)
  const [busy, setBusy] = useState(false)
  const rows = useMemo(() => (tree ? treeRows(tree, draft.homePage) : []), [tree, draft.homePage])
  const locked = dirty || busy

  const reorder = (page: PageNode, direction: -1 | 1) => {
    if (!tree) return
    const order = reorderedSibling(draft, tree, page.id, direction)
    if (!order) return
    setBusy(true)
    api
      .reorderPages(profileId, draft.id, order)
      .then(
        (layout) => {
          onStored(layout, pageId)
        },
        (e: unknown) => {
          toast.error(`“${pageName(page.name)}” was not moved ${direction < 0 ? "up" : "down"}`, {
            description: errorMessage(e),
          })
        },
      )
      .finally(() => {
        setBusy(false)
      })
  }

  const renderRows = (list: Row[]) => (
    <ul className="grid gap-0.5">
      {list.map(({ node, children }, index) => {
        const home = node.id === draft.homePage
        const name = pageName(node.name)
        return (
          <li key={node.id}>
            <div
              className={cn(
                "flex items-center gap-1 rounded-md pr-1 text-sm",
                node.id === pageId && "bg-accent",
              )}
            >
              <button
                type="button"
                className="min-w-0 flex-1 truncate rounded-md px-2 py-1 text-left hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                aria-current={node.id === pageId ? "page" : undefined}
                onClick={() => {
                  onOpen(node.id)
                }}
              >
                {name}
                {node.automaticBack && (
                  <span className="ml-2 text-xs text-muted-foreground">automatic Back</span>
                )}
              </button>
              {!home && (
                <>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Move “${name}” up`}
                    disabled={locked || index === 0 || list[index - 1]?.node.id === draft.homePage}
                    onClick={() => {
                      reorder(node, -1)
                    }}
                  >
                    <ArrowUp />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Move “${name}” down`}
                    disabled={locked || index === list.length - 1}
                    onClick={() => {
                      reorder(node, 1)
                    }}
                  >
                    <ArrowDown />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-7"
                    aria-label={`Move or copy “${name}”`}
                    disabled={locked}
                    onClick={() => {
                      setMoving(node)
                    }}
                  >
                    <FolderInput />
                  </Button>
                </>
              )}
            </div>
            {children.length > 0 && (
              <div className="ml-4 border-l pl-1">{renderRows(children)}</div>
            )}
          </li>
        )
      })}
    </ul>
  )

  return (
    <section className="grid gap-2" aria-label="Page tree">
      <h3 className="text-xs font-medium text-muted-foreground uppercase">Page tree</h3>
      {problem && (
        <p role="alert" className="text-xs text-destructive">
          The page tree could not be read: {problem}
        </p>
      )}
      {tree && renderRows(rows)}
      {dirty && (
        <p className="text-xs text-muted-foreground">
          Save or revert your changes to reorder, move or copy pages.
        </p>
      )}
      {tree && tree.warnings.length > 0 && (
        <ul className="grid gap-1 text-xs text-muted-foreground" aria-label="Page tree warnings">
          {tree.warnings.map((warning) => (
            <li key={warning}>{warning}</li>
          ))}
        </ul>
      )}
      {moving && tree && (
        <MoveDialog
          profileId={profileId}
          layout={draft}
          layouts={layouts}
          tree={tree}
          page={moving}
          onClose={() => {
            setMoving(null)
          }}
          onDone={(change, kind, target) => {
            setMoving(null)
            const here = change.layouts.find((l) => l.id === draft.id)
            const name = pageName(moving.name)
            const removed =
              change.removedKeys > 0
                ? ` ${change.removedKeys} key${change.removedKeys === 1 ? "" : "s"} that opened it ${change.removedKeys === 1 ? "was" : "were"} removed.`
                : ""
            toast.success(
              kind === "move" ? `“${name}” moved to ${target}` : `“${name}” copied to ${target}`,
              removed ? { description: removed.trim() } : undefined,
            )
            // Within this layout, show the page where it went.
            const shown = here?.pages.some((p) => p.id === change.page) ? change.page : undefined
            onStored(here ?? null, shown)
          }}
        />
      )}
    </section>
  )
}

/** `layoutId:pageId`; an empty page: the top level here, the home page elsewhere. */
type Destination = string

function MoveDialog({
  profileId,
  layout,
  layouts,
  tree,
  page,
  onClose,
  onDone,
}: {
  profileId: string
  layout: DeviceLayout
  layouts: DeviceLayout[]
  tree: PageTree
  page: PageNode
  onClose: () => void
  onDone: (change: PageChange, kind: "move" | "copy", target: string) => void
}) {
  const inside = subtree(tree, page.id)
  const others = layouts.filter((l) => l.id !== layout.id)
  const [destination, setDestination] = useState<Destination>(`${layout.id}:`)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const name = pageName(page.name)

  const label = (value: Destination): string => {
    const [layoutId = "", target = ""] = value.split(":")
    const destinationLayout = layouts.find((l) => l.id === layoutId) ?? layout
    const targetPage = destinationLayout.pages.find((p) => p.id === target)
    const where = targetPage ? `“${pageName(targetPage.name)}”` : "the top level"
    if (layoutId === layout.id) return where
    return targetPage
      ? `${where} in “${destinationLayout.name}”`
      : `the home page of “${destinationLayout.name}”`
  }

  const run = (kind: "move" | "copy") => {
    const [layoutId = "", target = ""] = destination.split(":")
    const targetLayout = layoutId === layout.id ? null : layoutId
    const parent = target || null
    setBusy(true)
    setError(null)
    const call = kind === "move" ? api.movePage : api.copyPage
    call(profileId, layout.id, page.id, targetLayout, parent)
      .then(
        (change) => {
          onDone(change, kind, label(destination))
        },
        (e: unknown) => {
          setError(`“${name}” was not ${kind === "move" ? "moved" : "copied"}: ${errorMessage(e)}`)
        },
      )
      .finally(() => {
        setBusy(false)
      })
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Move or copy “{name}”</DialogTitle>
          <DialogDescription>
            The page goes with every page inside it. Moving it to another layout removes the keys
            here that opened it.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <Label htmlFor="page-destination">Destination</Label>
          <Select value={destination} onValueChange={setDestination}>
            <SelectTrigger id="page-destination" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                <SelectLabel>{layout.name} (this layout)</SelectLabel>
                <SelectItem value={`${layout.id}:`}>Top level</SelectItem>
                {layout.pages
                  .filter((p) => !inside.has(p.id))
                  .map((p) => (
                    <SelectItem key={p.id} value={`${layout.id}:${p.id}`}>
                      Inside “{pageName(p.name)}”
                    </SelectItem>
                  ))}
              </SelectGroup>
              {others.map((other) => (
                <SelectGroup key={other.id}>
                  <SelectLabel>{other.name}</SelectLabel>
                  <SelectItem value={`${other.id}:`}>Home page of “{other.name}”</SelectItem>
                  {other.pages
                    .filter((p) => p.id !== other.homePage)
                    .map((p) => (
                      <SelectItem key={p.id} value={`${other.id}:${p.id}`}>
                        Inside “{pageName(p.name)}” in “{other.name}”
                      </SelectItem>
                    ))}
                </SelectGroup>
              ))}
            </SelectContent>
          </Select>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => {
              run("copy")
            }}
          >
            Copy
          </Button>
          <Button
            disabled={busy}
            onClick={() => {
              run("move")
            }}
          >
            Move
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
