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
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
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
  const { t } = useTranslation()
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
    void run(
      () => api.reorderProfileLayouts(profileId, next),
      t("layouts:profileLayouts.toast.orderSaved"),
    )
  }

  return (
    <SettingsSection
      title={t("layouts:profileLayouts.title")}
      description={t("layouts:profileLayouts.description")}
      actions={
        <AssignPicker
          layouts={available}
          loading={all.state.status === "loading"}
          disabled={busy}
          onAssign={(layout) => {
            void run(
              () => api.assignLayout(profileId, layout.id),
              t("layouts:profileLayouts.toast.assigned", { layout: layout.name }),
            )
          }}
        />
      }
    >
      {entries.state.status === "loading" && <Loading />}
      {entries.state.status === "error" && <ErrorNotice error={entries.state.error} />}
      {entries.state.status === "ready" &&
        (list.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("layouts:profileLayouts.empty")}</p>
        ) : (
          <SettingsGroup role="list" aria-label={t("layouts:profileLayouts.listLabel")}>
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
                    t("layouts:profileLayouts.toast.defaultChanged", {
                      model: modelLabel(entry.layout),
                    }),
                  )
                }}
                onRemove={() => {
                  void run(
                    () => api.unassignLayout(profileId, entry.layout.id),
                    t("layouts:profileLayouts.toast.removed"),
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
  const { t } = useTranslation()
  const format = useFormat()
  const { layout } = entry
  const model = modelLabel(layout)
  const others = format.list(
    layout.profiles.filter((p) => p.profileId !== profileId).map((p) => p.profileName),
  )
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
              <InlineStatus
                tone="positive"
                aria-label={t("layouts:profileLayouts.defaultFor", { model })}
              >
                {t("layouts:profileLayouts.defaultFor", { model })}
              </InlineStatus>
            )}
            {others.length > 0 && (
              <InlineStatus title={t("layouts:profileLayouts.editsApply", { profiles: others })}>
                {t("layouts:profileLayouts.alsoIn", { profiles: others })}
              </InlineStatus>
            )}
            {!hasUnknownModel(layout) && <Compatibility layout={layout} />}
            {entry.unavailable.length > 0 && <Unavailable items={entry.unavailable} />}
          </span>
        }
      >
        {!entry.isDefault && (
          <Button variant="ghost" size="xs" disabled={busy} onClick={onDefault}>
            {t("layouts:profileLayouts.makeDefault", { model })}
          </Button>
        )}
        <span className="flex items-center gap-0.5">
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={t("layouts:profileLayouts.moveUp", { layout: layout.name })}
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
            aria-label={t("layouts:profileLayouts.moveDown", { layout: layout.name })}
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
              aria-label={t("layouts:profileLayouts.open", { layout: layout.name })}
              title={t("layouts:profileLayouts.openTitle")}
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
                aria-label={t("layouts:profileLayouts.removeLabel", { layout: layout.name })}
                disabled={busy}
              >
                <X />
              </Button>
            }
            title={t("layouts:profileLayouts.remove.title", { layout: layout.name })}
            description={t("layouts:profileLayouts.remove.description")}
            confirmLabel={t("layouts:profileLayouts.remove.confirm")}
            onConfirm={onRemove}
          />
        </span>
      </SettingsRow>
    </div>
  )
}

/** How many other models the layout fits; the tooltip explains each model. */
function Compatibility({ layout }: { layout: SharedLayoutSummary }) {
  const { t } = useTranslation()
  const others = layout.compatibility.slice(1)
  const fits = others.filter((c) => fitsModel(c.fit)).length
  const misses = others.filter((c) => c.fit.type === "missing").length
  const lines = layout.compatibility.map((c) => `${c.modelName}: ${c.explanation}`)
  const label =
    others.length === 0
      ? t("layouts:profileLayouts.compatibility.ownOnly")
      : fits > 0 && misses > 0
        ? t("layouts:profileLayouts.compatibility.fitsMisses", { fits, misses })
        : fits > 0
          ? t("layouts:profileLayouts.compatibility.fits", { fits })
          : misses > 0
            ? t("layouts:profileLayouts.compatibility.misses", { misses })
            : t("layouts:profileLayouts.compatibility.unknown")
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <InlineStatus
          tabIndex={0}
          role="note"
          tone={misses > 0 ? "neutral" : "positive"}
          aria-label={t("layouts:profileLayouts.compatibility.label", { lines: lines.join("; ") })}
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
  const { t } = useTranslation()
  const label = t("layouts:profileLayouts.unavailable.label", { count: items.length })
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <InlineStatus
          tabIndex={0}
          role="note"
          tone="warning"
          aria-label={t("layouts:profileLayouts.unavailable.detail", {
            count: items.length,
            items: items.join("; "),
          })}
          className="rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          {label}
        </InlineStatus>
      </TooltipTrigger>
      <TooltipContent>
        <p className="mb-1 font-medium">{t("layouts:profileLayouts.unavailable.heading")}</p>
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
  const { t } = useTranslation()
  const format = useFormat()
  const sorted = [...layouts].sort((a, b) => format.compare(a.name, b.name))
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" disabled={disabled || loading}>
          <Plus />
          {t("layouts:profileLayouts.assign.button")}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-72">
        <DropdownMenuLabel className="text-xs text-muted-foreground">
          {sorted.length === 0
            ? t("layouts:profileLayouts.assign.allAssigned")
            : t("layouts:profileLayouts.assign.notInProfile")}
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
                    ? t("layouts:profileLayouts.assign.unknownModel", { model: layout.model })
                    : others.length > 0
                      ? t("layouts:profileLayouts.assign.fitsOf", {
                          model: modelLabel(layout),
                          fits,
                          count: others.length,
                        })
                      : modelLabel(layout)}
                </span>
              </span>
            </DropdownMenuItem>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
