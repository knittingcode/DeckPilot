import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useDraggable,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  ArrowLeft,
  Copy,
  Folder,
  FolderPlus,
  Home,
  Info,
  ListOrdered,
  Plus,
  RotateCcw,
  Save,
  Star,
  Trash2,
  X,
} from "lucide-react"
import { useEffect, useMemo, useRef, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { DeviceCanvas } from "@/components/device-canvas"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useAsync } from "@/hooks/use-async"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { ActionIcon } from "@/components/action-icon"
import { SUPPORT } from "@/lib/format"
import {
  addPage,
  bindingsFor,
  controlKey,
  controlLabel,
  controlsOf,
  defaultTrigger,
  deletePage,
  page as findPage,
  removeBinding,
  renamePage,
  sameTrigger,
  setBinding,
  supportedTriggers,
  targetLabel,
  triggerKey,
  triggerLabel,
} from "@/lib/layout-model"
import type {
  BindingTarget,
  ControlId,
  DeviceLayout,
  DeviceModel,
  KeyFace,
  ProfileView,
  Trigger,
} from "@/lib/types"
import { cn } from "@/lib/utils"

export function LayoutDesignerPage() {
  return (
    <Page
      title="Layout Designer"
      description="Assign configured actions to device controls, with or without hardware"
    >
      <RequireProfile>{(profileId) => <Designer initialProfileId={profileId} />}</RequireProfile>
    </Page>
  )
}

