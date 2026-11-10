import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { open as openDialog } from "@tauri-apps/plugin-dialog"
import {
  CircleAlert,
  CircleCheck,
  CircleDashed,
  FolderOpen,
  GripVertical,
  Pencil,
  Plus,
  Trash2,
  X,
} from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { ProjectSwitcher } from "@/components/project-switcher"
import { RequireProfile } from "@/components/require-profile"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useAsync } from "@/hooks/use-async"
import { useEditingProfile } from "@/hooks/use-profile"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { RESOURCE_KIND_LABELS } from "@/lib/format"
import type { Project, ProfileView, ResourceCheck, ResourceKind, ResourceStatus } from "@/lib/types"
import { cn } from "@/lib/utils"

export function ProjectsPage() {
  return (
    <Page title="Projects" description="Project context and the resources on this machine">
      <RequireProfile>{() => <Projects />}</RequireProfile>
    </Page>
  )
}

const STATUS: Record<
  ResourceStatus,
  { label: string; icon: typeof CircleCheck; className: string }
> = {
  ok: { label: "Found", icon: CircleCheck, className: "text-emerald-600" },
  missing: { label: "Missing", icon: CircleAlert, className: "text-destructive" },
  unbound: { label: "Not set on this machine", icon: CircleAlert, className: "text-amber-600" },
  unchecked: { label: "Not checked", icon: CircleDashed, className: "text-muted-foreground" },
}

function StatusIcon({ status }: { status: ResourceStatus }) {
  const { icon: Icon, className, label } = STATUS[status]
  return <Icon className={cn("size-3.5 shrink-0", className)} aria-label={label} />
}

function Projects() {
  const { state, reload, profileId } = useEditingProfile()
  const { revision } = useAppContext()
  const checks = useAsync(
    () => (profileId ? api.checkResources(profileId) : Promise.resolve([])),
    `${profileId ?? ""}:${revision}`,
  )
  if (state.status === "loading") return <Loading />
  if (state.status === "error") return <ErrorNotice error={state.error} />
  if (!profileId) return null
  const resourceChecks = checks.state.status === "ready" ? checks.state.data : []
  const reloadAll = () => {
    reload()
    checks.reload()
  }
  return (
    <Tabs defaultValue="projects">
      <TabsList>
        <TabsTrigger value="projects">Projects</TabsTrigger>
        <TabsTrigger value="machine">This machine</TabsTrigger>
      </TabsList>
      <TabsContent value="projects" className="grid gap-4">
        <ProjectCycle
          view={state.data}
          profileId={profileId}
          checks={resourceChecks}
          onChange={reloadAll}
        />
      </TabsContent>
      <TabsContent value="machine">
        <MachineResources checks={resourceChecks} onChange={reloadAll} />
      </TabsContent>
    </Tabs>
  )
}

type ProjectDialogState = { mode: "create" } | { mode: "edit"; project: Project } | null

