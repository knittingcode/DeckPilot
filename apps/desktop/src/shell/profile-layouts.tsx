import { ArrowDown, ArrowUp, Plus, SquarePen, X } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { LayoutDevice } from "@/components/layout-compatibility"
import { ErrorNotice, Loading } from "@/components/page"
import {
  InlineStatus,
  SettingsGroup,
  SettingsRow,
  SettingsSection,
} from "@/components/settings-layout"
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
 * Settings › Profiles, "Layouts": the shared layouts assigned to one
 * profile, in the profile's order, with the default layout per device
 * model, the order, removing (the layout itself is kept), assigning more
 * and opening one in the Device Editor.
 */
export function ProfileLayouts({
  profileId,
  onOpenLayout,
}: {
  profileId: string
  onOpenLayout?: (layout: SharedLayoutSummary) => void
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

  const list =
    entries.state.status === "ready"
      ? [...entries.state.data].sort((a, b) => a.position - b.position)
      : []
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
    <SettingsSection
      title="Layouts"
      description="The layouts this profile uses, in its order. Each device shows the default layout of its model; Switch layout keys show the others. Other profiles may use the same layouts."
      actions={
        <AssignPicker
          layouts={available}
          loading={all.state.status === "loading"}
          disabled={busy}
          onAssign={(layout) => {
            void run(() => api.assignLayout(profileId, layout.id), `${layout.name} assigned`)
          }}
        />
      }
    >
      {entries.state.status === "loading" && <Loading />}
      {entries.state.status === "error" && <ErrorNotice error={entries.state.error} />}
      {entries.state.status === "ready" &&
        (list.length === 0 ? (
          <p className="text-sm text-muted-foreground">No layouts assigned to this profile.</p>
        ) : (
          <SettingsGroup role="list" aria-label="Layouts of the profile">
            {list.map((entry, index) => (
              <EntryRow
                key={entry.layout.id}
                profileId={profileId}
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
                        onOpenLayout(entry.layout)
                      }
                    : undefined
                }
              />
            ))}
          </SettingsGroup>
        ))}
      {all.state.status === "error" && <ErrorNotice error={all.state.error} />}
    </SettingsSection>
  )
}

function EntryRow({
  profileId,
  entry,
  first,
  last,
  busy,
  onMove,
  onDefault,
  onRemove,
  onOpen,
}: {
  profileId: string
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
  const others = layout.profiles.filter((p) => p.profileId !== profileId)
  return (
    <div role="listitem" aria-label={layout.name}>
      <SettingsRow
        className="py-1.5"
        label={<span className="block truncate">{layout.name}</span>}
        description={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span className="max-w-48 min-w-0">
              <LayoutDevice layout={layout} />
            </span>
            {entry.isDefault && (
              <InlineStatus tone="positive" aria-label={`Default for ${model}`}>
                Default for {model}
              </InlineStatus>
            )}
            {others.length > 0 && (
              <InlineStatus
                title={`Edits apply to ${others.map((p) => p.profileName).join(", ")} too`}
              >
                Also in {others.map((p) => p.profileName).join(", ")}
              </InlineStatus>
            )}
            {!hasUnknownModel(layout) && <Compatibility layout={layout} />}
            {entry.unavailable.length > 0 && <Unavailable items={entry.unavailable} />}
          </span>
        }
      >
        {!entry.isDefault && (
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
              title="Open in the Device Editor"
              disabled={hasUnknownModel(layout)}
              onClick={onOpen}
            >
              <SquarePen />
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
      </SettingsRow>
    </div>
  )
}

/** How many other models the layout fits; the tooltip explains each model. */
function Compatibility({ layout }: { layout: SharedLayoutSummary }) {
  const others = layout.compatibility.slice(1)
  const fits = others.filter((c) => fitsModel(c.fit)).length
  const misses = others.filter((c) => c.fit.type === "missing").length
  const lines = layout.compatibility.map((c) => `${c.modelName}: ${c.explanation}`)
  const label =
    others.length === 0
      ? "Own model only"
      : [fits > 0 && `fits ${fits}`, misses > 0 && `misses ${misses}`].filter(Boolean).join(", ") ||
        "Other models unknown"
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <InlineStatus
          tabIndex={0}
          role="note"
          tone={misses > 0 ? "neutral" : "positive"}
          aria-label={`Compatibility: ${lines.join("; ")}`}
          className="rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          {label}
        </InlineStatus>
      </TooltipTrigger>
      <TooltipContent>
        <ul className="grid gap-0.5">
          {lines.map((text) => (
            <li key={text}>{text}</li>
          ))}
        </ul>
      </TooltipContent>
    </Tooltip>
  )
}

/** Keys that cannot run in this profile, listed in the tooltip. */
function Unavailable({ items }: { items: string[] }) {
  const label = `${items.length} ${items.length === 1 ? "key" : "keys"} unavailable`
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <InlineStatus
          tabIndex={0}
          role="note"
          tone="warning"
          aria-label={`${label} in this profile: ${items.join("; ")}`}
          className="rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          {label}
        </InlineStatus>
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
      <DropdownMenuContent align="end" className="w-72">
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
