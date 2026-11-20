import { Copy, MoreHorizontal, Pencil, Plus, Search, SquarePen, Trash2, Users } from "lucide-react"
import { useContext, useMemo, useRef, useState } from "react"
import { toast } from "sonner"

import { DestructiveDialog, FieldError, FormDialog } from "@/components/dialogs"
import { LayoutCompatibilitySummary, LayoutDevice } from "@/components/layout-compatibility"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useAsync } from "@/hooks/use-async"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { hasUnknownModel, modelLabel } from "@/lib/shared-layouts"
import type { ProfileSummary, SharedLayoutSummary } from "@/lib/types"
import { isSupported } from "@/lib/verification"
import { EditorSelectionContext } from "@/shell/editor-selection-context"
import { useOpenLayout } from "@/shell/open-layout"

/** "No profile" in profile choices (Radix selects have no empty value). */
const NO_PROFILE = "none"

/**
 * The shared layouts: stored once and assigned to any number of profiles,
 * each of which keeps its own order and default layout per device model.
 */
export function LayoutsPage() {
  const { t } = useTranslation()
  const { revision, refresh } = useAppContext()
  const { state, reload } = useAsync(() => api.listSharedLayouts(), `layouts:${revision}`)
  const [creating, setCreating] = useState(false)
  const searchRef = useRef<HTMLInputElement>(null)
  const changed = () => {
    reload()
    refresh()
  }
  return (
    <Page
      title={t("navigation:pages.layouts")}
      description={t("layouts:page.description")}
      actions={[
        {
          id: "layouts.new",
          label: t("layouts:page.new"),
          icon: Plus,
          placement: "primary",
          onSelect: () => {
            setCreating(true)
          },
        },
        {
          id: "layouts.search",
          label: t("layouts:page.search"),
          icon: Search,
          onSelect: () => {
            searchRef.current?.focus()
          },
        },
      ]}
    >
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <LayoutTable
          layouts={state.data}
          searchRef={searchRef}
          onChanged={changed}
          onNew={() => {
            setCreating(true)
          }}
        />
      )}
      <NewSharedLayoutDialog
        open={creating}
        onOpenChange={setCreating}
        onCreated={() => {
          setCreating(false)
          changed()
        }}
      />
    </Page>
  )
}

type RowDialog =
  | { kind: "rename"; layout: SharedLayoutSummary }
  | { kind: "duplicate"; layout: SharedLayoutSummary }
  | { kind: "delete"; layout: SharedLayoutSummary }

function LayoutTable({
  layouts,
  searchRef,
  onChanged,
  onNew,
}: {
  layouts: SharedLayoutSummary[]
  searchRef: React.RefObject<HTMLInputElement | null>
  onChanged: () => void
  onNew: () => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const [query, setQuery] = useState("")
  const [dialog, setDialog] = useState<RowDialog | null>(null)
  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return layouts
      .filter((l) => !needle || l.name.toLowerCase().includes(needle))
      .sort((a, b) => format.compare(a.name, b.name))
  }, [layouts, query, format])

  if (layouts.length === 0) {
    return (
      <EmptyState title={t("layouts:page.empty.title")}>
        <p>{t("layouts:page.empty.body")}</p>
        <Button size="sm" className="mt-3" onClick={onNew}>
          <Plus />
          {t("layouts:page.new")}
        </Button>
      </EmptyState>
    )
  }
  const close = () => {
    setDialog(null)
  }
  return (
    <div className="grid gap-3">
      <div className="relative max-w-xs">
        <Search className="absolute top-2 left-2 size-4 text-muted-foreground" aria-hidden />
        <Input
          ref={searchRef}
          className="h-8 pl-8"
          placeholder={t("layouts:page.searchPlaceholder")}
          aria-label={t("layouts:page.searchPlaceholder")}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
        />
      </div>
      {shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("layouts:page.noMatches")}</p>
      ) : (
        <Table className="text-sm">
          <TableHeader>
            <TableRow>
              <TableHead>{t("layouts:page.table.name")}</TableHead>
              <TableHead>{t("layouts:page.table.device")}</TableHead>
              <TableHead>{t("layouts:page.table.contents")}</TableHead>
              <TableHead>{t("layouts:page.table.usedBy")}</TableHead>
              <TableHead>{t("layouts:page.table.compatibility")}</TableHead>
              <TableHead className="w-0 text-right">
                <span className="sr-only">{t("layouts:page.table.actions")}</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {shown.map((layout) => (
              <LayoutRow
                key={layout.id}
                layout={layout}
                onChanged={onChanged}
                onDialog={setDialog}
              />
            ))}
          </TableBody>
        </Table>
      )}
      {dialog?.kind === "rename" && (
        <NameDialog
          title={t("layouts:page.rename.title")}
          label={t("layouts:page.rename.label")}
          confirmLabel={t("layouts:page.rename.confirm")}
          initial={dialog.layout.name}
          onClose={close}
          onSubmit={async (name) => {
            await api.renameSharedLayout(dialog.layout.id, name)
            toast.success(t("layouts:page.toast.renamed"))
            close()
            onChanged()
          }}
        />
      )}
      {dialog?.kind === "duplicate" && (
        <NameDialog
          title={t("layouts:page.duplicate.title")}
          description={t("layouts:page.duplicate.description")}
          label={t("layouts:page.duplicate.label")}
          confirmLabel={t("layouts:page.duplicate.confirm")}
          initial={t("layouts:page.duplicate.copyName", { layout: dialog.layout.name })}
          onClose={close}
          onSubmit={async (name) => {
            await api.duplicateSharedLayout(dialog.layout.id, name, null)
            toast.success(t("layouts:page.toast.duplicated"))
            close()
            onChanged()
          }}
        />
      )}
      {dialog?.kind === "delete" && (
        <DeleteDialog
          layout={dialog.layout}
          onClose={close}
          onDeleted={() => {
            close()
            onChanged()
          }}
        />
      )}
    </div>
  )
}