function ProjectCycle({
  view,
  profileId,
  checks,
  onChange,
}: {
  view: ProfileView
  profileId: string
  checks: ResourceCheck[]
  onChange: () => void
}) {
  const { refresh } = useAppContext()
  const [dialog, setDialog] = useState<ProjectDialogState>(null)
  const projects = view.profile.projects
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )
  const changed = () => {
    onChange()
    refresh()
  }
  const onDragEnd = (event: DragEndEvent) => {
    const to = projects.findIndex((p) => p.id === event.over?.id)
    if (to < 0 || event.active.id === event.over?.id) return
    api
      .moveProject(profileId, String(event.active.id), to)
      .then(changed, (e: unknown) => toast.error(errorMessage(e)))
  }
  const statusOf = (key: string) => checks.find((c) => c.key === key)?.status ?? "unbound"

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Project switcher</CardTitle>
          <CardDescription>
            Switching only changes which project actions refer to. Nothing is checked out, restarted
            or launched.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ProjectSwitcher
            profileId={profileId}
            projects={projects}
            activeProjectId={view.activeProjectId}
          />
        </CardContent>
      </Card>
      <Card>
        <CardHeader className="flex flex-row items-start gap-2">
          <div className="grid flex-1 gap-1.5">
            <CardTitle>Cycle order</CardTitle>
            <CardDescription>
              Drag to reorder. The last project wraps around to the first.
            </CardDescription>
          </div>
          <Button
            size="sm"
            onClick={() => {
              setDialog({ mode: "create" })
            }}
          >
            <Plus /> New project
          </Button>
        </CardHeader>
        <CardContent>
          {projects.length === 0 ? (
            <EmptyState title="No projects in this profile" />
          ) : (
            <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
              <SortableContext
                items={projects.map((p) => p.id)}
                strategy={verticalListSortingStrategy}
              >
                <ol className="grid gap-2">
                  {projects.map((project, index) => (
                    <SortableProject
                      key={project.id}
                      project={project}
                      index={index}
                      active={project.id === view.activeProjectId}
                      isDefault={project.id === view.profile.defaultProject}
                      statusOf={statusOf}
                      onEdit={() => {
                        setDialog({ mode: "edit", project })
                      }}
                      onDelete={() => {
                        api
                          .deleteProject(profileId, project.id)
                          .then(() => {
                            toast.success("Project deleted")
                            changed()
                          })
                          .catch((e: unknown) => toast.error(errorMessage(e)))
                      }}
                    />
                  ))}
                </ol>
              </SortableContext>
            </DndContext>
          )}
        </CardContent>
      </Card>
      <Dialog
        open={dialog !== null}
        onOpenChange={(open) => {
          if (!open) setDialog(null)
        }}
      >
        <DialogContent className="sm:max-w-xl">
          {dialog && (
            <ProjectForm
              key={dialog.mode === "edit" ? dialog.project.id : "new"}
              profileId={profileId}
              project={dialog.mode === "edit" ? dialog.project : null}
              onDone={() => {
                setDialog(null)
                changed()
              }}
            />
          )}
        </DialogContent>
      </Dialog>
    </>
  )
}

