import { Copy, MoreHorizontal, Pencil, Plus, Search, SquarePen, Trash2, Users } from "lucide-react"
import { useContext, useMemo, useRef, useState } from "react"
import { toast } from "sonner"

import { LayoutCompatibilitySummary, LayoutDevice } from "@/components/layout-compatibility"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { copyName, hasUnknownModel, modelLabel } from "@/lib/shared-layouts"
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
      title="Layouts"
      description="Layouts stored once and assigned to any number of profiles"
      actions={[
        {
          id: "layouts.new",
          label: "New layout",
          icon: Plus,
          placement: "primary",
          onSelect: () => {
            setCreating(true)
          },
        },
        {
          id: "layouts.search",
          label: "Search",
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
      <NewLayoutDialog
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
  const [query, setQuery] = useState("")
  const [dialog, setDialog] = useState<RowDialog | null>(null)
  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return layouts
      .filter((l) => !needle || l.name.toLowerCase().includes(needle))
      .sort((a, b) => a.name.localeCompare(b.name))
  }, [layouts, query])

  if (layouts.length === 0) {
    return (
      <EmptyState title="No layouts yet">
        <p>A layout holds the pages and keys of a device; assign it to any number of profiles.</p>
        <Button size="sm" className="mt-3" onClick={onNew}>
          <Plus />
          New layout
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
          placeholder="Search layouts"
          aria-label="Search layouts"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
        />
      </div>
      {shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">No layouts match.</p>
      ) : (
        <Table className="text-sm">
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Device</TableHead>
              <TableHead>Pages / keys</TableHead>
              <TableHead>Used by</TableHead>
              <TableHead>Compatibility</TableHead>
              <TableHead className="w-0 text-right">
                <span className="sr-only">Actions</span>
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
          title="Rename layout"
          label="Layout name"
          confirmLabel="Rename"
          initial={dialog.layout.name}
          onClose={close}
          onSubmit={async (name) => {
            await api.renameSharedLayout(dialog.layout.id, name)
            toast.success("Layout renamed")
            close()
            onChanged()
          }}
        />
      )}
      {dialog?.kind === "duplicate" && (
        <NameDialog
          title="Duplicate layout"
          description="An independent copy with its own pages; its keys run the same Action Library actions."
          label="Name of the copy"
          confirmLabel="Duplicate"
          initial={copyName(dialog.layout.name)}
          onClose={close}
          onSubmit={async (name) => {
            await api.duplicateSharedLayout(dialog.layout.id, name, null)
            toast.success("Layout duplicated")
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
  const open = useOpenLayout()
  const unused = layout.profiles.length === 0
  return (
    <TableRow data-layout={layout.id}>
      <TableCell className="max-w-64 truncate font-medium">{layout.name}</TableCell>
      <TableCell className="max-w-56">
        <LayoutDevice layout={layout} />
      </TableCell>
      <TableCell className="whitespace-nowrap text-muted-foreground tabular-nums">
        {layout.pages} {layout.pages === 1 ? "page" : "pages"} · {layout.bindings}{" "}
        {layout.bindings === 1 ? "key" : "keys"}
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
              <span tabIndex={unused ? 0 : -1} className="inline-flex">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Open ${layout.name} in the Device Editor`}
                  disabled={unused}
                  onClick={() => {
                    open(layout)
                  }}
                >
                  <SquarePen />
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>
              {unused ? "Assign it to a profile first" : "Open in Device Editor"}
            </TooltipContent>
          </Tooltip>
          <AssignmentPopover layout={layout} onChanged={onChanged} />
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" aria-label={`More actions for ${layout.name}`}>
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
                Rename…
              </DropdownMenuItem>
              <DropdownMenuItem
                onSelect={() => {
                  onDialog({ kind: "duplicate", layout })
                }}
              >
                <Copy />
                Duplicate…
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                variant="destructive"
                onSelect={() => {
                  onDialog({ kind: "delete", layout })
                }}
              >
                <Trash2 />
                Delete…
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </TableCell>
    </TableRow>
  )
}

function UsedBy({ layout }: { layout: SharedLayoutSummary }) {
  if (layout.profiles.length === 0) {
    return <span className="text-muted-foreground">Not used</span>
  }
  const sorted = [...layout.profiles].sort((a, b) => a.profileName.localeCompare(b.profileName))
  return (
    <ul className="flex flex-wrap gap-1" aria-label={`Profiles using ${layout.name}`}>
      {sorted.map((p) => (
        <li key={p.profileId}>
          <Badge
            variant={p.isDefault ? "secondary" : "outline"}
            title={
              p.isDefault
                ? `Default layout of ${p.profileName} for ${modelLabel(layout)}`
                : undefined
            }
          >
            {p.profileName}
            {p.isDefault && <span className="text-muted-foreground">· default</span>}
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
        assign ? `Assigned to ${profile.name}` : `Removed from ${profile.name}; the layout is kept`,
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
            <Button variant="ghost" size="icon-sm" aria-label={`Profiles using ${layout.name}`}>
              <Users />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Assign to profiles</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="w-64 p-3">
        <fieldset className="grid gap-2">
          <legend className="mb-1 text-xs font-medium text-muted-foreground">
            Profiles using {layout.name}
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
            <p className="text-xs text-muted-foreground">
              Its device model is unknown, so it cannot be assigned to more profiles.
            </p>
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
  const [busy, setBusy] = useState(false)
  const trimmed = name.trim()
  const submit = async () => {
    if (!trimmed) return
    setBusy(true)
    try {
      await onSubmit(trimmed)
    } catch (e) {
      toast.error(errorMessage(e))
      setBusy(false)
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="sm:max-w-md">
        <form
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault()
            void submit()
          }}
        >
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            {description && <DialogDescription>{description}</DialogDescription>}
          </DialogHeader>
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
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={!trimmed || busy}>
              {confirmLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
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
  const users = layout.profiles
  const remove = async () => {
    try {
      await api.deleteSharedLayout(
        layout.id,
        users.map((p) => p.profileId),
      )
      toast.success("Layout deleted")
      onDeleted()
    } catch (e) {
      toast.error(errorMessage(e))
      onClose()
    }
  }
  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete “{layout.name}”?</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="grid gap-2">
              {users.length === 0 ? (
                <p>No profile uses this layout. Its pages and keys are deleted.</p>
              ) : (
                <>
                  <p>
                    {users.length === 1 ? "This profile uses" : "These profiles use"} the layout:
                  </p>
                  <ul className="list-disc pl-5" aria-label="Profiles using the layout">
                    {users.map((p) => (
                      <li key={p.profileId}>
                        {p.profileName}
                        {p.isDefault ? " (default layout)" : ""}
                      </li>
                    ))}
                  </ul>
                  <p>
                    They fall back to their other layouts. The profiles themselves are not deleted.
                  </p>
                </>
              )}
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-white hover:bg-destructive/90"
            onClick={(e) => {
              // Closes once the backend answered.
              e.preventDefault()
              void remove()
            }}
          >
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** Name, device model and (optionally) the profile to assign it to. */
function NewLayoutDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: () => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        {open && (
          <NewLayoutForm
            onCancel={() => {
              onOpenChange(false)
            }}
            onCreated={onCreated}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function NewLayoutForm({ onCancel, onCreated }: { onCancel: () => void; onCreated: () => void }) {
  const { profiles, editingProfileId } = useAppContext()
  const selection = useContext(EditorSelectionContext)
  const { state: models } = useAsync(() => api.listDeviceModels(), "models")
  const catalog = models.status === "ready" ? models.data : []
  const ordered = [...catalog.filter(isSupported), ...catalog.filter((m) => !isSupported(m))]
  const [name, setName] = useState("")
  const [model, setModel] = useState<string | null>(null)
  const [profile, setProfile] = useState(editingProfileId ?? NO_PROFILE)
  const [busy, setBusy] = useState(false)
  // The model chosen in the toolbar, else the first supported one.
  const modelId =
    model ??
    (catalog.some((m) => m.id === selection?.modelId) ? selection?.modelId : ordered[0]?.id) ??
    null
  const trimmed = name.trim()
  const submit = async () => {
    if (!trimmed || !modelId) return
    setBusy(true)
    try {
      await api.createSharedLayout(modelId, trimmed, profile === NO_PROFILE ? null : profile)
      toast.success("Layout created")
      onCreated()
    } catch (e) {
      toast.error(errorMessage(e))
      setBusy(false)
    }
  }
  return (
    <form
      className="grid gap-4"
      onSubmit={(e) => {
        e.preventDefault()
        void submit()
      }}
    >
      <DialogHeader>
        <DialogTitle>New layout</DialogTitle>
        <DialogDescription>An empty layout for one device model.</DialogDescription>
      </DialogHeader>
      <div className="grid gap-1.5">
        <Label htmlFor="new-layout-name">Name</Label>
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
        <Label htmlFor="new-layout-model">Device model</Label>
        {models.status === "error" ? (
          <ErrorNotice error={models.error} />
        ) : (
          <Select value={modelId ?? ""} onValueChange={setModel}>
            <SelectTrigger id="new-layout-model" className="w-full">
              <SelectValue placeholder={models.status === "loading" ? "Loading…" : "Choose"} />
            </SelectTrigger>
            <SelectContent>
              {ordered.map((m) => (
                <SelectItem key={m.id} value={m.id}>
                  {m.name}
                  {!isSupported(m) && <span className="text-muted-foreground">(experimental)</span>}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="new-layout-profile">Assign to profile</Label>
        <Select value={profile} onValueChange={setProfile}>
          <SelectTrigger id="new-layout-profile" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NO_PROFILE}>No profile (unassigned)</SelectItem>
            {profiles.profiles.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" disabled={!trimmed || !modelId || busy}>
          Create
        </Button>
      </DialogFooter>
    </form>
  )
}