function Designer({ initialProfileId }: { initialProfileId: string }) {
  const { profiles, revision } = useAppContext()
  const [profileId, setProfileId] = useState(initialProfileId)
  const view = useAsync(() => api.getProfile(profileId), `${profileId}:${revision}`)
  const models = useAsync(() => api.listDeviceModels(), "models").state
  const [modelId, setModelId] = useState<string | null>(null)
  const [layoutId, setLayoutId] = useState<string | null>(null)
  const dirty = useRef(false)

  const guard = (change: () => void) => {
    if (dirty.current) {
      toast.warning("Save or revert the current layout first")
      return
    }
    change()
  }

  if (view.state.status === "loading" || models.status === "loading") return <Loading />
  if (view.state.status === "error") return <ErrorNotice error={view.state.error} />
  if (models.status === "error") return <ErrorNotice error={models.error} />
  const profile = view.state.data.profile
  const model =
    models.data.find((m) => m.id === modelId) ??
    models.data.find((m) => profile.layouts.some((l) => l.model === m.id)) ??
    models.data[0]
  if (!model) return <EmptyState title="No device models available" />
  const layouts = profile.layouts.filter((l) => l.model === model.id)
  const activeForModel = profile.activeLayouts?.[model.id] ?? layouts[0]?.id ?? null
  const layout =
    layouts.find((l) => l.id === layoutId) ??
    layouts.find((l) => l.id === activeForModel) ??
    layouts[0]

  return (
    <div className="grid gap-4">
      <Card>
        <CardContent className="flex flex-wrap items-end gap-4">
          <div className="grid gap-2">
            <Label htmlFor="designer-profile">Profile</Label>
            <Select
              value={profileId}
              onValueChange={(id) => {
                guard(() => {
                  setProfileId(id)
                  setLayoutId(null)
                })
              }}
            >
              <SelectTrigger id="designer-profile" className="w-48">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {profiles.profiles.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.name}
                    {p.id === profiles.activeProfileId ? " (on device)" : ""}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-2">
            <Label htmlFor="designer-model">Device model</Label>
            <Select
              value={model.id}
              onValueChange={(id) => {
                guard(() => {
                  setModelId(id)
                  setLayoutId(null)
                })
              }}
            >
              <SelectTrigger id="designer-model" className="w-52">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {models.data.map((m) => (
                  <SelectItem key={m.id} value={m.id}>
                    {m.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <LayoutToolbar
            profileId={profileId}
            model={model}
            layouts={layouts}
            current={layout ?? null}
            activeForModel={activeForModel}
            onSelect={(id) => {
              guard(() => {
                setLayoutId(id)
              })
            }}
            onChanged={(id) => {
              if (id) setLayoutId(id)
              view.reload()
            }}
          />
        </CardContent>
      </Card>

      {model.support !== "verified" && (
        <Alert>
          <Info />
          <AlertTitle>{SUPPORT[model.support].label}</AlertTitle>
          <AlertDescription>{SUPPORT[model.support].detail}</AlertDescription>
        </Alert>
      )}

      {layout ? (
        <LayoutEditor
          key={`${profileId}:${layout.id}`}
          profileId={profileId}
          view={view.state.data}
          model={model}
          saved={layout}
          onDirtyChange={(value) => {
            dirty.current = value
          }}
          onSaved={() => {
            dirty.current = false
            view.reload()
          }}
        />
      ) : (
        <EmptyState title={`No layout for ${model.name} in this profile`}>
          Create one with “New layout”. Layouts can be designed without the device.
        </EmptyState>
      )}
    </div>
  )
}

function LayoutToolbar({
  profileId,
  model,
  layouts,
  current,
  activeForModel,
  onSelect,
  onChanged,
}: {
  profileId: string
  model: DeviceModel
  layouts: DeviceLayout[]
  current: DeviceLayout | null
  activeForModel: string | null
  onSelect: (id: string) => void
  onChanged: (id?: string) => void
}) {
  const { refresh } = useAppContext()
  const [creating, setCreating] = useState(false)
  const run = (operation: () => Promise<string | undefined>, message: string) => {
    operation().then(
      (id) => {
        toast.success(message)
        onChanged(id)
        refresh()
      },
      (e: unknown) => toast.error(errorMessage(e)),
    )
  }
  return (
    <>
      <div className="grid gap-2">
        <Label htmlFor="designer-layout">Layout</Label>
        <Select value={current?.id ?? ""} onValueChange={onSelect} disabled={layouts.length === 0}>
          <SelectTrigger id="designer-layout" className="w-52">
            <SelectValue placeholder="No layouts" />
          </SelectTrigger>
          <SelectContent>
            {layouts.map((l) => (
              <SelectItem key={l.id} value={l.id}>
                {l.name}
                {l.id === activeForModel ? " ★" : ""}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          onClick={() => {
            setCreating(true)
          }}
        >
          <Plus /> New layout
        </Button>
        {current && (
          <>
            <Button
              variant="outline"
              onClick={() => {
                run(
                  () => api.duplicateLayout(profileId, current.id, `${current.name} (copy)`),
                  "Layout duplicated",
                )
              }}
            >
              <Copy /> Duplicate
            </Button>
            <Button
              variant="outline"
              disabled={current.id === activeForModel}
              title="Use this layout for the model"
              onClick={() => {
                run(async () => {
                  await api.activateLayout(profileId, current.id)
                  return current.id
                }, "Layout activated")
              }}
            >
              <Star /> {current.id === activeForModel ? "Active" : "Make active"}
            </Button>
            <ConfirmDialog
              trigger={
                <Button variant="ghost">
                  <Trash2 /> Delete
                </Button>
              }
              title={`Delete “${current.name}”?`}
              description="The layout and its pages are removed. Actions and workflows stay in the library."
              onConfirm={() => {
                run(async () => {
                  await api.deleteLayout(profileId, current.id)
                  return undefined
                }, "Layout deleted")
              }}
            />
          </>
        )}
      </div>
      <NewLayoutDialog
        open={creating}
        model={model}
        onClose={() => {
          setCreating(false)
        }}
        onCreate={(name, generate) => {
          setCreating(false)
          run(
            async () => (await api.createLayout(profileId, model.id, name, generate)).id,
            "Layout created",
          )
        }}
      />
    </>
  )
}

function NewLayoutDialog({
  open,
  model,
  onClose,
  onCreate,
}: {
  open: boolean
  model: DeviceModel
  onClose: () => void
  onCreate: (name: string, generate: boolean) => void
}) {
  const [name, setName] = useState("")
  const [generate, setGenerate] = useState(true)
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
    >
      <DialogContent>
        <form
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault()
            onCreate(name, generate)
            setName("")
          }}
        >
          <DialogHeader>
            <DialogTitle>New layout for {model.name}</DialogTitle>
          </DialogHeader>
          <div className="grid gap-2">
            <Label htmlFor="layout-name">Name</Label>
            <Input
              id="layout-name"
              value={name}
              autoFocus
              placeholder={`${model.name} layout`}
              onChange={(e) => {
                setName(e.target.value)
              }}
            />
          </div>
          <label className="flex items-center gap-3 text-sm">
            <Switch checked={generate} onCheckedChange={setGenerate} />
            Place the profile&apos;s actions and workflows on the keys
          </label>
          <DialogFooter>
            <Button type="submit">Create</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

type PaletteItem = { id: string; label: string; icon: React.ReactNode; target: BindingTarget }

function PaletteEntry({ item }: { item: PaletteItem }) {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({ id: item.id })
  return (
    <li
      ref={setNodeRef}
      className={cn(
        "flex min-w-0 cursor-grab items-center gap-2 rounded-md border bg-card px-2 py-1.5 text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
        isDragging && "opacity-60",
      )}
      aria-label={`Drag ${item.label} onto a control`}
      {...attributes}
      {...listeners}
    >
      {item.icon}
      <span className="min-w-0 flex-1 truncate" title={item.label}>
        {item.label}
      </span>
    </li>
  )
}

function encodeTarget(target: BindingTarget | undefined): string {
  if (!target) return "none"
  switch (target.type) {
    case "action":
      return `action:${target.action}`
    case "workflow":
      return `workflow:${target.workflow}`
    case "openPage":
      return `page:${target.page}`
    case "back":
      return "back"
  }
}

function decodeTarget(value: string): BindingTarget | null {
  if (value === "back") return { type: "back" }
  const [kind, id] = [value.slice(0, value.indexOf(":")), value.slice(value.indexOf(":") + 1)]
  if (kind === "action") return { type: "action", action: id }
  if (kind === "workflow") return { type: "workflow", workflow: id }
  if (kind === "page") return { type: "openPage", page: id }
  return null
}

function parseControl(key: string): ControlId | null {
  const [kind, index] = key.split(":")
  if (kind !== "key" && kind !== "encoder" && kind !== "touchZone") return null
  return { kind, index: Number(index) }
}

function LayoutEditor({
  profileId,
  view,
  model,
  saved,
  onDirtyChange,
  onSaved,
}: {
  profileId: string
  view: ProfileView
  model: DeviceModel
  saved: DeviceLayout
  onDirtyChange: (dirty: boolean) => void
  onSaved: () => void
}) {
  const { profiles, refresh } = useAppContext()
  const [draft, setDraft] = useState(saved)
  const [pageId, setPageId] = useState(saved.homePage)
  const [selected, setSelected] = useState<ControlId | null>(null)
  const [previewProject, setPreviewProject] = useState<string | null>(view.activeProjectId)
  const [faces, setFaces] = useState(new Map<string, KeyFace>())
  const [errors, setErrors] = useState<string[]>([])
  const [newPage, setNewPage] = useState("")
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved)
  const { actions, workflows, projects } = view.profile
  const currentPage = findPage(draft, pageId) ?? draft.pages[0]

  useEffect(() => {
    onDirtyChange(dirty)
  }, [dirty, onDirtyChange])

  // Preview faces exactly as the agent would render them, debounced.
  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(() => {
      api.previewLayout(profileId, draft, pageId, previewProject).then(
        (result) => {
          if (!cancelled) setFaces(new Map(result.map((r) => [controlKey(r.control), r.face])))
        },
        () => undefined,
      )
    }, 150)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [profileId, draft, pageId, previewProject])

  const pageBindings = useMemo(() => currentPage?.bindings ?? [], [currentPage])
  const icons = useMemo(() => {
    const map = new Map<string, string | null | undefined>()
    for (const binding of pageBindings) {
      if (binding.target.type === "action") {
        const id = binding.target.action
        const key = controlKey(binding.control)
        if (!map.has(key) || binding.trigger.type === "press") {
          map.set(key, actions.find((a) => a.id === id)?.icon)
        }
      }
    }
    return map
  }, [pageBindings, actions])
  const previewed = projects.find((p) => p.id === previewProject)
  const captions = useMemo(() => {
    const render = (text: string) =>
      text
        .replaceAll("{profile}", view.profile.name)
        .replaceAll("{project.key}", previewed?.key ?? "-")
        .replaceAll("{project.name}", previewed?.name ?? "-")
    const map = new Map<string, string>()
    for (const control of controlsOf(model).encoders) {
      const labels = bindingsFor(draft, pageId, control).map(
        (b) =>
          `${triggerLabel(b.trigger)}: ${render(b.label ?? targetLabel(b.target, actions, workflows, draft))}`,
      )
      map.set(controlKey(control), labels.join(" · "))
    }
    return map
  }, [model, draft, pageId, actions, workflows, previewed, view.profile.name])

  const palette: PaletteItem[] = [
    ...actions.map((a): PaletteItem => ({
      id: `action:${a.id}`,
      label: a.name,
      icon: <ActionIcon name={a.icon} className="size-4 shrink-0" />,
      target: { type: "action", action: a.id },
    })),
    ...workflows.map((w): PaletteItem => ({
      id: `workflow:${w.id}`,
      label: w.name,
      icon: <ListOrdered className="size-4 shrink-0" />,
      target: { type: "workflow", workflow: w.id },
    })),
    {
      id: "back",
      label: "Back",
      icon: <ArrowLeft className="size-4 shrink-0" />,
      target: { type: "back" },
    },
    ...draft.pages
      .filter((p) => p.id !== pageId)
      .map((p): PaletteItem => ({
        id: `page:${p.id}`,
        label: `Open ${p.name}`,
        icon: <Folder className="size-4 shrink-0" />,
        target: { type: "openPage", page: p.id },
      })),
  ]

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor),
  )
  const onDragEnd = (event: DragEndEvent) => {
    const control = event.over ? parseControl(String(event.over.id)) : null
    const item = palette.find((p) => p.id === event.active.id)
    if (!control || !item) return
    const trigger = defaultTrigger(model, draft, pageId, control)
    if (!trigger) return
    setDraft(setBinding(draft, pageId, { control, trigger, target: item.target, label: null }))
    setSelected(control)
  }

  const save = async () => {
    setErrors([])
    try {
      await api.saveLayout(profileId, draft)
      toast.success(
        profiles.activeProfileId === profileId
          ? "Layout saved and sent to the agent"
          : "Layout saved",
      )
      onSaved()
      refresh()
    } catch (e) {
      setErrors(errorMessage(e).split("\n"))
    }
  }

  return (
    <DndContext sensors={sensors} onDragEnd={onDragEnd}>
      <div className="grid gap-4 xl:grid-cols-[14rem_1fr] 2xl:grid-cols-[16rem_1fr_20rem]">
        <Card className="h-fit xl:row-span-2 2xl:row-span-1">
          <CardHeader>
            <CardTitle className="text-sm">Library</CardTitle>
            <CardDescription>Drag onto a control, or select a control and choose.</CardDescription>
          </CardHeader>
          <CardContent>
            {palette.length === 1 ? (
              <p className="text-sm text-muted-foreground">Create actions in the Action Library.</p>
            ) : (
              <ul className="grid max-h-[28rem] gap-1 overflow-y-auto">
                {palette.map((item) => (
                  <PaletteEntry key={item.id} item={item} />
                ))}
              </ul>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="gap-3">
            <div className="flex flex-wrap items-center gap-2">
              <Input
                aria-label="Layout name"
                className="h-8 w-56 font-medium"
                value={draft.name}
                onChange={(e) => {
                  setDraft({ ...draft, name: e.target.value })
                }}
              />
              {dirty && <Badge variant="outline">Unsaved</Badge>}
              <div className="flex-1" />
              <Button
                variant="outline"
                size="sm"
                disabled={!dirty}
                onClick={() => {
                  setDraft(saved)
                  setErrors([])
                }}
              >
                <RotateCcw /> Revert
              </Button>
              <Button size="sm" disabled={!dirty} onClick={() => void save()}>
                <Save /> Save
              </Button>
            </div>
            <div className="flex flex-wrap items-center gap-1" role="tablist" aria-label="Pages">
              {draft.pages.map((p) => (
                <Button
                  key={p.id}
                  role="tab"
                  aria-selected={p.id === pageId}
                  variant={p.id === pageId ? "secondary" : "ghost"}
                  size="sm"
                  onClick={() => {
                    setPageId(p.id)
                    setSelected(null)
                  }}
                >
                  {p.id === draft.homePage ? <Home /> : <Folder />}
                  {p.name}
                </Button>
              ))}
              <form
                className="flex items-center gap-1"
                onSubmit={(e) => {
                  e.preventDefault()
                  if (!newPage.trim()) return
                  const id = crypto.randomUUID()
                  setDraft(addPage(draft, id, newPage.trim()))
                  setPageId(id)
                  setNewPage("")
                }}
              >
                <Input
                  aria-label="New page name"
                  className="h-8 w-32"
                  placeholder="New folder"
                  value={newPage}
                  onChange={(e) => {
                    setNewPage(e.target.value)
                  }}
                />
                <Button
                  type="submit"
                  variant="ghost"
                  size="icon"
                  aria-label="Add page"
                  disabled={!newPage.trim()}
                >
                  <FolderPlus />
                </Button>
              </form>
            </div>
            {currentPage && currentPage.id !== draft.homePage && (
              <div className="flex flex-wrap items-center gap-2 text-sm">
                <Label htmlFor="page-name">Page name</Label>
                <Input
                  id="page-name"
                  className="h-8 w-40"
                  value={currentPage.name}
                  onChange={(e) => {
                    setDraft(renamePage(draft, currentPage.id, e.target.value))
                  }}
                />
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setDraft(deletePage(draft, currentPage.id))
                    setPageId(draft.homePage)
                  }}
                >
                  <Trash2 /> Delete page
                </Button>
                <span className="text-xs text-muted-foreground">
                  Add a “Back” binding so the folder can be left on the device.
                </span>
              </div>
            )}
          </CardHeader>
          <CardContent className="grid gap-4">
            {errors.length > 0 && <ErrorNotice error={errors.join(" ")} />}
            <div className="overflow-x-auto">
              <DeviceCanvas
                model={model}
                faces={faces}
                icons={icons}
                captions={captions}
                selected={selected}
                onSelect={setSelected}
                droppable
              />
            </div>
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <Label htmlFor="preview-project">Preview with project</Label>
              <Select
                value={previewProject ?? "none"}
                onValueChange={(value) => {
                  setPreviewProject(value === "none" ? null : value)
                }}
              >
                <SelectTrigger id="preview-project" className="h-8 w-40">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">No project</SelectItem>
                  {projects.map((p) => (
                    <SelectItem key={p.id} value={p.id}>
                      {p.key}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <span className="text-xs text-muted-foreground">
                Labels such as {"{project.key}"} follow the selected project.
              </span>
            </div>
          </CardContent>
        </Card>

        <Inspector
          model={model}
          draft={draft}
          pageId={pageId}
          control={selected}
          view={view}
          onChange={setDraft}
          onClose={() => {
            setSelected(null)
          }}
        />
      </div>
    </DndContext>
  )
}

function Inspector({
  model,
  draft,
  pageId,
  control,
  view,
  onChange,
  onClose,
}: {
  model: DeviceModel
  draft: DeviceLayout
  pageId: string
  control: ControlId | null
  view: ProfileView
  onChange: (layout: DeviceLayout) => void
  onClose: () => void
}) {
  if (!control) {
    return (
      <Card className="h-fit">
        <CardHeader>
          <CardTitle className="text-sm">Control</CardTitle>
          <CardDescription>Select a key, dial or touch zone to configure it.</CardDescription>
        </CardHeader>
      </Card>
    )
  }
  const bindings = bindingsFor(draft, pageId, control)
  const setTarget = (trigger: Trigger, value: string) => {
    const target = decodeTarget(value)
    if (!target) {
      onChange(removeBinding(draft, pageId, control, trigger))
      return
    }
    const existing = bindings.find((b) => sameTrigger(b.trigger, trigger))
    onChange(
      setBinding(draft, pageId, { control, trigger, target, label: existing?.label ?? null }),
    )
  }
  return (
    <Card className="h-fit">
      <CardHeader className="flex flex-row items-center gap-2">
        <CardTitle className="flex-1 text-sm">{controlLabel(control)}</CardTitle>
        <Button variant="ghost" size="icon" aria-label="Close" onClick={onClose}>
          <X />
        </Button>
      </CardHeader>
      <CardContent className="grid gap-4">
        {supportedTriggers(model, control).map((trigger) => {
          const binding = bindings.find((b) => sameTrigger(b.trigger, trigger))
          const id = `${controlKey(control)}-${triggerKey(trigger)}`
          return (
            <div key={triggerKey(trigger)} className="grid gap-2">
              <Label htmlFor={id}>{triggerLabel(trigger)}</Label>
              <Select
                value={encodeTarget(binding?.target)}
                onValueChange={(value) => {
                  setTarget(trigger, value)
                }}
              >
                <SelectTrigger id={id} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">Nothing</SelectItem>
                  <SelectGroup>
                    <SelectLabel>Actions</SelectLabel>
                    {view.profile.actions.map((a) => (
                      <SelectItem key={a.id} value={`action:${a.id}`}>
                        {a.name}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                  {view.profile.workflows.length > 0 && (
                    <SelectGroup>
                      <SelectLabel>Workflows</SelectLabel>
                      {view.profile.workflows.map((w) => (
                        <SelectItem key={w.id} value={`workflow:${w.id}`}>
                          {w.name}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  )}
                  <SelectGroup>
                    <SelectLabel>Navigation</SelectLabel>
                    <SelectItem value="back">Back</SelectItem>
                    {draft.pages
                      .filter((p) => p.id !== pageId)
                      .map((p) => (
                        <SelectItem key={p.id} value={`page:${p.id}`}>
                          Open {p.name}
                        </SelectItem>
                      ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {binding && (
                <Input
                  aria-label={`Label for ${triggerLabel(trigger)}`}
                  placeholder="Label (optional)"
                  maxLength={32}
                  value={binding.label ?? ""}
                  onChange={(e) => {
                    onChange(
                      setBinding(draft, pageId, { ...binding, label: e.target.value || null }),
                    )
                  }}
                />
              )}
            </div>
          )
        })}
        <p className="text-xs text-muted-foreground">
          Only events this model can produce are offered. Unbound controls stay blank on the device.
        </p>
      </CardContent>
    </Card>
  )
}
