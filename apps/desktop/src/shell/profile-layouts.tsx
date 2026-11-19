import { AlertTriangle, ArrowDown, ArrowUp, ExternalLink, Plus, Star, X } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { LayoutCompatibilitySummary, LayoutDevice } from "@/components/layout-compatibility"
import { ErrorNotice, Loading } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useAsync } from "@/hooks/use-async"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { fitsModel, hasUnknownModel, modelLabel } from "@/lib/shared-layouts"
import type { ProfileLayoutEntry, SharedLayoutSummary } from "@/lib/types"

/**
 * The shared layouts assigned to one profile, in the profile's order: the
 * default layout per device model, the order, removing (the layout itself
 * is kept) and assigning more.
 *
 * phase 2: shown in Settings › Profiles; `onOpenLayout` opens the layout
 * in the Device Editor there.
 */
export function ProfileLayouts({
  profileId,
  onOpenLayout,
}: {
  profileId: string
  onOpenLayout?: (layoutId: string) => void
}) {
  const { revision, refresh } = useAppContext()
  const key = `${profileId}:${revision}`
  const entries = useAsync(() => api.profileLayouts(profileId), `profile-layouts:${key}`)
  const all = useAsync(() => api.listSharedLayouts(), `shared-layouts:${key}`)
  const [busy, setBusy] = useState(false)

  const run = async (change: () => Promise<unknown>, done: string) => {
    setBusy(true)
    try {
      await change()
      toast.success(done)
      entries.reload()
      all.reload()
      refresh()
    } catch (e) {
      toast.error(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  if (entries.state.status === "loading") return <Loading />
  if (entries.state.status === "error") return <ErrorNotice error={entries.state.error} />
  const list = [...entries.state.data].sort((a, b) => a.position - b.position)
  const order = list.map((e) => e.layout.id)
  const assigned = new Set(order)
  const available =
    all.state.status === "ready" ? all.state.data.filter((l) => !assigned.has(l.id)) : []

  const move = (index: number, by: -1 | 1) => {
    const next = [...order]
    const target = index + by
    ;[next[index], next[target]] = [next[target] as string, next[index] as string]
    void run(() => api.reorderProfileLayouts(profileId, next), "Order saved")
  }

  return (
    <section className="grid gap-2" aria-label="Layouts of the profile">
      {list.length === 0 ? (
        <p className="text-sm text-muted-foreground">No layouts assigned to this profile.</p>
      ) : (
        <ol className="grid divide-y rounded-md border">
          {list.map((entry, index) => (
            <EntryRow
              key={entry.layout.id}
              entry={entry}
              first={index === 0}
              last={index === list.length - 1}
              busy={busy}
              onMove={(by) => {
                move(index, by)
              }}
              onDefault={() => {
                void run(
                  () => api.setDefaultLayout(profileId, entry.layout.id),
                  `Default layout for ${modelLabel(entry.layout)} changed`,
                )
              }}
              onRemove={() => {
                void run(
                  () => api.unassignLayout(profileId, entry.layout.id),
                  "Layout removed from the profile; the layout is kept",
                )
              }}
              onOpen={
                onOpenLayout
                  ? () => {
                      onOpenLayout(entry.layout.id)
                    }
                  : undefined
              }
            />
          ))}
        </ol>
      )}
      <div>
        <AssignPicker
          layouts={available}
          loading={all.state.status === "loading"}
          disabled={busy}
          onAssign={(layout) => {
            void run(() => api.assignLayout(profileId, layout.id), `${layout.name} assigned`)
          }}
        />
        {all.state.status === "error" && <ErrorNotice error={all.state.error} />}
      </div>
    </section>
  )
}

function EntryRow({
  entry,
  first,
  last,
  busy,
  onMove,
  onDefault,
  onRemove,
  onOpen,
}: {
  entry: ProfileLayoutEntry
  first: boolean
  last: boolean
  busy: boolean
  onMove: (by: -1 | 1) => void
  onDefault: () => void
  onRemove: () => void
  onOpen?: () => void
}) {
  const { layout } = entry
  const model = modelLabel(layout)
  return (
    <li
      className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-1.5 text-sm"
      aria-label={layout.name}
    >
      <span className="min-w-0 flex-1 truncate font-medium">{layout.name}</span>
      <span className="max-w-48 min-w-0 text-muted-foreground">
        <LayoutDevice layout={layout} />
      </span>
      {!hasUnknownModel(layout) && <LayoutCompatibilitySummary layout={layout} />}
      {entry.unavailable.length > 0 && <Unavailable items={entry.unavailable} />}
      {entry.isDefault ? (
        <Badge variant="secondary" aria-label={`Default for ${model}`}>
          <Star aria-hidden />
          Default for {model}
        </Badge>
      ) : (
        <Button variant="ghost" size="xs" disabled={busy} onClick={onDefault}>
          Make default for {model}
        </Button>
      )}
      <span className="flex items-center gap-0.5">
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={`Move ${layout.name} up`}
          disabled={busy || first}
          onClick={() => {
            onMove(-1)
          }}
        >
          <ArrowUp />
        </Button>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={`Move ${layout.name} down`}
          disabled={busy || last}
          onClick={() => {
            onMove(1)
          }}
        >
          <ArrowDown />
        </Button>
        {onOpen && (
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`Open ${layout.name}`}
            onClick={onOpen}
          >
            <ExternalLink />
          </Button>
        )}
        <ConfirmDialog
          trigger={
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label={`Remove ${layout.name} from the profile`}
              disabled={busy}
            >
              <X />
            </Button>
          }
          title={`Remove “${layout.name}” from this profile?`}
          description="The layout itself is kept, with its pages and keys, and stays available to other profiles and in Layouts."
          confirmLabel="Remove"
          onConfirm={onRemove}
        />
      </span>
    </li>
  )
}

/** Keys that cannot run in this profile, listed in the tooltip. */
function Unavailable({ items }: { items: string[] }) {
  const label = `${items.length} ${items.length === 1 ? "key" : "keys"} unavailable`
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          tabIndex={0}
          role="note"
          aria-label={`${label} in this profile: ${items.join("; ")}`}
          className="inline-flex items-center gap-1 rounded text-xs text-amber-700 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none dark:text-amber-400"
        >
          <AlertTriangle className="size-3.5" aria-hidden />
          {label}
        </span>
      </TooltipTrigger>
      <TooltipContent>
        <p className="mb-1 font-medium">Cannot run in this profile:</p>
        <ul className="grid gap-0.5">
          {items.map((item) => (
            <li key={item}>{item}</li>
          ))}
        </ul>
      </TooltipContent>
    </Tooltip>
  )
}

/** The layouts not assigned yet, with their device and compatibility. */
function AssignPicker({
  layouts,
  loading,
  disabled,
  onAssign,
}: {
  layouts: SharedLayoutSummary[]
  loading: boolean
  disabled: boolean
  onAssign: (layout: SharedLayoutSummary) => void
}) {
  const sorted = [...layouts].sort((a, b) => a.name.localeCompare(b.name))
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" disabled={disabled || loading}>
          <Plus />
          Assign layout
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-72">
        <DropdownMenuLabel className="text-xs text-muted-foreground">
          {sorted.length === 0 ? "Every layout is assigned" : "Layouts not in this profile"}
        </DropdownMenuLabel>
        {sorted.map((layout) => {
          const unknown = hasUnknownModel(layout)
          const others = layout.compatibility.slice(1)
          const fits = others.filter((c) => fitsModel(c.fit)).length
          return (
            <DropdownMenuItem
              key={layout.id}
              disabled={unknown}
              onSelect={() => {
                onAssign(layout)
              }}
            >
              <span className="grid min-w-0 flex-1">
                <span className="truncate">{layout.name}</span>
                <span className="truncate text-xs text-muted-foreground">
                  {unknown
                    ? `${layout.model}: unknown model, cannot be assigned`
                    : `${modelLabel(layout)}${others.length > 0 ? ` · fits ${fits} of ${others.length} other models` : ""}`}
                </span>
              </span>
            </DropdownMenuItem>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
