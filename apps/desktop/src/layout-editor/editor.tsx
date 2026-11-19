import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  defaultAnnouncements,
  useSensor,
  useSensors,
  type Announcements,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core"
import { Redo2, RotateCcw, Save, Undo2 } from "lucide-react"
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react"
import { toast } from "sonner"

import { DeviceCanvas } from "@/components/device-canvas"
import { deviceGeometry, KEY_SIZE, keySize } from "@/components/device-geometry"
import { EmptyState, ErrorNotice } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
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
import { IS_MAC } from "@/hooks/use-navigation"
import { api, errorMessage } from "@/lib/api"
import { appliedByAgent, toastSaved } from "@/lib/agent-reload"
import { useActionModules, useAppContext } from "@/lib/app-context"
import {
  controlCapabilities,
  describeIncompatibility,
  whyNotForTrigger,
  whyNotOnControl,
} from "@/lib/compatibility"
import { pluginActionUnavailable, withPluginTypes } from "@/lib/plugin-actions"
import {
  bindingsFor,
  controlKey,
  controlLabel,
  page as findPage,
  hasWayBack,
  pagePosition,
  targetLabel,
  triggerLabel,
} from "@/lib/layout-model"
import { DEFAULT_TIMINGS, readableTriggers } from "@/components/gesture-editor/model"
import { isStepWorkflow, newMultiAction } from "@/lib/multi-action"
import {
  SWITCH_LAYOUT,
  backUnavailable,
  layoutSwitchParameters,
  switchableLayouts,
} from "@/lib/layout-navigation"
import { StepEditor } from "@/pages/workflow-steps"
import type {
  ActionInstance,
  ActionType,
  BindingTarget,
  ControlId,
  DeviceInfo,
  DeviceLayout,
  DeviceModel,
  GestureTimings,
  ProfileView,
  Trigger,
  Workflow,
} from "@/lib/types"

import { effectiveUsage } from "./appearance"
import { AppearancePanel, type IconState } from "./appearance-panel"
import {
  catalogGroups,
  existingDefault,
  existingPreset,
  newAction,
  newPresetAction,
} from "./catalog"
import { ControlMenu } from "./control-menu"
import { DeletePageDialog } from "./delete-page-dialog"
import {
  boundControls,
  folderOf,
  isFree,
  orderedControls,
  pageDeletion,
  parseControlKey,
  type ActionPreset,
  type DragSource,
} from "./editing"
import { Inspector } from "./inspector"
import { LibraryDock } from "./library-dock"
import { LibraryPanel, type PresetEntry } from "./library-panel"
import { dragLook } from "./drag-look"
import { DragPreview } from "./drag-preview"
import { PageNav } from "./page-nav"
import { PageSettings, PagesSheet } from "./page-settings"
import { PageTreePanel } from "./page-tree-panel"
import { markIncompatible, targetGroups } from "./targets"
import { useEditorShortcuts } from "@/hooks/use-editor-shortcuts"
import { shortcutLabel } from "@/lib/shortcuts"
import { useElementSize } from "./use-element-size"
import { useIcons } from "./use-icons"
import { useLayoutEditing } from "./use-layout-editing"
import { LiveValuesNote } from "./live-values-note"
import { useLiveKeyStates } from "./use-live-key-states"
import { usePreviewFaces } from "./use-preview-faces"

/**
 * Action types and default gesture timings for the gesture editor. In
 * builds with plugins the plugin types are the ones the app read from the
 * agent (`useActionModules`), available now, rather than whatever the
 * backend's list held when the editor opened.
 */
function useGestureContext() {
  const modules = useActionModules()
  const [listed, setTypes] = useState<ActionType[]>([])
  const types = useMemo(() => withPluginTypes(listed, modules), [listed, modules])
  const [defaults, setDefaults] = useState<GestureTimings>(DEFAULT_TIMINGS)
  useEffect(() => {
    let cancelled = false
    api.listActionTypes().then(
      (result) => {
        if (!cancelled) setTypes(result)
      },
      () => undefined,
    )
    api.getGestureDefaults().then(
      (result) => {
        if (!cancelled) setDefaults(result)
      },
      () => undefined,
    )
    return () => {
      cancelled = true
    }
  }, [])
  return { types, defaults }
}

