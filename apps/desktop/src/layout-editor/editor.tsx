import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import { Redo2, RotateCcw, Save, Undo2 } from "lucide-react"
import { useEffect, useMemo, useState, type KeyboardEvent } from "react"
import { toast } from "sonner"

import { DeviceCanvas } from "@/components/device-canvas"
import { EmptyState, ErrorNotice } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { IS_MAC } from "@/hooks/use-navigation"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import {
  bindingsFor,
  controlKey,
  controlLabel,
  page as findPage,
  hasWayBack,
  targetLabel,
  triggerLabel,
} from "@/lib/layout-model"
import type { ControlId, DeviceLayout, DeviceModel, KeyFace, ProfileView } from "@/lib/types"

import { effectiveUsage } from "./appearance"
import { AppearancePanel, type IconState } from "./appearance-panel"
import { ControlMenu } from "./control-menu"
import {
  boundControls,
  folderOf,
  orderedControls,
  parseControlKey,
  type DragSource,
} from "./editing"
import { Inspector } from "./inspector"
import { LibraryPanel } from "./library-panel"
import { PageBar } from "./page-bar"
import { targetGroups } from "./targets"
import { useEditorShortcuts } from "@/hooks/use-editor-shortcuts"
import { shortcutLabel } from "@/lib/shortcuts"
import { useIcons } from "./use-icons"
import { useLayoutEditing } from "./use-layout-editing"

/**
 * Faces and key images exactly as the agent would render them (with
 * appearance and icons), debounced while editing.
 */
function usePreviewFaces(
  profileId: string,
  draft: DeviceLayout,
  pageId: string,
  project: string | null,
  /** Changes when the profile's icons change, so previews pick them up. */
  iconsVersion: string,
) {
  const [preview, setPreview] = useState({
    faces: new Map<string, KeyFace>(),
    images: new Map<string, string>(),
  })
  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(() => {
      api.previewLayout(profileId, draft, pageId, project, true).then(
        (result) => {
          if (cancelled) return
          setPreview({
            faces: new Map(result.map((r) => [controlKey(r.control), r.face])),
            images: new Map(
              result.flatMap((r) => (r.image ? [[controlKey(r.control), r.image] as const] : [])),
            ),
          })
        },
        () => undefined,
      )
    }, 150)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [profileId, draft, pageId, project, iconsVersion])
  return preview
}

