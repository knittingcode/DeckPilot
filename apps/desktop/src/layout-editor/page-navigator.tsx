import {
  DndContext,
  PointerSensor,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragMoveEvent,
} from "@dnd-kit/core"
import {
  Check,
  ChevronRight,
  Copy,
  File,
  FilePlus,
  Folder,
  Home,
  MoreHorizontal,
  Pencil,
  Trash2,
} from "lucide-react"
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react"
import { toast } from "sonner"

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useTranslation } from "@/i18n"
import { api, errorMessage } from "@/lib/api"
import { ancestors, page as findPage, pageParents } from "@/lib/layout-model"
import type { DeviceLayout, PageNode, PageTree } from "@/lib/types"
import { cn } from "@/lib/utils"

import {
  type DropPosition,
  type PlannedDrop,
  type Row,
  dropPosition,
  plannedDrop,
  reorderedSibling,
  treeRows,
} from "./page-tree"
import { movedMessage, pageName, usePageTree } from "./page-tree-data"
import { RenameField } from "./rename-field"

/** Commands of the navigator: the editor's own (draft) and stored ones. */
export interface NavigatorCommands {
  /** Shows a page (from the top: no folder trail). */
  open: (page: string) => void
  /** The editor's page rename (the breadcrumb renames through it too). */
  rename: (page: string, name: string) => void
  /** Asks before deleting (the editor's deletion dialog). */
  remove: (page: string) => void
  /** A new page in `folder`, shown at once. */
  addIn: (folder: string, name: string) => void
  setAutoReturn: (page: string, on: boolean) => void
  /** A command stored the layout: `layout` as stored (`null`: unchanged); show `page`. */
  stored: (layout: DeviceLayout | null, page?: string) => void
}

/**
 * The layout's pages as a tree, from the draft (the editor's data model;
 * the same parents the agent uses), with what only the backend knows (the
 * automatic Back keys) added when it has answered.
 */
function useNavigatorTree(draft: DeviceLayout): PageTree & { problem: string | null } {
  const backend = usePageTree(draft)
  return useMemo(() => {
    const parents = pageParents(draft)
    const known = new Map(backend.tree?.pages.map((n) => [n.id, n]) ?? [])
    return {
      pages: draft.pages.map((p) => ({
        id: p.id,
        name: p.name,
        parent: p.id === draft.homePage ? null : (parents.get(p.id) ?? null),
        stored: Boolean(p.parent),
        depth: ancestors(draft, p.id).length,
        automaticBack: known.get(p.id)?.automaticBack ?? null,
      })),
      warnings: backend.tree?.warnings ?? [],
      problem: backend.problem,
    }
  }, [draft, backend.tree, backend.problem])
}

interface Visible {
  row: Row
  level: number
  parent: string | null
}

function flatten(rows: Row[], expanded: (id: string) => boolean): Visible[] {
  const out: Visible[] = []
  const walk = (list: Row[], level: number, parent: string | null) => {
    for (const row of list) {
      out.push({ row, level, parent })
      if (row.children.length > 0 && expanded(row.node.id))
        walk(row.children, level + 1, row.node.id)
    }
  }
  walk(rows, 1, null)
  return out
}

interface MenuEntry {
  key: string
  label: string
  icon: ComponentType<{ className?: string }>
  destructive?: boolean
  separator?: boolean
  /** A setting turned on or off (a checkbox item). */
  checked?: boolean
  run: () => void
}

/** The same entries in a context menu and in the row's "⋯" menu. */
function MenuItems({ entries, kind }: { entries: MenuEntry[]; kind: "context" | "dropdown" }) {
  const Item = kind === "context" ? ContextMenuItem : DropdownMenuItem
  const Separator = kind === "context" ? ContextMenuSeparator : DropdownMenuSeparator
  return entries.map((entry) => {
    const Icon = entry.icon
    return (
      <div key={entry.key} className="contents">
        {entry.separator && <Separator />}
        <Item
          data-entry={entry.key}
          className="h-8 py-0"
          variant={entry.destructive ? "destructive" : "default"}
          onSelect={entry.run}
          {...(entry.checked === undefined
            ? {}
            : { role: "menuitemcheckbox", "aria-checked": entry.checked })}
        >
          <Icon className={cn("size-4", entry.checked === false && "invisible")} />
          <span className="truncate">{entry.label}</span>
        </Item>
      </div>
    )
  })
}

const MENU = "w-auto min-w-40 max-w-64"

