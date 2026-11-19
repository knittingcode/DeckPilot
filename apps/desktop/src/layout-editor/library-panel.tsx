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
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import type { ActionType } from "@/lib/types"
import { cn } from "@/lib/utils"

import {
  catalogDragId,
  filterCatalog,
  presetDragId,
  type CatalogEntry,
  type CatalogGroup,
} from "./catalog"
import type { ActionPreset, DragSource } from "./editing"
import { loadCompatibleOnly, saveCompatibleOnly } from "./library-sidebar"
import type { TargetGroup, TargetOption } from "./targets"

/** A 16px icon in a small neutral square, the same for every row. */
function EntryIcon({ children }: { children: ReactNode }) {
  return (
    <span
      data-slot="library-icon"
      className="flex size-6 shrink-0 items-center justify-center rounded-md border bg-muted/60 text-muted-foreground [&_svg]:size-4 [&_svg]:shrink-0"
    >
      {children}
    </span>
  )
}

function OptionIcon({ option }: { option: TargetOption }) {
  if (option.kind === "action") return <ActionIcon name={option.icon} />
  if (option.kind === "workflow") return <ListOrdered aria-hidden />
  if (option.target.type === "back") return <ArrowLeft aria-hidden />
  return <Folder aria-hidden />
}

/** A compact row of the library, about 32px high: icon, then name. */
const ENTRY =
  "flex h-8 min-w-0 cursor-grab items-center gap-2 rounded-md px-1 text-sm transition-colors select-none hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none active:cursor-grabbing"

/** A row that cannot be dragged: muted, with why under the name. */
const DISABLED_ENTRY = "grid min-w-0 gap-0.5 rounded-md px-1 py-1 text-sm text-muted-foreground"

function LibraryEntry({ option }: { option: TargetOption }) {
  const source: DragSource = { type: "target", target: option.target }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: `library:${option.value}`,
    data: source,
    // Plugin actions that cannot run now cannot be newly placed, nor
    // actions that cannot run from the selected control.
    disabled: option.unavailable !== undefined || option.incompatible !== undefined,
  })
  if (option.incompatible !== undefined && option.unavailable === undefined) {
    return (
      <IncompatibleEntry
        label={option.label}
        reason={option.incompatible}
        icon={<OptionIcon option={option} />}
      />
    )
  }
  if (option.unavailable !== undefined) {
    return (
      <li
        className={DISABLED_ENTRY}
        aria-label={`${option.label}: unavailable, ${option.unavailable}`}
        aria-disabled
        data-compatibility="unavailable"
      >
        <span className="flex h-6 min-w-0 items-center gap-2">
          <EntryIcon>
            <OptionIcon option={option} />
          </EntryIcon>
          <span className="min-w-0 flex-1 truncate" title={option.label}>
            {option.label}
          </span>
        </span>
        <span className="pl-8 text-xs">{option.unavailable}</span>
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
      <EntryIcon>
        <OptionIcon option={option} />
      </EntryIcon>
      <span className="min-w-0 flex-1 truncate" title={option.label}>
        {option.label}
      </span>
    </li>
  )
}

/**
 * An entry that cannot go on the selected control: shown, not draggable,
 * with the reason (docs/device-controls.md).
 */
function IncompatibleEntry({
  label,
  reason,
  icon,
}: {
  label: string
  reason: string
  icon: ReactNode
}) {
  return (
    <li
      className={DISABLED_ENTRY}
      aria-label={`${label}: not for the selected control. ${reason}`}
      aria-disabled
      data-compatibility="incompatible"
    >
      <span className="flex h-6 min-w-0 items-center gap-2 opacity-70">
        <EntryIcon>{icon}</EntryIcon>
        <span className="min-w-0 flex-1 truncate" title={label}>
          {label}
        </span>
      </span>
      <span className="pl-8 text-xs">{reason}</span>
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
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label="Drag New folder onto an empty control"
      title="Drop on an empty key to create a folder there. Double-click the key to enter it."
    >
      <EntryIcon>
        <FolderPlus aria-hidden />
      </EntryIcon>
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
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label="Drag Multi Action onto a control"
      title="Runs several actions in order, with delays between them. Drop it on a key, then add its steps."
    >
      <EntryIcon>
        <ListPlus aria-hidden />
      </EntryIcon>
      <span className="min-w-0 flex-1 truncate">Multi Action</span>
    </li>
  )
}

/**
 * An action type: dropped on a control, an action of it with its default
 * settings is created (or the one that exists is reused) and bound.
 */
function CatalogItem({ entry, incompatible }: { entry: CatalogEntry; incompatible?: string }) {
  const source: DragSource = { type: "actionType", actionType: entry.type.id }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: catalogDragId(entry.type.id),
    data: source,
    disabled: incompatible !== undefined,
  })
  if (incompatible !== undefined) {
    return (
      <IncompatibleEntry
        label={entry.type.name}
        reason={incompatible}
        icon={<ActionIcon name={entry.type.node?.icon} />}
      />
    )
  }
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={`Drag ${entry.type.name} onto a control`}
      title={`${entry.type.description} (${entry.module})`}
    >
      <EntryIcon>
        <ActionIcon name={entry.type.node?.icon} />
      </EntryIcon>
      <span className="min-w-0 flex-1 truncate">{entry.type.name}</span>
    </li>
  )
}

/**
 * A ready-made action of a type in the library, e.g. "Switch to Teams": the
 * profile's action that is exactly it, else a new one, is put on the control.
 */