export function LayoutEditor({
  profileId,
  view,
  model,
  saved,
  header,
  device = null,
  onDirtyChange,
  onSaved,
}: {
  profileId: string
  view: ProfileView
  model: DeviceModel
  saved: DeviceLayout
  /** Shown above the device, e.g. the layout chooser. */
  header?: ReactNode
  /**
   * The connected device the editor is for, as the agent reports it: the
   * layout it shows and whether Back to previous layout can go back.
   */
  device?: DeviceInfo | null
  onDirtyChange: (dirty: boolean) => void
  onSaved: () => void
}) {
  const { profiles, refresh, agent } = useAppContext()
  // Actions created by dropping a type from the catalog, until the
  // reloaded profile has them: they are named, drawn and reused at once.
  const [created, setCreated] = useState<ActionInstance[]>([])
  // Actions changed here (a Switch layout key's destination), until the
  // profile is reloaded: changes made against the profile as it is now.
  const [changed, setChanged] = useState<{ base: ActionInstance[]; actions: ActionInstance[] }>({
    base: [],
    actions: [],
  })
  const actions = useMemo(() => {
    const updates = changed.base === view.profile.actions ? changed.actions : []
    return [
      ...view.profile.actions.map((a) => updates.find((c) => c.id === a.id) ?? a),
      ...created.filter((c) => !view.profile.actions.some((a) => a.id === c.id)),
    ]
  }, [view.profile.actions, created, changed])
  const remember = (action: ActionInstance) => {
    if (view.profile.actions.some((a) => a.id === action.id)) {
      setChanged((current) => ({
        base: view.profile.actions,
        actions: [
          ...(current.base === view.profile.actions ? current.actions : []).filter(
            (c) => c.id !== action.id,
          ),
          action,
        ],
      }))
    } else {
      setCreated((current) => [...current.filter((c) => c.id !== action.id), action])
    }
  }
  const gestureContext = useGestureContext()
  const modules = useActionModules()
  const catalog = useMemo(() => catalogGroups(modules), [modules])
  // Every action type the editor knows, for compatibility
  // (docs/device-controls.md): the backend's list with the plugins', and
  // the catalog's.
  const typeById = useMemo(() => {
    const map = new Map<string, ActionType>()
    for (const module of modules) for (const type of module.actionTypes) map.set(type.id, type)
    for (const group of catalog)
      for (const entry of group.entries) map.set(entry.type.id, entry.type)
    for (const type of gestureContext.types) map.set(type.id, type)
    return map
  }, [modules, catalog, gestureContext.types])
  // The profile's other layouts for this model, as "Switch to …" entries.
  const switchType = typeById.get(SWITCH_LAYOUT)
  const presets = useMemo((): PresetEntry[] => {
    if (!switchType) return []
    return switchableLayouts(view.profile.layouts, model.id)
      .filter((l) => l.id !== saved.id)
      .map((l) => ({
        key: `switch-layout:${l.id}`,
        label: `Switch to ${l.name}`,
        description: `Shows ${l.name} on this device; Back to previous layout returns here.`,
        category: switchType.category,
        type: switchType,
        preset: {
          name: l.name,
          parameters: layoutSwitchParameters({ layout: l.id, page: null, history: "push" }),
        },
      }))
  }, [switchType, view.profile.layouts, model.id, saved.id])
  const typeOfTarget = (target: BindingTarget): ActionType | undefined => {
    if (target.type !== "action") return undefined
    const action = actions.find((a) => a.id === target.action)
    return action ? typeById.get(action.actionType) : undefined
  }
  /** Why `target` cannot run from `control` for `trigger`; `null` when it can. */
  const incompatibleFor = (control: ControlId, target: BindingTarget, trigger: Trigger) => {
    const type = typeOfTarget(target)
    return type ? whyNotForTrigger(type, model, control, trigger) : null
  }
  const editing = useLayoutEditing(
    model,
    saved,
    (control, target, trigger) => incompatibleFor(control, target, trigger) === null,
  )
  const { draft, pageId, selected, selection, commands } = editing
  const [previewProject, setPreviewProject] = useState<string | null>(view.activeProjectId)
  const [errors, setErrors] = useState<string[]>([])
  const dirty = JSON.stringify(draft) !== JSON.stringify(editing.baseline)
  const { projects } = view.profile
  // Multi Actions created here, likewise until the reloaded profile has them.
  const [createdWorkflows, setCreatedWorkflows] = useState<Workflow[]>([])
  const workflows = useMemo(
    () => [
      ...view.profile.workflows,
      ...createdWorkflows.filter((c) => !view.profile.workflows.some((w) => w.id === c.id)),
    ],
    [view.profile.workflows, createdWorkflows],
  )
  // The Multi Action whose steps are open.
  const [stepsOf, setStepsOf] = useState<string | null>(null)
  const openSteps = workflows.find((w) => w.id === stepsOf)
  const profile = useMemo(() => ({ actions, workflows, projects }), [actions, workflows, projects])
  const library = useIcons(profileId)
  // The page's actions whose keys may show live values: the agent reports
  // their states, the same ones the device's keys show.
  const pageActions = useMemo(
    () =>
      (findPage(draft, pageId)?.bindings ?? []).flatMap((b) =>
        b.target.type === "action" ? [b.target.action] : [],
      ),
    [draft, pageId],
  )
  const liveStates = useLiveKeyStates(profileId, pageActions, agent)
  const { faces, images, incompatible } = usePreviewFaces(
    profileId,
    draft,
    pageId,
    previewProject,
    library.icons.map((i) => i.id).join(","),
    liveStates.live,
  )
  const iconState: IconState = {
    ...library,
    usage: (icon) => effectiveUsage(icon.usages, icon.id, editing.baseline, draft),
  }
  const groups = useMemo(
    () => targetGroups(profile, draft, pageId, (type) => pluginActionUnavailable(modules, type)),
    [profile, draft, pageId, modules],
  )
  const bound = boundControls(draft, pageId)
  const controls = orderedControls(model)

  /** The targets, those that cannot go on `control` marked with the reason. */
  const groupsFor = (control: ControlId) =>
    markIncompatible(groups, (typeId) => {
      const type = typeById.get(typeId)
      return type ? whyNotOnControl(type, model, control) : null
    })
  // The library is for the selected control, when exactly one is.
  const libraryControl = selected.length === 1 ? (selected[0] ?? null) : null

  /**
   * Why what is dragged cannot be dropped on `control`, in words; `null`
   * when it can. An action that cannot run from the control (any of its
   * triggers, or the ones a moved control's bindings use) is refused.
   */
  const dropRejection = (control: ControlId, dragged: unknown): string | null => {
    const source = dragged as DragSource | undefined
    if (!source) return null
    if (source.type === "actionType") {
      const type = typeById.get(source.actionType)
      return type ? whyNotOnControl(type, model, control) : null
    }
    if (source.type === "target") {
      const type = typeOfTarget(source.target)
      return type ? whyNotOnControl(type, model, control) : null
    }
    if (source.type === "control") {
      const moves = [
        ...bindingsFor(draft, pageId, source.control).map((b) => [b, control] as const),
        ...bindingsFor(draft, pageId, control).map((b) => [b, source.control] as const),
      ]
      for (const [binding, to] of moves) {
        const why = incompatibleFor(to, binding.target, binding.trigger)
        if (why) return why
      }
    }
    return null
  }
  /** What is dragged, as the announcement names it. */
  const draggedName = (dragged: unknown): string => {
    const source = dragged as DragSource | undefined
    if (source?.type === "actionType")
      return source.preset?.name ?? typeById.get(source.actionType)?.name ?? "The action"
    if (source?.type === "target") {
      return targetLabel(source.target, actions, workflows, draft)
    }
    if (source?.type === "control") return controlLabel(source.control)
    return "This"
  }
  const refusal = (control: ControlId, dragged: unknown): string | null => {
    const why = dropRejection(control, dragged)
    return why ? `${draggedName(dragged)} cannot go on ${controlLabel(control)}. ${why}` : null
  }
  // The control that just refused a drop and the announcement, for a moment.
  const [rejected, setRejected] = useState<{ control: ControlId; message: string } | null>(null)
  const rejectedTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(
    () => () => {
      clearTimeout(rejectedTimer.current)
    },
    [],
  )
  const reject = (control: ControlId, dragged: unknown, why: string) => {
    const message = `${draggedName(dragged)} cannot go on ${controlLabel(control)}. ${why}`
    toast.error(`${draggedName(dragged)} was not put on ${controlLabel(control)}`, {
      description: why,
    })
    setRejected({ control, message })
    clearTimeout(rejectedTimer.current)
    rejectedTimer.current = setTimeout(() => {
      setRejected(null)
    }, 2500)
  }
  const announcements: Announcements = {
    ...defaultAnnouncements,
    onDragOver(event) {
      const control = event.over ? parseControlKey(String(event.over.id)) : null
      const why = control ? refusal(control, event.active.data.current) : null
      return why ?? defaultAnnouncements.onDragOver(event)
    },
    onDragEnd(event) {
      const control = event.over ? parseControlKey(String(event.over.id)) : null
      const why = control ? refusal(control, event.active.data.current) : null
      return why ? `Not placed. ${why}` : defaultAnnouncements.onDragEnd(event)
    },
  }

  /**
   * The action a dropped type stands for: the profile's action that is
   * exactly a new one of the type, else a new action with the type's
   * default settings, saved to the profile at once (a key can only run a
   * saved action). What it is put on is part of the draft as usual.
   */
  const actionOfType = async (
    typeId: string,
    preset?: ActionPreset,
  ): Promise<BindingTarget | null> => {
    const type = preset
      ? typeById.get(typeId)
      : catalog.flatMap((g) => g.entries).find((e) => e.type.id === typeId)?.type
    if (!type) return null
    try {
      let action = preset ? existingPreset(actions, type, preset) : existingDefault(actions, type)
      if (!action) {
        action = await api.createAction(
          profileId,
          preset ? newPresetAction(actions, type, preset) : newAction(actions, type),
        )
        const made = action
        setCreated((current) => [...current, made])
        refresh()
      }
      return { type: "action", action: action.id }
    } catch (e) {
      toast.error(`“${preset?.name ?? type.name}” could not be added`, {
        description: errorMessage(e),
      })
      return null
    }
  }

  const placeType = async (typeId: string, control: ControlId, preset?: ActionPreset) => {
    const target = await actionOfType(typeId, preset)
    if (target) commands.drop({ type: "target", target }, control)
  }

  /**
   * A new, empty Multi Action: a step workflow saved to the profile at
   * once (a key can only run a saved workflow), whose steps then open.
   */
  const multiAction = async (): Promise<BindingTarget | null> => {
    try {
      const fresh = newMultiAction(workflows)
      const saved = await api.saveWorkflow(profileId, fresh)
      const workflow: Workflow = { ...fresh, id: saved.id }
      setCreatedWorkflows((current) => [...current, workflow])
      refresh()
      setStepsOf(workflow.id)
      return { type: "workflow", workflow: workflow.id }
    } catch (e) {
      toast.error("The Multi Action could not be created", { description: errorMessage(e) })
      return null
    }
  }

  const placeMultiAction = async (control: ControlId) => {
    const target = await multiAction()
    if (target) commands.drop({ type: "target", target }, control)
  }

  /**
   * What a gesture's slot takes from the library: an action, a workflow
   * or a page as it is, an action type as its action. Keys and "New
   * folder" belong on keys.
   */
  const resolveSlotDrop = async (dragged: unknown): Promise<BindingTarget | null> => {
    const source = dragged as DragSource | undefined
    if (source?.type === "actionType") return actionOfType(source.actionType, source.preset)
    if (source?.type === "newMultiAction") return multiAction()
    if (source?.type === "target") {
      const opensItself = source.target.type === "openPage" && source.target.page === pageId
      if (!opensItself) return source.target
    }
    toast.error("A gesture takes an action, a workflow or another page", {
      description: "Drag one from the library onto the gesture.",
    })
    return null
  }

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
  // Back to previous layout keys while the device shows this layout of
  // this (active) profile with nothing to go back to: unavailable, as there.
  const noWayBack = useMemo(
    () =>
      new Map(
        [
          ...backUnavailable(
            pageBindings,
            actions,
            profiles.activeProfileId === profileId ? device : null,
            draft.id,
          ),
        ].map((key) => [key, "No previous layout"]),
      ),
    [pageBindings, actions, profiles.activeProfileId, profileId, device, draft.id],
  )
  const icons = useMemo(() => {
    const map = new Map<string, string | null | undefined>()
    for (const binding of pageBindings) {
      if (binding.target.type === "action") {
        const id = binding.target.action
        const key = controlKey(binding.control)
        if (!map.has(key) || binding.trigger.type === "press" || binding.trigger.type === "click") {
          map.set(key, actions.find((a) => a.id === id)?.icon)
        }
      }
    }
    return map
  }, [pageBindings, actions])
  const previewed = projects.find((p) => p.id === previewProject)
  const summaries = useMemo(() => {
    const position = pagePosition(draft, pageId)
    const render = (text: string) =>
      text
        .replaceAll("{profile}", view.profile.name)
        .replaceAll("{project.key}", previewed?.key ?? "-")
        .replaceAll("{project.name}", previewed?.name ?? "-")
        .replaceAll("{page.number}", String(position.number))
        .replaceAll("{page.count}", String(position.count))
        .replaceAll("{page.name}", findPage(draft, pageId)?.name ?? "-")
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
  // What is being dragged, for the drag preview.
  const [dragging, setDragging] = useState<DragSource | null>(null)
  const onDragStart = (event: DragStartEvent) => {
    setDragging((event.active.data.current as DragSource | undefined) ?? null)
  }
  const onDragEnd = (event: DragEndEvent) => {
    setDragging(null)
    const source = event.active.data.current as DragSource | undefined
    const target = event.over ? parseControlKey(String(event.over.id)) : null
    if (!source || !target) return
    const why = dropRejection(target, source)
    if (why) {
      reject(target, source, why)
      return
    }
    if (source.type === "actionType") void placeType(source.actionType, target, source.preset)
    else if (source.type === "newMultiAction") void placeMultiAction(target)
    else commands.drop(source, target)
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
      const saved = await api.saveLayout(profileId, draft)
      toastSaved(
        profiles.activeProfileId === profileId && appliedByAgent(saved.agentReload)
          ? "Layout saved and sent to the agent"
          : "Layout saved",
        saved.agentReload,
      )
      const kept = saved.incompatible ?? []
      if (kept.length > 0) {
        toast.warning(
          kept.length === 1
            ? "1 binding is kept but not run"
            : `${kept.length} bindings are kept but not run`,
          {
            description: `Their action cannot run from their control: ${kept.map(readableTriggers).join("; ")}`,
          },
        )
      }
      onSaved()
      refresh()
    } catch (e) {
      setErrors(errorMessage(e).split("\n"))
    }
  }

  // The preview fills the width of its workspace and at most about half
  // the editor's height, so the inspector stays in view below it.
  const workspace = useRef<HTMLDivElement>(null)
  const main = useRef<HTMLDivElement>(null)
  const space = useElementSize(workspace)
  const editorSpace = useElementSize(main)
  const unit = space
    ? keySize(
        {
          // Less the padding around the device.
          width: space.width - 16,
          height: editorSpace ? editorSpace.height * 0.55 : undefined,
        },
        deviceGeometry(model),
      )
    : KEY_SIZE.fallback
  const [managingPages, setManagingPages] = useState(false)

  const folder = selection.focus ? folderOf(draft, pageId, selection.focus) : undefined
  const isFolderPage = pageId !== draft.homePage
  const hasControls = controls.length > 0

  return (
    <DndContext
      sensors={sensors}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragCancel={() => {
        setDragging(null)
      }}
      accessibility={{ announcements }}
    >
      {/*
        One drag and drop context for the whole editor: the library (right)
        drops onto the device (centre) and onto the gestures of the
        inspector (below the device).
      */}
      <div data-slot="device-editor" className="relative flex min-h-0 flex-1">
        <div
          ref={main}
          data-slot="device-editor-main"
          className="grid min-w-0 flex-1 content-start gap-4 overflow-y-auto p-4"
        >
          {header}
          <Card className="min-w-0">
            <CardHeader className="gap-3 border-b [.border-b]:pb-3">
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
            </CardHeader>
            <CardContent className="grid gap-4">
              {errors.length > 0 && <ErrorNotice error={errors.map(readableTriggers).join(" ")} />}
              <p
                className="sr-only"
                role="status"
                aria-live="assertive"
                data-slot="drop-announcement"
              >
                {rejected?.message ?? ""}
              </p>
              {hasControls ? (
                <div
                  ref={workspace}
                  data-slot="device-workspace"
                  className="grid min-w-0 justify-items-center gap-3"
                >
                  {/* Scrolls sideways only where even the smallest keys do not fit. */}
                  <div className="flex max-w-full justify-center-safe overflow-x-auto p-2">
                    <DeviceCanvas
                      model={model}
                      keySize={unit}
                      images={images}
                      faces={faces}
                      unavailable={noWayBack}
                      icons={icons}
                      captions={summaries}
                      summaries={summaries}
                      selection={selection.controls}
                      focused={selection.focus}
                      onSelect={commands.select}
                      onOpen={commands.openFolder}
                      onControlKeyDown={onControlKeyDown}
                      droppable
                      draggable
                      dropRejection={dropRejection}
                      rejected={rejected?.control ?? null}
                      wrapControl={(control, element) => (
                        <ControlMenu
                          label={controlLabel(control)}
                          groups={groupsFor(control)}
                          state={{
                            count: selected.length,
                            hasBindings: selected.some((c) => bound.has(controlKey(c))),
                            canPaste: editing.clipboard !== null,
                            folderName: (() => {
                              const id = folderOf(draft, pageId, control)
                              return id ? findPage(draft, id)?.name : undefined
                            })(),
                            canCreateFolder: isFree(draft, pageId, control),
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
                            newFolder: () => {
                              commands.newFolder(control)
                            },
                          }}
                        >
                          {element}
                        </ControlMenu>
                      )}
                    />
                  </div>
                  <PageNav
                    draft={draft}
                    pageId={pageId}
                    trail={editing.trail}
                    onOpen={commands.openPage}
                    onBack={commands.back}
                    onAdd={commands.addPage}
                    onManage={() => {
                      setManagingPages(true)
                    }}
                    status={<LiveValuesNote status={liveStates.status} live={liveStates.live} />}
                  />
                </div>
              ) : (
                <EmptyState title={`${model.name} has no configurable controls`} />
              )}
              {hasControls && bound.size === 0 && (
                <EmptyState title={isFolderPage ? "This folder is empty" : "This page is empty"}>
                  Drag an action, a workflow or “New folder” from the library onto a control, or
                  select a control and choose in the inspector.
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
            profile={profile}
            types={gestureContext.types}
            gestureDefaults={gestureContext.defaults}
            issues={errors}
            onChange={commands.change}
            onClear={commands.clear}
            onOpenFolder={() => {
              if (selection.focus) commands.openFolder(selection.focus)
            }}
            onClose={commands.clearSelection}
            resolveDrop={resolveSlotDrop}
            onEditWorkflow={setStepsOf}
            incompatible={incompatibleFor}
            layoutNavigation={{
              profileId,
              modelName: model.name,
              layouts: switchableLayouts(view.profile.layouts, model.id),
              defaultLayout:
                view.profile.activeLayouts?.[model.id] ??
                switchableLayouts(view.profile.layouts, model.id)[0]?.id ??
                null,
              usages: view.actionUsages,
              device,
              onActionSaved: (action) => {
                remember(action)
                refresh()
              },
            }}
            notRun={
              selection.focus
                ? (incompatible.get(controlKey(selection.focus)) ?? []).map((found) => ({
                    trigger: found.trigger,
                    reason: describeIncompatibility(
                      found.reason,
                      model,
                      selection.focus ? controlCapabilities(model, selection.focus) : null,
                    ),
                  }))
                : []
            }
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
        <LibraryDock>
          <LibraryPanel
            groups={libraryControl ? groupsFor(libraryControl) : groups}
            catalog={catalog}
            presets={presets}
            multiAction
            variant="plain"
            compatibility={{
              control: libraryControl ? controlLabel(libraryControl) : null,
              catalog: (type) =>
                libraryControl ? whyNotOnControl(type, model, libraryControl) : null,
            }}
          />
        </LibraryDock>
      </div>
      <DragOverlay dropAnimation={null}>
        {dragging ? (
          <DragPreview
            look={dragLook(dragging, {
              actions,
              workflows,
              types: typeById,
              pageName: (id) => findPage(draft, id)?.name,
              images,
              faces,
              icons,
            })}
          />
        ) : null}
      </DragOverlay>
      <PagesSheet open={managingPages} onOpenChange={setManagingPages}>
        <PageSettings
          draft={draft}
          pageId={pageId}
          onRename={commands.renamePage}
          onDelete={() => {
            setManagingPages(false)
            commands.deletePage()
          }}
          onAutoReturn={commands.setAutoReturn}
        />
        <PageTreePanel
          profileId={profileId}
          draft={draft}
          layouts={view.profile.layouts}
          dirty={dirty}
          pageId={pageId}
          onOpen={commands.openPage}
          onStored={(layout, page) => {
            if (layout) commands.reset(layout, page)
            onSaved()
            refresh()
          }}
        />
      </PagesSheet>
      <DeletePageDialog
        deletion={editing.deleting ? pageDeletion(draft, editing.deleting) : null}
        onCancel={commands.cancelDeletePage}
        onConfirm={commands.confirmDeletePage}
      />
      <Dialog
        open={isStepWorkflow(openSteps)}
        onOpenChange={(open) => {
          if (!open) setStepsOf(null)
        }}
      >
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>Multi Action</DialogTitle>
            <DialogDescription>
              The steps run in order in the agent when the key is pressed. A delay waits between two
              steps; a running Multi Action can be cancelled from the status bar.
            </DialogDescription>
          </DialogHeader>
          {isStepWorkflow(openSteps) && (
            <StepEditor
              key={openSteps.id}
              view={{ ...view, profile: { ...view.profile, actions, workflows } }}
              types={gestureContext.types}
              profileId={profileId}
              initial={openSteps}
              isNew={false}
              embedded
              onChanged={() => {
                refresh()
                setStepsOf(null)
              }}
            />
          )}
        </DialogContent>
      </Dialog>
    </DndContext>
  )
}