/** A row as dnd-kit sees it: dragged by the row itself, and a drop target. */
function DndRow({
  id,
  draggable,
  children,
}: {
  id: string
  draggable: boolean
  children: (bind: {
    ref: (element: HTMLElement | null) => void
    onPointerDown?: (event: ReactPointerEvent) => void
    /** The hidden how-to (dnd-kit's instructions), for the row's description. */
    describedBy?: string
    dragging: boolean
  }) => ReactNode
}) {
  const drag = useDraggable({ id, disabled: !draggable })
  const drop = useDroppable({ id })
  const onPointerDown = (
    drag.listeners as { onPointerDown?: (e: ReactPointerEvent) => void } | undefined
  )?.onPointerDown
  return children({
    ref: (element) => {
      drag.setNodeRef(element)
      drop.setNodeRef(element)
    },
    onPointerDown: draggable ? onPointerDown : undefined,
    describedBy: draggable ? drag.attributes["aria-describedby"] : undefined,
    dragging: drag.isDragging,
  })
}

/** The drop under the pointer: the row it is over and where against it. */
function dropAt(event: DragMoveEvent | DragEndEvent): { target: string; position: DropPosition } {
  const { over, activatorEvent, delta } = event
  if (!over) return { target: "", position: "inside" }
  const y =
    (activatorEvent as PointerEvent | null)?.clientY !== undefined
      ? (activatorEvent as PointerEvent).clientY + delta.y
      : over.rect.top + over.rect.height / 2
  return { target: String(over.id), position: dropPosition(y, over.rect.top, over.rect.height) }
}

/**
 * The Page Navigator: the content of the page counter's popover. The
 * layout's page tree (the home page, folders and the pages in them) as an
 * ARIA tree: a click or Enter shows the page (editor navigation only,
 * nothing runs), the chevron opens and closes a folder, and each row has
 * the commands it can take in a context menu and a "⋯" menu. Pages are
 * reordered or moved into a folder by dragging them (or Alt+Up/Down
 * among their siblings); like duplicating, that stores the layout, so it
 * needs the draft to be saved first.
 */