export function LayoutEditor({
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
  const editing = useLayoutEditing(model, saved)
  const { draft, pageId, selected, selection, commands } = editing
  const [previewProject, setPreviewProject] = useState<string | null>(view.activeProjectId)
  const [errors, setErrors] = useState<string[]>([])
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved)
  const { actions, workflows, projects } = view.profile
  const library = useIcons(profileId)
  const { faces, images } = usePreviewFaces(
    profileId,
    draft,
    pageId,
    previewProject,
    library.icons.map((i) => i.id).join(","),
  )
  const iconState: IconState = {
    ...library,
    usage: (icon) => effectiveUsage(icon.usages, icon.id, saved, draft),
  }
  const groups = useMemo(
    () => targetGroups(view.profile, draft, pageId),
    [view.profile, draft, pageId],
  )
  const bound = boundControls(draft, pageId)
  const controls = orderedControls(model)

  useEffect(() => {
    onDirtyChange(dirty)
  }, [dirty, onDirtyChange])

  useEditorShortcuts({
    undo: commands.undo,
    redo: commands.redo,
    copy: commands.copy,
    cut: commands.cut,
    paste: commands.paste,
    duplicate: commands.duplicate,
    save: () => {
      if (dirty) void save()
    },
  })

  const pageBindings = useMemo(() => findPage(draft, pageId)?.bindings ?? [], [draft, pageId])
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
  const summaries = useMemo(() => {
    const render = (text: string) =>
      text
        .replaceAll("{profile}", view.profile.name)
        .replaceAll("{project.key}", previewed?.key ?? "-")
        .replaceAll("{project.name}", previewed?.name ?? "-")
    const map = new Map<string, string>()
    for (const control of orderedControls(model)) {
      const labels = bindingsFor(draft, pageId, control).map(
        (b) =>
          `${triggerLabel(b.trigger)}: ${render(b.label ?? targetLabel(b.target, actions, workflows, draft))}`,
      )
      if (labels.length > 0) map.set(controlKey(control), labels.join(" · "))
    }
    return map
  }, [model, draft, pageId, actions, workflows, previewed, view.profile.name])

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor),
  )
  const onDragEnd = (event: DragEndEvent) => {
    const source = event.active.data.current as DragSource | undefined
    const target = event.over ? parseControlKey(String(event.over.id)) : null
    if (source && target) commands.drop(source, target)
  }

  const onControlKeyDown = (event: KeyboardEvent<HTMLButtonElement>, control: ControlId) => {
    const mod = IS_MAC ? event.metaKey : event.ctrlKey
    if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault()
      if (selected.length > 0) commands.clear()
    } else if (event.key === "Escape") {
      commands.clearSelection()
    } else if (mod && event.key.toLowerCase() === "a") {
      event.preventDefault()
      commands.selectAll(controls)
    } else if (event.key === "Enter" && event.shiftKey) {
      event.preventDefault()
      commands.openFolder(control)
    }
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

  const folder = selection.focus ? folderOf(draft, pageId, selection.focus) : undefined
  const isFolderPage = pageId !== draft.homePage
  const hasControls = controls.length > 0

  return (
    <DndContext sensors={sensors} onDragEnd={onDragEnd}>
      <div className="grid gap-4 xl:grid-cols-[14rem_1fr] 2xl:grid-cols-[16rem_1fr_20rem]">
        <LibraryPanel groups={groups} />

        <Card>
          <CardHeader className="gap-3">
            <div className="flex flex-wrap items-center gap-2">
              <Input
                aria-label="Layout name"
                className="h-8 w-56 font-medium"
                value={draft.name}
                onChange={(e) => {
                  commands.change({ ...draft, name: e.target.value }, "layout-name")
                }}
              />
              {dirty && <Badge variant="outline">Unsaved</Badge>}
              <div className="flex-1" />
              <Button
                variant="ghost"
                size="icon"
                aria-label="Undo"
                title={`Undo (${shortcutLabel("Z")})`}
                disabled={!editing.canUndo}
                onClick={commands.undo}
              >
                <Undo2 />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                aria-label="Redo"
                title={`Redo (${shortcutLabel("Z", true)})`}
                disabled={!editing.canRedo}
                onClick={commands.redo}
              >
                <Redo2 />
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={!dirty}
                onClick={() => {
                  commands.revert()
                  setErrors([])
                }}
              >
                <RotateCcw /> Revert
              </Button>
              <Button size="sm" disabled={!dirty} onClick={() => void save()}>
                <Save /> Save
              </Button>
            </div>
            <PageBar
              draft={draft}
              pageId={pageId}
              trail={editing.trail}
              onOpen={commands.openPage}
              onBack={commands.back}
              onAdd={commands.addPage}
              onRename={commands.renamePage}
              onDelete={commands.deletePage}
            />
          </CardHeader>
          <CardContent className="grid gap-4">
            {errors.length > 0 && <ErrorNotice error={errors.join(" ")} />}
            {hasControls ? (
              <div className="overflow-x-auto p-1">
                <DeviceCanvas
                  model={model}
                  images={images}
                  faces={faces}
                  icons={icons}
                  captions={summaries}
                  summaries={summaries}
                  selection={selection.controls}
                  focused={selection.focus}
                  onSelect={commands.select}
                  onControlKeyDown={onControlKeyDown}
                  droppable
                  draggable
                  wrapControl={(control, element) => (
                    <ControlMenu
                      label={controlLabel(control)}
                      groups={groups}
                      state={{
                        count: selected.length,
                        hasBindings: selected.some((c) => bound.has(controlKey(c))),
                        canPaste: editing.clipboard !== null,
                        folderName: (() => {
                          const id = folderOf(draft, pageId, control)
                          return id ? findPage(draft, id)?.name : undefined
                        })(),
                      }}
                      commands={{
                        onOpen: () => {
                          commands.selectForMenu(control)
                        },
                        assign: commands.assign,
                        copy: commands.copy,
                        cut: commands.cut,
                        paste: commands.paste,
                        duplicate: commands.duplicate,
                        clear: commands.clear,
                        openFolder: () => {
                          commands.openFolder(control)
                        },
                      }}
                    >
                      {element}
                    </ControlMenu>
                  )}
                />
              </div>
            ) : (
              <EmptyState title={`${model.name} has no configurable controls`} />
            )}
            {hasControls && bound.size === 0 && (
              <EmptyState title={isFolderPage ? "This folder is empty" : "This page is empty"}>
                Drag an action or workflow from the library onto a control, or select a control and
                choose in the inspector.
              </EmptyState>
            )}
            {isFolderPage && !hasWayBack(draft, pageId) && (
              <p className="text-xs text-muted-foreground" role="note">
                This folder has no “Back” binding. On the device, DeckPilot adds one on its first
                free key. To choose the key, drag “Back” from the library onto a control.
              </p>
            )}
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
          selection={selected}
          focus={selection.focus}
          groups={groups}
          folder={folder}
          onChange={commands.change}
          onClear={commands.clear}
          onOpenFolder={() => {
            if (selection.focus) commands.openFolder(selection.focus)
          }}
          onClose={commands.clearSelection}
          appearance={
            selected.length > 0 ? (
              <AppearancePanel
                draft={draft}
                pageId={pageId}
                controls={selected}
                images={images}
                icons={iconState}
                onChange={commands.restyle}
              />
            ) : undefined
          }
        />
      </div>
    </DndContext>
  )
}
