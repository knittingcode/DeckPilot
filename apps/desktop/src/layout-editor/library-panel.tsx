import { useDraggable } from "@dnd-kit/core"
import {
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  Folder,
  FolderPlus,
  ListOrdered,
  ListPlus,
} from "lucide-react"
import { useState, type ReactNode } from "react"

import { ActionIcon } from "@/components/action-icon"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"

import { catalogDragId, filterCatalog, type CatalogEntry, type CatalogGroup } from "./catalog"
import type { DragSource } from "./editing"
import type { TargetGroup, TargetOption } from "./targets"

function OptionIcon({ option }: { option: TargetOption }) {
  if (option.kind === "action") return <ActionIcon name={option.icon} className="size-4 shrink-0" />
  if (option.kind === "workflow") return <ListOrdered className="size-4 shrink-0" />
  if (option.target.type === "back") return <ArrowLeft className="size-4 shrink-0" />
  return <Folder className="size-4 shrink-0" />
}

const ENTRY =
  "flex min-w-0 cursor-grab items-center gap-2 rounded-md border bg-card px-2 py-1.5 text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"

function LibraryEntry({ option }: { option: TargetOption }) {
  const source: DragSource = { type: "target", target: option.target }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: `library:${option.value}`,
    data: source,
    // Plugin actions that cannot run now cannot be newly placed.
    disabled: option.unavailable !== undefined,
  })
  if (option.unavailable !== undefined) {
    return (
      <li
        className="grid min-w-0 gap-0.5 rounded-md border border-dashed px-2 py-1.5 text-sm text-muted-foreground"
        aria-label={`${option.label}: unavailable, ${option.unavailable}`}
        aria-disabled
      >
        <span className="flex min-w-0 items-center gap-2">
          <OptionIcon option={option} />
          <span className="min-w-0 flex-1 truncate" title={option.label}>
            {option.label}
          </span>
        </span>
        <span className="text-xs">{option.unavailable}</span>
      </li>
    )
  }
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={`Drag ${option.label} onto a control`}
    >
      <OptionIcon option={option} />
      <span className="min-w-0 flex-1 truncate" title={option.label}>
        {option.label}
      </span>
    </li>
  )
}

/** The id of the "New folder" entry among the editor's draggables. */
export const NEW_FOLDER_DRAG_ID = "library:new-folder"

/**
 * "New folder": dropped on an empty control, it creates a folder in the
 * open page and makes the control open it.
 */
function NewFolderEntry() {
  const source: DragSource = { type: "newFolder" }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: NEW_FOLDER_DRAG_ID,
    data: source,
  })
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, "border-dashed", isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label="Drag New folder onto an empty control"
      title="Drop on an empty key to create a folder there. Double-click the key to enter it."
    >
      <FolderPlus className="size-4 shrink-0" />
      <span className="min-w-0 flex-1 truncate">New folder</span>
    </li>
  )
}

/** The id of the "Multi Action" entry among the editor's draggables. */
export const NEW_MULTI_ACTION_DRAG_ID = "library:new-multi-action"

/**
 * "Multi Action": dropped on a control or a gesture, it creates an empty
 * step workflow, binds it and opens its steps.
 */
function NewMultiActionEntry() {
  const source: DragSource = { type: "newMultiAction" }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: NEW_MULTI_ACTION_DRAG_ID,
    data: source,
  })
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, "border-dashed", isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label="Drag Multi Action onto a control"
      title="Runs several actions in order, with delays between them. Drop it on a key, then add its steps."
    >
      <ListPlus className="size-4 shrink-0" />
      <span className="min-w-0 flex-1 truncate">Multi Action</span>
    </li>
  )
}

/**
 * An action type: dropped on a control, an action of it with its default
 * settings is created (or the one that exists is reused) and bound.
 */
function CatalogItem({ entry }: { entry: CatalogEntry }) {
  const source: DragSource = { type: "actionType", actionType: entry.type.id }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: catalogDragId(entry.type.id),
    data: source,
  })
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, "border-dashed", isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={`Drag ${entry.type.name} onto a control`}
      title={`${entry.type.description} (${entry.module})`}
    >
      <ActionIcon name={entry.type.node?.icon} className="size-4 shrink-0" />
      <span className="min-w-0 flex-1 truncate">{entry.type.name}</span>
    </li>
  )
}

/** A group of the library that can be folded away. */
function Section({
  label,
  count,
  open,
  onToggle,
  children,
}: {
  label: string
  count: number
  open: boolean
  onToggle: () => void
  children: ReactNode
}) {
  return (
    <section className="grid gap-1" aria-label={label}>
      <h3>
        <button
          type="button"
          className="flex w-full items-center gap-1 rounded-sm text-left text-xs font-medium text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-expanded={open}
          onClick={onToggle}
        >
          {open ? (
            <ChevronDown className="size-3" aria-hidden />
          ) : (
            <ChevronRight className="size-3" aria-hidden />
          )}
          <span className="flex-1">{label}</span>
          <span aria-hidden>{count}</span>
        </button>
      </h3>
      {open && <ul className="grid gap-1">{children}</ul>}
    </section>
  )
}