function LayoutRow({
  layout,
  onChanged,
  onDialog,
}: {
  layout: SharedLayoutSummary
  onChanged: () => void
  onDialog: (dialog: RowDialog) => void
}) {
  const { t } = useTranslation()
  const { open, dialog } = useOpenLayout()
  // Unused: opening offers to assign it to the profile being edited, which
  // a layout of an unknown model cannot be.
  const closed = layout.profiles.length === 0 && hasUnknownModel(layout)
  return (
    <TableRow data-layout={layout.id}>
      <TableCell className="max-w-64 truncate font-medium">{layout.name}</TableCell>
      <TableCell className="max-w-56">
        <LayoutDevice layout={layout} />
      </TableCell>
      <TableCell className="whitespace-nowrap text-muted-foreground tabular-nums">
        {t("layouts:page.contents", {
          pages: t("layouts:page.pages", { count: layout.pages }),
          keys: t("layouts:page.keys", { count: layout.bindings }),
        })}
      </TableCell>
      <TableCell className="max-w-64">
        <UsedBy layout={layout} />
      </TableCell>
      <TableCell>
        <LayoutCompatibilitySummary layout={layout} />
      </TableCell>
      <TableCell>
        <div className="flex items-center justify-end gap-0.5">
          <Tooltip>
            <TooltipTrigger asChild>
              {/* A wrapper: disabled buttons get no pointer events, so no tooltip. */}
              <span tabIndex={closed ? 0 : -1} className="inline-flex">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("layouts:page.openLabel", { layout: layout.name })}
                  disabled={closed}
                  onClick={() => {
                    open(layout)
                  }}
                >
                  <SquarePen />
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>
              {closed ? t("layouts:page.unknownModel") : t("layouts:page.openTooltip")}
            </TooltipContent>
          </Tooltip>
          {dialog}
          <AssignmentPopover layout={layout} onChanged={onChanged} />
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("layouts:page.more", { layout: layout.name })}
              >
                <MoreHorizontal />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem
                onSelect={() => {
                  onDialog({ kind: "rename", layout })
                }}
              >
                <Pencil />
                {t("layouts:page.menu.rename")}
              </DropdownMenuItem>
              <DropdownMenuItem
                onSelect={() => {
                  onDialog({ kind: "duplicate", layout })
                }}
              >
                <Copy />
                {t("layouts:page.menu.duplicate")}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                variant="destructive"
                onSelect={() => {
                  onDialog({ kind: "delete", layout })
                }}
              >
                <Trash2 />
                {t("layouts:page.menu.delete")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </TableCell>
    </TableRow>
  )
}