function SortableProject({
  project,
  index,
  active,
  isDefault,
  statusOf,
  onEdit,
  onDelete,
}: {
  project: Project
  index: number
  active: boolean
  isDefault: boolean
  statusOf: (key: string) => ResourceStatus
  onEdit: () => void
  onDelete: () => void
}) {
  const { attributes, listeners, setNodeRef, transform, transition } = useSortable({
    id: project.id,
  })
  const resources = Object.entries(project.resources)
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className="flex items-start gap-2 rounded-lg border bg-card p-3 data-[active=true]:border-primary"
      data-active={active}
    >
      <button
        type="button"
        className="cursor-grab rounded p-1 text-muted-foreground hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        aria-label={`Reorder ${project.key}`}
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-4" />
      </button>
      <span className="w-5 pt-1 text-right text-sm text-muted-foreground tabular-nums">
        {index + 1}.
      </span>
      <div className="grid flex-1 gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-mono text-sm font-semibold">{project.key}</span>
          <span className="text-sm">{project.name}</span>
          {active && <Badge>Active</Badge>}
          {isDefault && <Badge variant="outline">Default</Badge>}
        </div>
        {resources.length > 0 && (
          <ul className="grid gap-0.5 text-xs text-muted-foreground">
            {resources.map(([role, key]) => (
              <li key={role} className="flex items-center gap-1">
                <StatusIcon status={statusOf(key)} />
                {role} → <span className="font-mono">{key}</span>
                <span className="text-muted-foreground/70">({STATUS[statusOf(key)].label})</span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <Button variant="ghost" size="icon" aria-label={`Edit ${project.key}`} onClick={onEdit}>
        <Pencil />
      </Button>
      <ConfirmDialog
        trigger={
          <Button variant="ghost" size="icon" aria-label={`Delete ${project.key}`}>
            <Trash2 />
          </Button>
        }
        title={`Delete ${project.key}?`}
        description="Actions that refer to its resources through the active project will no longer resolve while no project provides them."
        onConfirm={onDelete}
      />
    </li>
  )
}

interface ResourceRow {
  role: string
  key: string
}

function ProjectForm({
  profileId,
  project,
  onDone,
}: {
  profileId: string
  project: Project | null
  onDone: () => void
}) {
  const [key, setKey] = useState(project?.key ?? "")
  const [name, setName] = useState(project?.name ?? "")
  const [rows, setRows] = useState<ResourceRow[]>(
    Object.entries(project?.resources ?? {}).map(([role, resource]) => ({ role, key: resource })),
  )
  const [error, setError] = useState<string | null>(null)
  const update = (index: number, patch: Partial<ResourceRow>) => {
    setRows((current) => current.map((row, i) => (i === index ? { ...row, ...patch } : row)))
  }
  const submit = async (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    const resources = Object.fromEntries(
      rows.filter((r) => r.role.trim() || r.key.trim()).map((r) => [r.role, r.key]),
    )
    try {
      if (project) await api.updateProject(profileId, project.id, { key, name, resources })
      else await api.createProject(profileId, { key, name, resources })
      toast.success("Project saved")
      onDone()
    } catch (e) {
      setError(errorMessage(e))
    }
  }
  return (
    <form className="grid gap-4" onSubmit={(e) => void submit(e)}>
      <DialogHeader>
        <DialogTitle>{project ? `Edit ${project.key}` : "New project"}</DialogTitle>
        <DialogDescription>
          Resources map a role (for example <code>repository</code>) to a logical key such as
          <code className="mx-1">shop.repository</code>. Each machine maps keys to real paths.
        </DialogDescription>
      </DialogHeader>
      {error && <ErrorNotice error={error} />}
      <div className="grid grid-cols-[8rem_1fr] gap-4">
        <div className="grid gap-2">
          <Label htmlFor="project-key">Key</Label>
          <Input
            id="project-key"
            value={key}
            placeholder="SHOP"
            autoFocus
            onChange={(e) => {
              setKey(e.target.value)
            }}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="project-name">Name</Label>
          <Input
            id="project-name"
            value={name}
            placeholder="Online Shop"
            onChange={(e) => {
              setName(e.target.value)
            }}
          />
        </div>
      </div>
      <div className="grid gap-2">
        <Label>Resources</Label>
        {rows.map((row, index) => (
          <div key={index} className="grid grid-cols-[1fr_1fr_auto] gap-2">
            <Input
              aria-label="Role"
              value={row.role}
              placeholder="repository"
              onChange={(e) => {
                update(index, { role: e.target.value })
              }}
            />
            <Input
              aria-label="Resource key"
              value={row.key}
              placeholder="shop.repository"
              onChange={(e) => {
                update(index, { key: e.target.value })
              }}
            />
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label="Remove resource"
              onClick={() => {
                setRows((current) => current.filter((_, i) => i !== index))
              }}
            >
              <X />
            </Button>
          </div>
        ))}
        <div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              setRows((current) => [...current, { role: "", key: "" }])
            }}
          >
            <Plus /> Add resource
          </Button>
        </div>
      </div>
      <DialogFooter>
        <Button type="submit" disabled={!key.trim()}>
          Save
        </Button>
      </DialogFooter>
    </form>
  )
}

const KINDS = Object.keys(RESOURCE_KIND_LABELS) as ResourceKind[]
const PICKABLE: ResourceKind[] = ["directory", "application", "terminal"]

function MachineResources({ checks, onChange }: { checks: ResourceCheck[]; onChange: () => void }) {
  const [editing, setEditing] = useState<ResourceCheck | "new" | null>(null)
  return (
    <Card>
      <CardHeader className="flex flex-row items-start gap-2">
        <div className="grid flex-1 gap-1.5">
          <CardTitle>Resources on this machine</CardTitle>
          <CardDescription>
            Logical resources used by projects, mapped to paths, applications or URLs on this
            computer. These mappings are never part of a profile.
          </CardDescription>
        </div>
        <Button
          size="sm"
          onClick={() => {
            setEditing("new")
          }}
        >
          <Plus /> Add mapping
        </Button>
      </CardHeader>
      <CardContent className="grid gap-2">
        {checks.length === 0 && <EmptyState title="No resources used or mapped yet" />}
        {checks.map((check) => (
          <div key={check.key} className="flex flex-wrap items-center gap-2 rounded-lg border p-3">
            <StatusIcon status={check.status} />
            <div className="grid min-w-0 flex-1 gap-0.5">
              <span className="font-mono text-sm">{check.key}</span>
              <span className="truncate text-xs text-muted-foreground select-text">
                {check.value
                  ? `${check.kind ? RESOURCE_KIND_LABELS[check.kind] : ""}: ${check.value}`
                  : "Not set on this machine"}
              </span>
              {check.usedBy.length > 0 && (
                <span className="text-xs text-muted-foreground">
                  Used by {check.usedBy.join(", ")}
                </span>
              )}
            </div>
            <Badge variant={check.status === "ok" ? "secondary" : "outline"}>
              {STATUS[check.status].label}
            </Badge>
            <Button
              variant="ghost"
              size="icon"
              aria-label={`Edit ${check.key}`}
              onClick={() => {
                setEditing(check)
              }}
            >
              <Pencil />
            </Button>
            {check.value && (
              <ConfirmDialog
                trigger={
                  <Button variant="ghost" size="icon" aria-label={`Remove ${check.key}`}>
                    <Trash2 />
                  </Button>
                }
                title={`Remove the mapping for ${check.key}?`}
                description="Actions using this resource cannot resolve it on this machine until it is mapped again."
                confirmLabel="Remove"
                onConfirm={() => {
                  api
                    .removeResourceBinding(check.key)
                    .then(onChange, (e: unknown) => toast.error(errorMessage(e)))
                }}
              />
            )}
          </div>
        ))}
      </CardContent>
      <Dialog
        open={editing !== null}
        onOpenChange={(open) => {
          if (!open) setEditing(null)
        }}
      >
        <DialogContent>
          {editing && (
            <BindingForm
              key={editing === "new" ? "new" : editing.key}
              check={editing === "new" ? null : editing}
              onDone={() => {
                setEditing(null)
                onChange()
              }}
            />
          )}
        </DialogContent>
      </Dialog>
    </Card>
  )
}

function BindingForm({ check, onDone }: { check: ResourceCheck | null; onDone: () => void }) {
  const [key, setKey] = useState(check?.key ?? "")
  const [kind, setKind] = useState<ResourceKind>(check?.kind ?? "directory")
  const [value, setValue] = useState(check?.value ?? "")
  const [error, setError] = useState<string | null>(null)
  const browse = async () => {
    try {
      const picked = await openDialog({ directory: kind === "directory", multiple: false })
      if (typeof picked === "string") setValue(picked)
    } catch (e) {
      setError(errorMessage(e))
    }
  }
  const submit = async (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    try {
      await api.setResourceBinding(key, kind, value)
      toast.success("Mapping saved")
      onDone()
    } catch (e) {
      setError(errorMessage(e))
    }
  }
  return (
    <form className="grid gap-4" onSubmit={(e) => void submit(e)}>
      <DialogHeader>
        <DialogTitle>{check ? `Map ${check.key}` : "New mapping"}</DialogTitle>
        <DialogDescription>Stored for this computer only.</DialogDescription>
      </DialogHeader>
      {error && <ErrorNotice error={error} />}
      <div className="grid grid-cols-[1fr_12rem] gap-4">
        <div className="grid gap-2">
          <Label htmlFor="binding-key">Resource key</Label>
          <Input
            id="binding-key"
            value={key}
            disabled={Boolean(check)}
            placeholder="shop.repository"
            onChange={(e) => {
              setKey(e.target.value)
            }}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="binding-kind">Kind</Label>
          <Select
            value={kind}
            onValueChange={(next) => {
              setKind(next as ResourceKind)
            }}
          >
            <SelectTrigger id="binding-kind" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {KINDS.map((option) => (
                <SelectItem key={option} value={option}>
                  {RESOURCE_KIND_LABELS[option]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      <div className="grid gap-2">
        <Label htmlFor="binding-value">Value</Label>
        <div className="flex gap-2">
          <Input
            id="binding-value"
            value={value}
            placeholder={kind === "url" ? "https://…" : "Path, application, container name…"}
            onChange={(e) => {
              setValue(e.target.value)
            }}
          />
          {PICKABLE.includes(kind) && (
            <Button type="button" variant="outline" onClick={() => void browse()}>
              <FolderOpen /> Browse
            </Button>
          )}
        </div>
      </div>
      <DialogFooter>
        <Button type="submit" disabled={!key.trim() || !value.trim()}>
          Save
        </Button>
      </DialogFooter>
    </form>
  )
}
