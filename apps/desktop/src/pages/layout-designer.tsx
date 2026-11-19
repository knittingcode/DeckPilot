import { Copy, Info, Plus, Star, Trash2 } from "lucide-react"
import { type ReactNode, useEffect, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
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
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useAsync } from "@/hooks/use-async"
import { useTitlebar } from "@/hooks/use-titlebar"
import { LayoutEditor } from "@/layout-editor/editor"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { useFocusTarget } from "@/lib/focus"
import type { DeviceLayout, DeviceModel, Platform } from "@/lib/types"
import { LEVELS, PLATFORM_LABELS, currentPlatform, verificationOn } from "@/lib/verification"
import { useEditorSelection } from "@/shell/editor-selection-context"

export function LayoutDesignerPage() {
  useTitlebar({
    title: "Device Editor",
    subtitle: "Assign actions to the controls of the device chosen in the toolbar",
  })
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <RequireProfile>{(profileId) => <Designer profileId={profileId} />}</RequireProfile>
    </div>
  )
}

/**
 * The device editor. Profile and device come from the toolbar (the shell's
 * editor selection); the layout of the model is chosen here.
 */
function Designer({ profileId }: { profileId: string }) {
  const { revision, agent } = useAppContext()
  const selection = useEditorSelection()
  const view = useAsync(() => api.getProfile(profileId), `${profileId}:${revision}`)
  const models = selection.models
  // Opened from another page (e.g. the references of an action): that layout.
  const focus = useFocusTarget("layout")
  const { showLayout, adoptModel, setDirty } = selection
  useEffect(() => {
    if (focus) showLayout(focus.layout)
  }, [focus, showLayout])
  const ready = view.state.status === "ready" && models.status === "ready"
  const profile = view.state.status === "ready" ? view.state.data.profile : null
  const catalog = models.status === "ready" ? models.data : []
  const connected = (agent?.status?.devices ?? [])
    .filter((d) => d.state === "connected")
    .map((d) => d.descriptor?.modelId ?? d.id.model)
  const focused = profile?.layouts.find((l) => l.id === selection.layoutId)
  const model = profile
    ? (catalog.find((m) => m.id === focused?.model) ??
      catalog.find((m) => m.id === selection.modelId) ??
      // Nothing chosen yet: a connected device first.
      catalog.find((m) => connected.includes(m.id)) ??
      catalog.find((m) => profile.layouts.some((l) => l.model === m.id)) ??
      catalog[0])
    : undefined
  // The toolbar shows the model the editor settled on.
  useEffect(() => {
    if (model && model.id !== selection.modelId) adoptModel(model.id)
  }, [model, selection.modelId, adoptModel])
  // Nothing unsaved once the editor is gone.
  useEffect(
    () => () => {
      setDirty(false)
    },
    [setDirty],
  )

  if (!ready) {
    if (view.state.status === "error")
      return (
        <Padded>
          <ErrorNotice error={view.state.error} />
        </Padded>
      )
    if (models.status === "error")
      return (
        <Padded>
          <ErrorNotice error={models.error} />
        </Padded>
      )
    return (
      <Padded>
        <Loading />
      </Padded>
    )
  }
  if (!model || !profile || view.state.status !== "ready") {
    return (
      <Padded>
        <EmptyState title="No device models available" />
      </Padded>
    )
  }
  const layouts = profile.layouts.filter((l) => l.model === model.id)
  const activeForModel = profile.activeLayouts?.[model.id] ?? layouts[0]?.id ?? null
  const layout =
    layouts.find((l) => l.id === selection.layoutId) ??
    layouts.find((l) => l.id === activeForModel) ??
    layouts[0]

  const header = (
    <>
      <div className="flex flex-wrap items-end gap-4">
        <LayoutToolbar
          profileId={profileId}
          model={model}
          layouts={layouts}
          current={layout ?? null}
          activeForModel={activeForModel}
          onSelect={(id) => {
            selection.selectLayout(id)
          }}
          onChanged={(id) => {
            if (id) showLayout(id, model.id)
            view.reload()
          }}
        />
      </div>
      <VerificationNotice model={model} />
    </>
  )

  return layout ? (
    <LayoutEditor
      key={`${profileId}:${layout.id}`}
      profileId={profileId}
      view={view.state.data}
      model={model}
      saved={layout}
      header={header}
      onDirtyChange={setDirty}
      onSaved={() => {
        setDirty(false)
        view.reload()
      }}
    />
  ) : (
    <Padded>
      {header}
      <EmptyState title={`No layout for ${model.name} in this profile`}>
        Create one with “New layout”. Layouts can be designed without the device.
      </EmptyState>
    </Padded>
  )
}

function Padded({ children }: { children: ReactNode }) {
  return <div className="grid content-start gap-4 p-4">{children}</div>
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
  const run = (operation: () => Promise<string | undefined>, message: string, failed: string) => {
    operation().then(
      (id) => {
        toast.success(message)
        onChanged(id)
        refresh()
      },
      (e: unknown) => {
        toastFailure(failed, e)
      },
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
                  `“${current.name}” was not duplicated`,
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
                run(
                  async () => {
                    await api.activateLayout(profileId, current.id)
                    return current.id
                  },
                  "Layout activated",
                  `“${current.name}” was not made active`,
                )
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
                run(
                  async () => {
                    await api.deleteLayout(profileId, current.id)
                    return undefined
                  },
                  "Layout deleted",
                  `“${current.name}” was not deleted`,
                )
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
            `“${name}” was not created`,
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

/** Shown unless the model was tested on a physical device on this platform. */
export function VerificationNotice({
  model,
  platform = currentPlatform(),
}: {
  model: DeviceModel
  platform?: Platform
}) {
  const entry = verificationOn(model, platform)
  if (entry.level === "physicallyVerified") return null
  return (
    <Alert>
      <Info />
      <AlertTitle>
        {LEVELS[entry.level].label} on {PLATFORM_LABELS[platform]}
      </AlertTitle>
      <AlertDescription>{entry.evidence || LEVELS[entry.level].detail}</AlertDescription>
    </Alert>
  )
}
