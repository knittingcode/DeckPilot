import { GitBranch, Merge, OctagonX, Plus, Repeat, Search, Split, Timer } from "lucide-react"
import { useState } from "react"

import { ActionIcon } from "@/components/action-icon"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import type { ActionType } from "@/lib/types"

import { type CatalogEntry, type CatalogGroup, DRAG_TYPE, searchCatalog } from "./catalog"

const FLOW_ICONS: Record<string, typeof GitBranch> = {
  condition: GitBranch,
  parallel: Split,
  join: Merge,
  loop: Repeat,
  delay: Timer,
  stop: OctagonX,
}

function EntryIcon({ entry }: { entry: CatalogEntry }) {
  if (entry.kind.type !== "action") {
    const Icon = FLOW_ICONS[entry.kind.type] ?? GitBranch
    return <Icon className="size-4" aria-hidden />
  }
  return <ActionIcon name={entry.icon} className="size-4" />
}

/**
 * Searchable list of nodes. Entries are dragged onto the canvas or added
 * with a click (after the selected node).
 */
export function CatalogPanel({
  groups,
  onAdd,
  onCreateAction,
}: {
  groups: CatalogGroup[]
  onAdd: (entry: CatalogEntry) => void
  onCreateAction: (type: ActionType) => void
}) {
  const [query, setQuery] = useState("")
  const visible = searchCatalog(groups, query)
  return (
    <div className="flex min-h-0 flex-col">
      <div className="relative border-b p-2">
        <Search className="pointer-events-none absolute top-1/2 left-4 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
          placeholder="Search nodes"
          aria-label="Search nodes"
          className="h-8 pl-7 text-sm"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2" role="list" aria-label="Nodes">
        {visible.length === 0 && (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">No matching nodes</p>
        )}
        {visible.map((group) => (
          <section key={group.id} className="mb-3 grid gap-0.5">
            <h3 className="px-2 pb-1 text-[0.6875rem] font-medium tracking-wide text-muted-foreground uppercase">
              {group.title}
            </h3>
            {group.entries.map((entry) => (
              <button
                key={entry.key}
                type="button"
                role="listitem"
                draggable
                onDragStart={(event) => {
                  event.dataTransfer.setData(DRAG_TYPE, entry.key)
                  event.dataTransfer.effectAllowed = "copy"
                }}
                onClick={() => {
                  onAdd(entry)
                }}
                className="flex cursor-grab items-center gap-2 rounded-md px-2 py-1.5 text-left hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none active:cursor-grabbing"
                title={`${entry.title} — drag onto the canvas or click to add`}
              >
                <span className="flex size-6 shrink-0 items-center justify-center rounded bg-muted text-muted-foreground">
                  <EntryIcon entry={entry} />
                </span>
                <span className="grid min-w-0">
                  <span className="truncate text-sm">{entry.title}</span>
                  <span className="truncate text-xs text-muted-foreground">
                    {entry.description}
                  </span>
                </span>
              </button>
            ))}
            {group.types.length > 0 && (
              <div className="mt-0.5 flex flex-wrap gap-1 px-2">
                {group.types.map((type) => (
                  <Button
                    key={type.id}
                    variant="ghost"
                    size="sm"
                    className="h-6 px-1.5 text-xs text-muted-foreground"
                    title={type.description}
                    onClick={() => {
                      onCreateAction(type)
                    }}
                  >
                    <Plus className="size-3" />
                    {type.name}
                  </Button>
                ))}
              </div>
            )}
          </section>
        ))}
      </div>
    </div>
  )
}