export interface PresetEntry {
  /** Unique among the presets. */
  key: string
  label: string
  description: string
  /** The catalog group it is listed in, e.g. "Navigation". */
  category: string
  type: ActionType
  preset: ActionPreset
}

function PresetItem({ entry, incompatible }: { entry: PresetEntry; incompatible?: string }) {
  const source: DragSource = { type: "actionType", actionType: entry.type.id, preset: entry.preset }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: presetDragId(entry.key),
    data: source,
    disabled: incompatible !== undefined,
  })
  const icon = <ActionIcon name={entry.preset.icon ?? entry.type.node?.icon} />
  if (incompatible !== undefined) {
    return <IncompatibleEntry label={entry.label} reason={incompatible} icon={icon} />
  }
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={`Drag ${entry.label} onto a control`}
      title={entry.description}
    >
      <EntryIcon>{icon}</EntryIcon>
      <span className="min-w-0 flex-1 truncate">{entry.label}</span>
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
    <section className="grid gap-0.5 border-t pt-2 first:border-t-0 first:pt-0" aria-label={label}>
      <h3>
        <button
          type="button"
          className="flex h-6 w-full items-center gap-1 rounded-sm px-1 text-left text-[11px] font-medium tracking-wide text-muted-foreground uppercase hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-expanded={open}
          onClick={onToggle}
        >
          {open ? (
            <ChevronDown className="size-3" aria-hidden />
          ) : (
            <ChevronRight className="size-3" aria-hidden />
          )}
          <span className="flex-1 truncate">{label}</span>
          <span className="tabular-nums" aria-hidden>
            {count}
          </span>
        </button>
      </h3>
      {open && <ul className="grid gap-px">{children}</ul>}
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
  presets = [],
  multiAction = false,
  variant = "card",
  compatibility,
}: {
  groups: TargetGroup[]
  /** Action types that can be placed directly; see `catalogGroups`. */
  catalog?: CatalogGroup[]
  /** Ready-made actions listed with the catalog's group of their category. */
  presets?: PresetEntry[]
  /** Offers "Multi Action", which puts a new step workflow on a key. */
  multiAction?: boolean
  /**
   * `card`: a card of its own. `plain`: the content only, for a container
   * that has the title and scrolls (the editor's library sidebar).
   */
  variant?: "card" | "plain"
  /**
   * The control the library is for (docs/device-controls.md): entries that
   * cannot go on it are disabled with the reason (`incompatible` of the
   * groups' options, `catalog` for action types), and "Compatible only"
   * hides them.
   */
  compatibility?: {
    /** The selected control, e.g. "Key 3"; `null` when none is. */
    control: string | null
    catalog: (type: ActionType) => string | null
  }
}) {
  const [query, setQuery] = useState("")
  const [compatibleOnly, setCompatibleOnlyState] = useState(loadCompatibleOnly)
  const setCompatibleOnly = (on: boolean) => {
    setCompatibleOnlyState(on)
    saveCompatibleOnly(on)
  }
  const hiding = Boolean(compatibility?.control) && compatibleOnly
  const whyNot = (type: ActionType) =>
    compatibility?.control ? (compatibility.catalog(type) ?? undefined) : undefined
  const [closed, setClosed] = useState<Set<string>>(new Set())
  const needle = query.trim().toLowerCase()
  const searching = needle !== ""
  const shown = groups
    .map((group) => ({
      ...group,
      options: group.options.filter(
        (o) =>
          matches(`${o.label} ${group.label}`, needle) && !(hiding && o.incompatible !== undefined),
      ),
    }))
    .filter((group) => group.options.length > 0 || (group.label === "Navigation" && !searching))
  const types = filterCatalog(catalog, query)
    .map((group) => ({
      ...group,
      entries: hiding ? group.entries.filter((e) => whyNot(e.type) === undefined) : group.entries,
    }))
    .filter((group) => group.entries.length > 0)
  const shownPresets = presets.filter(
    (p) =>
      matches(`${p.label} ${p.description} ${p.category}`, needle) &&
      !(hiding && whyNot(p.type) !== undefined),
  )
  const presetsOf = (category: string) => shownPresets.filter((p) => p.category === category)
  for (const category of new Set(shownPresets.map((p) => p.category))) {
    if (!types.some((group) => group.label === category))
      types.push({ label: category, entries: [] })
  }
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
      {compatibility && (
        <div className="grid gap-1">
          <div className="flex items-center gap-2">
            <Switch
              id="library-compatible-only"
              size="sm"
              checked={compatibleOnly}
              onCheckedChange={setCompatibleOnly}
            />
            <Label htmlFor="library-compatible-only" className="text-xs font-normal">
              Compatible only
            </Label>
          </div>
          <p className="text-xs text-muted-foreground">
            {compatibility.control
              ? `Showing what fits ${compatibility.control}.`
              : "Select a control to see what fits it."}
          </p>
        </div>
      )}
      <div className={cn("grid gap-2", variant === "card" && "max-h-[28rem] overflow-y-auto")}>
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
              count={group.entries.length + presetsOf(group.label).length}
              open={isOpen(id)}
              onToggle={() => {
                toggle(id)
              }}
            >
              {group.entries.map((entry) => (
                <CatalogItem key={entry.type.id} entry={entry} incompatible={whyNot(entry.type)} />
              ))}
              {presetsOf(group.label).map((entry) => (
                <PresetItem key={entry.key} entry={entry} incompatible={whyNot(entry.type)} />
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