function UsedBy({ layout }: { layout: SharedLayoutSummary }) {
  const { t } = useTranslation()
  const format = useFormat()
  if (layout.profiles.length === 0) {
    return <span className="text-muted-foreground">{t("layouts:page.notUsed")}</span>
  }
  const sorted = [...layout.profiles].sort((a, b) => format.compare(a.profileName, b.profileName))
  return (
    <ul
      className="flex flex-wrap gap-1"
      aria-label={t("layouts:page.profilesUsing", { layout: layout.name })}
    >
      {sorted.map((p) => (
        <li key={p.profileId}>
          <Badge
            variant={p.isDefault ? "secondary" : "outline"}
            title={
              p.isDefault
                ? t("layouts:page.defaultOf", { profile: p.profileName, model: modelLabel(layout) })
                : undefined
            }
          >
            {p.profileName}
            {p.isDefault && (
              <span className="text-muted-foreground">· {t("layouts:menu.defaultMark")}</span>
            )}
          </Badge>
        </li>
      ))}
    </ul>
  )
}

/** A checkbox per profile: assigns the layout to it or removes it. */
function AssignmentPopover({
  layout,
  onChanged,
}: {
  layout: SharedLayoutSummary
  onChanged: () => void
}) {
  const { t } = useTranslation()
  const { profiles } = useAppContext()
  const [pending, setPending] = useState<string | null>(null)
  const unknown = hasUnknownModel(layout)
  const assigned = new Set(layout.profiles.map((p) => p.profileId))
  const toggle = async (profile: ProfileSummary, assign: boolean) => {
    setPending(profile.id)
    try {
      if (assign) await api.assignLayout(profile.id, layout.id)
      else await api.unassignLayout(profile.id, layout.id)
      toast.success(
        assign
          ? t("layouts:page.toast.assigned", { profile: profile.name })
          : t("layouts:page.toast.unassigned", { profile: profile.name }),
      )
      onChanged()
    } catch (e) {
      toast.error(errorMessage(e))
    } finally {
      setPending(null)
    }
  }
  return (
    <Popover>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("layouts:page.profilesUsing", { layout: layout.name })}
            >
              <Users />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>{t("layouts:page.assignTooltip")}</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="w-64 p-3">
        <fieldset className="grid gap-2">
          <legend className="mb-1 text-xs font-medium text-muted-foreground">
            {t("layouts:page.profilesUsing", { layout: layout.name })}
          </legend>
          {profiles.profiles.map((profile) => {
            const checked = assigned.has(profile.id)
            const id = `assign-${layout.id}-${profile.id}`
            return (
              <div key={profile.id} className="flex items-center gap-2">
                <Checkbox
                  id={id}
                  checked={checked}
                  disabled={pending !== null || (!checked && unknown)}
                  onCheckedChange={(value) => {
                    void toggle(profile, value === true)
                  }}
                />
                <Label htmlFor={id} className="font-normal">
                  {profile.name}
                </Label>
              </div>
            )
          })}
          {unknown && (
            <p className="text-xs text-muted-foreground">{t("layouts:page.unknownModelAssign")}</p>
          )}
        </fieldset>
      </PopoverContent>
    </Popover>
  )
}

/** Asks for a name: rename and duplicate. */
function NameDialog({
  title,
  description,
  label,
  confirmLabel,
  initial,
  onClose,
  onSubmit,
}: {
  title: string
  description?: string
  label: string
  confirmLabel: string
  initial: string
  onClose: () => void
  onSubmit: (name: string) => Promise<void>
}) {
  const [name, setName] = useState(initial)
  const trimmed = name.trim()
  return (
    <FormDialog
      open
      size="compact"
      title={title}
      description={description}
      submitLabel={confirmLabel}
      canSubmit={Boolean(trimmed)}
      dirty={name !== initial}
      onClose={onClose}
      onSubmit={() => onSubmit(trimmed)}
    >
      <div className="grid gap-1.5">
        <Label htmlFor="layout-name">{label}</Label>
        <Input
          id="layout-name"
          autoFocus
          value={name}
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
    </FormDialog>
  )
}

/**
 * Lists the profiles that use the layout and passes exactly those to the
 * backend, which refuses if they changed in the meantime.
 */