const matches = (text: string, needle: string) => text.toLowerCase().includes(needle)

/**
 * What can be dragged onto controls, in one searchable list: the
 * profile's actions and workflows, navigation (pages, Back, a new folder)
 * and, under "New: …", the action types that need no settings.
 */
export function LibraryPanel({
  groups,
  catalog = [],
  multiAction = false,
  variant = "card",
}: {
  groups: TargetGroup[]
  /** Action types that can be placed directly; see `catalogGroups`. */
  catalog?: CatalogGroup[]
  /** Offers "Multi Action", which puts a new step workflow on a key. */
  multiAction?: boolean
  /**
   * `card`: a card of its own. `plain`: the content only, for a container
   * that has the title and scrolls (the editor's library sidebar).
   */
  variant?: "card" | "plain"
}) {
  const [query, setQuery] = useState("")
  const [closed, setClosed] = useState<Set<string>>(new Set())
  const needle = query.trim().toLowerCase()
  const searching = needle !== ""
  const shown = groups
    .map((group) => ({
      ...group,
      options: group.options.filter((o) => matches(`${o.label} ${group.label}`, needle)),
    }))
    .filter((group) => group.options.length > 0 || (group.label === "Navigation" && !searching))
  const types = filterCatalog(catalog, query)
  const folder = !searching || matches("new folder folders navigation", needle)
  const multi =
    multiAction && (!searching || matches("new multi action sequence steps workflow", needle))
  const hasItems = groups.some((g) => g.label !== "Navigation")
  // A search shows what it found, whatever is folded.
  const isOpen = (id: string) => searching || !closed.has(id)
  const toggle = (id: string) => {
    setClosed((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }
  const nothing = searching && shown.length === 0 && types.length === 0 && !folder && !multi

  const hint = "Drag onto a control, or select a control and choose in the inspector."
  const content = (
    <>
      <Input
        type="search"
        className="h-8"
        placeholder="Search the library"
        aria-label="Search the library"
        value={query}
        onChange={(event) => {
          setQuery(event.target.value)
        }}
      />
      <div className={cn("grid gap-3", variant === "card" && "max-h-[28rem] overflow-y-auto")}>
        {!hasItems && !searching && catalog.length === 0 && (
          <p className="text-sm text-muted-foreground">
            No actions or workflows yet. Create them in the Action Library.
          </p>
        )}
        {nothing && <p className="text-sm text-muted-foreground">Nothing matches.</p>}
        {shown
          .filter((group) => group.label !== "Navigation")
          .map((group) => (
            <Section
              key={group.label}
              label={group.label}
              count={group.options.length}
              open={isOpen(group.label)}
              onToggle={() => {
                toggle(group.label)
              }}
            >
              {group.options.map((option) => (
                <LibraryEntry key={option.value} option={option} />
              ))}
            </Section>
          ))}
        {(folder || shown.some((group) => group.label === "Navigation")) && (
          <Section
            label="Navigation"
            count={
              (shown.find((group) => group.label === "Navigation")?.options.length ?? 0) +
              (folder ? 1 : 0)
            }
            open={isOpen("Navigation")}
            onToggle={() => {
              toggle("Navigation")
            }}
          >
            {folder && <NewFolderEntry />}
            {shown
              .find((group) => group.label === "Navigation")
              ?.options.map((option) => (
                <LibraryEntry key={option.value} option={option} />
              ))}
          </Section>
        )}
        {multi && (
          <Section
            label="New: Sequence"
            count={1}
            open={isOpen("New: Sequence")}
            onToggle={() => {
              toggle("New: Sequence")
            }}
          >
            <NewMultiActionEntry />
          </Section>
        )}
        {types.map((group) => {
          const id = `New: ${group.label}`
          return (
            <Section
              key={id}
              label={id}
              count={group.entries.length}
              open={isOpen(id)}
              onToggle={() => {
                toggle(id)
              }}
            >
              {group.entries.map((entry) => (
                <CatalogItem key={entry.type.id} entry={entry} />
              ))}
            </Section>
          )
        })}
      </div>
    </>
  )
  if (variant === "plain") {
    return (
      <div className="grid gap-3">
        <p className="text-xs text-muted-foreground">{hint}</p>
        {content}
      </div>
    )
  }
  return (
    <Card className="h-fit">
      <CardHeader>
        <CardTitle className="text-sm">Library</CardTitle>
        <CardDescription>{hint}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3">{content}</CardContent>
    </Card>
  )
}