export function PageNavigator({
  profileId,
  draft,
  dirty,
  pageId,
  commands,
  newPageName,
  onClose,
}: {
  profileId: string
  draft: DeviceLayout
  dirty: boolean
  pageId: string
  commands: NavigatorCommands
  newPageName: () => string
  /** Closes the popover (after showing a page). */
  onClose: () => void
}) {
  const { t } = useTranslation()
  const tree = useNavigatorTree(draft)
  const rows = useMemo(() => treeRows(tree, draft.homePage), [tree, draft.homePage])
  // Folders closed by the user; everything is open otherwise.
  const [closed, setClosed] = useState<Set<string>>(() => new Set())
  const visible = flatten(rows, (id) => !closed.has(id))
  const [focusId, setFocusId] = useState(pageId)
  const focused = visible.some((v) => v.row.node.id === focusId)
    ? focusId
    : (visible[0]?.row.node.id ?? pageId)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [announcement, setAnnouncement] = useState("")
  // The drag under way: the page, and the drop it would make where it is.
  const [drag, setDrag] = useState<{
    page: string
    target: string
    position: DropPosition
    plan: PlannedDrop | null
  } | null>(null)
  const locked = dirty || busy
  const treeRef = useRef<HTMLUListElement>(null)
  const keepFocus = useRef(false)
  const onCloseAutoFocus = (event: Event) => {
    if (!keepFocus.current) return
    keepFocus.current = false
    event.preventDefault()
  }
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }))

  const focusRow = (id: string) => {
    setFocusId(id)
    treeRef.current?.querySelector<HTMLElement>(`[data-page-id="${CSS.escape(id)}"]`)?.focus()
  }
  // The open page has focus when the navigator opens.
  useEffect(() => {
    treeRef.current
      ?.querySelector<HTMLElement>(`[data-page-id="${CSS.escape(pageId)}"]`)
      ?.focus({ preventScroll: false })
    // Once, when it opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const show = (id: string) => {
    commands.open(id)
    onClose()
  }
  const toggle = (id: string, open?: boolean) => {
    setClosed((current) => {
      const next = new Set(current)
      const isOpen = !current.has(id)
      if (open ?? !isOpen) next.delete(id)
      else next.add(id)
      return next
    })
  }
  const nameOf = (id: string) => pageName(findPage(draft, id)?.name ?? "")

  /** Stores a new order or a move (the same commands as ever); `done` on success. */
  const place = (page: string, plan: PlannedDrop, done: () => void, failed: string) => {
    if (locked) {
      toast.error(t("editor:pageTree.saveFirst"))
      return
    }
    setBusy(true)
    const request =
      plan.kind === "reorder"
        ? api.reorderPages(profileId, draft.id, plan.order).then((layout) => {
            commands.stored(layout, pageId)
          })
        : api.movePage(profileId, draft.id, page, null, plan.parent).then((change) => {
            toast.success(
              movedMessage(nameOf(page), plan.parent === null ? undefined : nameOf(plan.parent)),
            )
            commands.stored(change.layouts.find((l) => l.id === draft.id) ?? null, pageId)
          })
    request
      .then(done, (e: unknown) => {
        toast.error(failed, { description: errorMessage(e) })
      })
      .finally(() => {
        setBusy(false)
      })
  }

  /** Alt+Up/Down: one place up or down among its siblings. */
  const reorder = (page: string, direction: -1 | 1) => {
    const order = reorderedSibling(draft, tree, page, direction)
    if (!order) return
    const values = { page: nameOf(page) }
    place(
      page,
      { kind: "reorder", order },
      () => {
        setAnnouncement(
          direction < 0
            ? t("editor:navigator.movedUp", values)
            : t("editor:navigator.movedDown", values),
        )
        requestAnimationFrame(() => {
          focusRow(page)
        })
      },
      direction < 0
        ? t("editor:pageTree.notMovedUp", values)
        : t("editor:pageTree.notMovedDown", values),
    )
  }

  const duplicate = (page: PageNode) => {
    setBusy(true)
    api
      .copyPage(profileId, draft.id, page.id, null, page.parent)
      .then(
        (change) => {
          const here = change.layouts.find((l) => l.id === draft.id)
          toast.success(t("editor:navigator.duplicated", { page: pageName(page.name) }))
          commands.stored(here ?? null, change.page)
        },
        (e: unknown) => {
          toast.error(
            t("editor:pageTree.copyFailed", { page: pageName(page.name), reason: errorMessage(e) }),
          )
        },
      )
      .finally(() => {
        setBusy(false)
      })
  }

  const onDragMove = (event: DragMoveEvent) => {
    const page = String(event.active.id)
    const { target, position } = dropAt(event)
    const plan = target ? plannedDrop(draft, tree, page, target, position) : null
    setDrag({ page, target, position, plan })
  }
  const onDragEnd = (event: DragEndEvent) => {
    setDrag(null)
    const page = String(event.active.id)
    const { target, position } = dropAt(event)
    const plan = target ? plannedDrop(draft, tree, page, target, position) : null
    if (!plan) return
    place(
      page,
      plan,
      () => {
        if (plan.kind === "reorder")
          setAnnouncement(t("editor:navigator.reordered", { page: nameOf(page) }))
      },
      t("editor:navigator.notMoved", { page: nameOf(page) }),
    )
  }

  /**
   * What a row can do, from what it is (its id and place in the tree, never
   * its name): the home page is renamed and gets new pages; any other page
   * is renamed, duplicated (once the draft is saved) and deleted, and a
   * folder gets new pages inside it.
   */
  const entriesFor = (v: Visible): MenuEntry[] => {
    const { node } = v.row
    const rename: MenuEntry = {
      key: "rename",
      label: t("editor:navigator.rename"),
      icon: Pencil,
      run: () => {
        // The field keeps focus: the closing menu does not take it back.
        keepFocus.current = true
        // Once the menu is gone (it holds focus while open).
        setTimeout(() => {
          setRenaming(node.id)
        }, 0)
      },
    }
    const addInside = (label: string): MenuEntry => ({
      key: "add",
      label,
      icon: FilePlus,
      run: () => {
        commands.addIn(node.id, newPageName())
        onClose()
      },
    })
    if (node.id === draft.homePage) return [rename, addInside(t("editor:pageNav.add"))]
    const page = findPage(draft, node.id)
    return [
      rename,
      ...(locked
        ? []
        : [
            {
              key: "duplicate",
              label: t("editor:navigator.duplicate"),
              icon: Copy,
              run: () => {
                duplicate(node)
              },
            },
          ]),
      ...(v.row.children.length > 0 ? [addInside(t("editor:navigator.newPage"))] : []),
      ...(v.level > 1
        ? [
            {
              key: "autoReturn",
              label: t("editor:pageSettings.autoReturn"),
              icon: Check,
              checked: page?.autoReturn === true,
              run: () => {
                commands.setAutoReturn(node.id, page?.autoReturn !== true)
              },
            },
          ]
        : []),
      {
        key: "delete",
        label: t("editor:pageSettings.delete"),
        icon: Trash2,
        destructive: true,
        separator: true,
        run: () => {
          onClose()
          commands.remove(node.id)
        },
      },
    ]
  }

  const onKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
    // Keys of a row's menu (a portal, bubbling here through React) are its own.
    if (renaming || !(event.target instanceof Node) || !event.currentTarget.contains(event.target))
      return
    const at = visible.findIndex((v) => v.row.node.id === focused)
    const current = visible[at]
    if (!current) return
    const id = current.row.node.id
    const hasChildren = current.row.children.length > 0
    const go = (index: number) => {
      const target = visible[Math.max(0, Math.min(visible.length - 1, index))]
      if (target) focusRow(target.row.node.id)
    }
    if (event.altKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
      event.preventDefault()
      reorder(id, event.key === "ArrowUp" ? -1 : 1)
      return
    }
    switch (event.key) {
      case "ArrowDown":
        go(at + 1)
        break
      case "ArrowUp":
        go(at - 1)
        break
      case "Home":
        go(0)
        break
      case "End":
        go(visible.length - 1)
        break
      case "ArrowRight":
        if (hasChildren && closed.has(id)) toggle(id, true)
        else if (hasChildren) go(at + 1)
        break
      case "ArrowLeft":
        if (hasChildren && !closed.has(id)) toggle(id, false)
        else if (current.parent) focusRow(current.parent)
        break
      case "Enter":
      case " ":
        show(id)
        break
      case "F2":
        setRenaming(id)
        break
      default:
        return
    }
    event.preventDefault()
  }

  const silent = () => undefined
  return (
    <div className="flex min-h-0 flex-col" data-slot="page-navigator">
      {tree.problem && (
        <p role="alert" className="px-2 py-1 text-xs text-destructive">
          {t("editor:pageTree.unreadable", { problem: tree.problem })}
        </p>
      )}
      <DndContext
        id="page-tree"
        sensors={sensors}
        onDragStart={(event) => {
          setDrag({ page: String(event.active.id), target: "", position: "inside", plan: null })
        }}
        onDragMove={onDragMove}
        onDragEnd={onDragEnd}
        onDragCancel={() => {
          setDrag(null)
        }}
        // The navigator says what happened itself (its live region).
        accessibility={{
          container: document.body,
          screenReaderInstructions: { draggable: t("editor:navigator.dragHint") },
          announcements: {
            onDragStart: silent,
            onDragMove: silent,
            onDragOver: silent,
            onDragEnd: silent,
            onDragCancel: silent,
          },
        }}
      >
        {/* The scroll container is the containing block of its content. */}
        <div data-slot="page-navigator-scroll" className="relative min-h-0 overflow-y-auto">
          <ul
            ref={treeRef}
            role="tree"
            aria-label={t("editor:pageTree.label")}
            className="grid"
            onKeyDown={onKeyDown}
          >
            {visible.map((v) => {
              const { node } = v.row
              const home = node.id === draft.homePage
              const active = node.id === pageId
              const hasChildren = v.row.children.length > 0
              const open = !closed.has(node.id)
              const Icon = home ? Home : hasChildren ? Folder : File
              const name = pageName(node.name)
              const entries = entriesFor(v)
              const indent = 0.25 + (v.level - 1) * 0.875
              const dropHere = drag?.plan && drag.target === node.id ? drag.position : null
              return (
                <DndRow key={node.id} id={node.id} draggable={!home && renaming !== node.id}>
                  {(bind) => (
                    <ContextMenu>
                      <ContextMenuTrigger asChild>
                        <li
                          ref={bind.ref}
                          role="treeitem"
                          data-page-id={node.id}
                          data-drop={dropHere ?? undefined}
                          aria-level={v.level}
                          aria-expanded={hasChildren ? open : undefined}
                          aria-selected={active}
                          aria-current={active ? "page" : undefined}
                          aria-label={name}
                          aria-keyshortcuts={home ? undefined : "Alt+ArrowUp Alt+ArrowDown"}
                          aria-describedby={bind.describedBy}
                          tabIndex={node.id === focused ? 0 : -1}
                          onPointerDown={bind.onPointerDown}
                          onFocus={(event) => {
                            if (event.target === event.currentTarget) setFocusId(node.id)
                          }}
                          onKeyDown={(event) => {
                            if (
                              event.key === "ContextMenu" ||
                              (event.shiftKey && event.key === "F10")
                            ) {
                              event.preventDefault()
                              event.stopPropagation()
                              const box = event.currentTarget.getBoundingClientRect()
                              event.currentTarget.dispatchEvent(
                                new MouseEvent("contextmenu", {
                                  bubbles: true,
                                  cancelable: true,
                                  clientX: box.left + 16,
                                  clientY: box.top + box.height / 2,
                                }),
                              )
                            }
                          }}
                          className={cn(
                            "group relative flex h-8 cursor-default items-center gap-1.5 rounded-sm pr-1 text-sm text-foreground/90 outline-none select-none",
                            "hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                            active && "bg-accent/40 font-medium text-foreground",
                            bind.dragging && "opacity-50",
                            dropHere === "inside" && "bg-accent ring-1 ring-primary/60 ring-inset",
                          )}
                          style={{ paddingLeft: `${indent}rem` }}
                          onClick={(event) => {
                            const inRow =
                              event.target instanceof Node &&
                              event.currentTarget.contains(event.target)
                            if (inRow && renaming !== node.id) show(node.id)
                          }}
                        >
                          {(dropHere === "before" || dropHere === "after") && (
                            // Where the page goes: a line between the rows.
                            <span
                              aria-hidden
                              data-slot="drop-line"
                              className={cn(
                                "pointer-events-none absolute right-1 h-0.5 rounded-full bg-primary",
                                dropHere === "before" ? "-top-px" : "-bottom-px",
                              )}
                              style={{ left: `${indent}rem` }}
                            />
                          )}
                          <span
                            aria-hidden
                            data-slot="tree-chevron"
                            className={cn(
                              "-mr-0.5 flex size-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:text-foreground",
                              !hasChildren && "invisible",
                            )}
                            title={
                              open ? t("editor:navigator.collapse") : t("editor:navigator.expand")
                            }
                            onClick={(event) => {
                              event.stopPropagation()
                              toggle(node.id)
                            }}
                          >
                            <ChevronRight
                              className={cn("size-4 transition-transform", open && "rotate-90")}
                            />
                          </span>
                          <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                          {renaming === node.id ? (
                            <RenameField
                              initial={node.name}
                              onDone={(name) => {
                                setRenaming(null)
                                if (name !== null) commands.rename(node.id, name)
                                focusRow(node.id)
                              }}
                            />
                          ) : (
                            <span
                              className="min-w-0 flex-1 truncate"
                              data-slot="page-tree-name"
                              title={name}
                            >
                              {name}
                              {node.automaticBack && (
                                <span className="ml-2 text-xs font-normal text-muted-foreground">
                                  {t("editor:pageTree.automaticBack")}
                                </span>
                              )}
                            </span>
                          )}
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <button
                                type="button"
                                tabIndex={-1}
                                data-slot="row-actions"
                                aria-label={t("editor:navigator.rowActions", { page: name })}
                                title={t("editor:navigator.rowActions", { page: name })}
                                className="flex size-6 shrink-0 items-center justify-center rounded-sm text-muted-foreground opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 hover:bg-accent hover:text-foreground focus-visible:opacity-100 data-[state=open]:opacity-100"
                                onClick={(event) => {
                                  event.stopPropagation()
                                }}
                              >
                                <MoreHorizontal className="size-4" />
                              </button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent
                              align="end"
                              className={MENU}
                              onCloseAutoFocus={onCloseAutoFocus}
                            >
                              <MenuItems entries={entries} kind="dropdown" />
                            </DropdownMenuContent>
                          </DropdownMenu>
                        </li>
                      </ContextMenuTrigger>
                      <ContextMenuContent
                        className={MENU}
                        onCloseAutoFocus={onCloseAutoFocus}
                        aria-label={t("editor:navigator.rowActions", { page: name })}
                      >
                        <MenuItems entries={entries} kind="context" />
                      </ContextMenuContent>
                    </ContextMenu>
                  )}
                </DndRow>
              )
            })}
          </ul>
        </div>
      </DndContext>
      <p className="sr-only" role="status" aria-live="polite" data-slot="page-tree-announcement">
        {announcement}
      </p>
    </div>
  )
}