function DeleteDialog({
  layout,
  onClose,
  onDeleted,
}: {
  layout: SharedLayoutSummary
  onClose: () => void
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  const users = layout.profiles
  return (
    <DestructiveDialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={t("layouts:page.delete.title", { layout: layout.name })}
      description={
        users.length === 0
          ? t("layouts:page.delete.unused")
          : t("layouts:page.delete.usedBy", { count: users.length })
      }
      // Stays open, busy, until the backend answered; a refusal is shown inline.
      onConfirm={async () => {
        await api.deleteSharedLayout(
          layout.id,
          users.map((p) => p.profileId),
        )
        toast.success(t("layouts:page.toast.deleted"))
        onDeleted()
      }}
    >
      {users.length > 0 && (
        <div className="grid gap-2 text-sm text-muted-foreground">
          <ul className="list-disc pl-5" aria-label={t("layouts:page.delete.listLabel")}>
            {users.map((p) => (
              <li key={p.profileId}>
                {p.isDefault
                  ? t("layouts:page.delete.defaultEntry", { profile: p.profileName })
                  : p.profileName}
              </li>
            ))}
          </ul>
          <p>{t("layouts:page.delete.fallback", { count: users.length })}</p>
        </div>
      )}
    </DestructiveDialog>
  )
}

/** Name, device model and (optionally) the profile to assign a new shared layout to. */
function NewSharedLayoutDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: () => void
}) {
  // Mounted only while open, so every opening starts with an empty form.
  if (!open) return null
  return (
    <NewSharedLayoutForm
      onCancel={() => {
        onOpenChange(false)
      }}
      onCreated={onCreated}
    />
  )
}

function NewSharedLayoutForm({
  onCancel,
  onCreated,
}: {
  onCancel: () => void
  onCreated: () => void
}) {
  const { t } = useTranslation()
  const { profiles, editingProfileId } = useAppContext()
  const selection = useContext(EditorSelectionContext)
  const { state: models } = useAsync(() => api.listDeviceModels(), "models")
  const catalog = models.status === "ready" ? models.data : []
  const ordered = [...catalog.filter(isSupported), ...catalog.filter((m) => !isSupported(m))]
  const [name, setName] = useState("")
  const [model, setModel] = useState<string | null>(null)
  const [profile, setProfile] = useState(editingProfileId ?? NO_PROFILE)
  // The model chosen in the toolbar, else the first supported one.
  const modelId =
    model ??
    (catalog.some((m) => m.id === selection?.modelId) ? selection?.modelId : ordered[0]?.id) ??
    null
  const trimmed = name.trim()
  return (
    <FormDialog
      open
      size="compact"
      title={t("layouts:page.create.title")}
      description={t("layouts:page.create.description")}
      submitLabel={t("layouts:page.create.submit")}
      canSubmit={Boolean(trimmed && modelId)}
      dirty={Boolean(trimmed)}
      onClose={onCancel}
      onSubmit={async () => {
        if (!trimmed || !modelId) return
        await api.createSharedLayout(modelId, trimmed, profile === NO_PROFILE ? null : profile)
        toast.success(t("layouts:page.toast.created"))
        onCreated()
      }}
    >
      <div className="grid gap-1.5">
        <Label htmlFor="new-layout-name">{t("layouts:page.create.name")}</Label>
        <Input
          id="new-layout-name"
          autoFocus
          value={name}
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="new-layout-model">{t("layouts:page.create.model")}</Label>
        {models.status === "error" ? (
          <FieldError id="new-layout-model-error" message={errorMessage(models.error)} />
        ) : (
          <Select value={modelId ?? ""} onValueChange={setModel}>
            <SelectTrigger id="new-layout-model" className="w-full">
              <SelectValue
                placeholder={
                  models.status === "loading"
                    ? t("common:status.loading")
                    : t("layouts:page.create.choose")
                }
              />
            </SelectTrigger>
            <SelectContent>
              {ordered.map((m) => (
                <SelectItem key={m.id} value={m.id}>
                  {m.name}
                  {!isSupported(m) && (
                    <span className="text-muted-foreground">
                      {t("layouts:page.create.experimental")}
                    </span>
                  )}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="new-layout-profile">{t("layouts:page.create.profile")}</Label>
        <Select value={profile} onValueChange={setProfile}>
          <SelectTrigger id="new-layout-profile" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NO_PROFILE}>{t("layouts:page.create.noProfile")}</SelectItem>
            {profiles.profiles.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    </FormDialog>
  )
}
