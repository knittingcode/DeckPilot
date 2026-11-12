import { Copy, Info, Plus, Star, Trash2 } from "lucide-react"
import { useRef, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
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
import { LayoutEditor } from "@/layout-editor/editor"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { SUPPORT } from "@/lib/format"
import type { DeviceLayout, DeviceModel } from "@/lib/types"

export function LayoutDesignerPage() {
  return (
    <Page
      title="Layout Designer"
      description="Assign configured actions to device controls, with or without hardware"
      profileScoped
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
