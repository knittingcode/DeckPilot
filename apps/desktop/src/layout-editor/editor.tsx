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
import { ListOrdered, Redo2, RotateCcw, Save, Undo2 } from "lucide-react"
import {
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react"
import { toast } from "sonner"

import { DeviceCanvas } from "@/components/device-canvas"
import { ModalShell } from "@/components/dialogs"
import { DialogsContext } from "@/components/dialogs/use-dialogs"
import { KEY_SIZE, keyBorderPixels } from "@/components/device-geometry"
import { EmptyState, ErrorNotice } from "@/components/page"
import { Button } from "@/components/ui/button"
import { displayPixels, useDevicePixelRatio } from "@/hooks/use-device-pixel-ratio"
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
import { WORKFLOWS_PLUGIN, useIsPluginActive } from "@/lib/plugin-features"
import { isSystemBack, systemBackControl } from "@/lib/folder-back"
import { isFolderControl } from "@/lib/folder-control"
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
import { useOptionalEditorSelection } from "@/shell/editor-selection-context"
import { type TranslationKey, useTranslation } from "@/i18n"
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

import { defaultAppearanceOf, effectiveUsage, shownBinding } from "./appearance"
import type { IconState } from "./icon-library"
import { type KeyLook, KeyLookContext } from "./key-look-context"
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
  opensFolderBesides,
  orderedControls,
  pageDeletion,
  parseControlKey,
  type ActionPreset,
  type DragSource,
} from "./editing"
import { Inspector, InspectorDeselect } from "./inspector"
import { LibraryDock } from "./library-dock"
import { LibraryPanel, type PresetEntry } from "./library-panel"
import { dragLook } from "./drag-look"
import { DragPreview } from "./drag-preview"
import { PageNavigationDock } from "./page-nav"
import { PageNavigator } from "./page-navigator"
import { newPageName } from "./page-navigation"
import { markIncompatible, targetGroups } from "./targets"
import { useEditorShortcuts } from "@/hooks/use-editor-shortcuts"
import { shortcutLabel } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { useIcons } from "./use-icons"
import { useVisualLibrary } from "./use-visual-library"
import { useLayoutEditing } from "./use-layout-editing"
import { LiveValuesNote } from "./live-values-note"
import { useLiveKeyStates } from "./use-live-key-states"
import { usePreviewFaces } from "./use-preview-faces"
import { useMarqueeFrames } from "./use-marquee-frames"
import { previewPixels } from "./canvas-viewport"
import { DeviceViewport } from "./device-viewport"
import {
  adjacentPage,
  canGoBack,
  controlMove,
  peekLayoutHistory,
  popLayoutHistory,
  pushLayoutHistory,
  type EditorMove,
} from "./preview-navigation"
import { useDeviceSimulation } from "./use-device-simulation"
import { type LiveStatus, useLivePreview } from "./use-live-preview"

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
  notices,
  device = null,
  onDirtyChange,
  onSaved,
}: {
  profileId: string
  view: ProfileView
  model: DeviceModel
  saved: DeviceLayout
  /** Notes about the layout, shown at the top of the workspace (e.g. it is shared). */
  notices?: ReactNode
  /**
   * The connected device the editor is for, as the agent reports it: the
   * layout it shows and whether Back to previous layout can go back.
   */
  device?: DeviceInfo | null
  onDirtyChange: (dirty: boolean) => void
  onSaved: () => void
}) {
  const { t, i18n: translations } = useTranslation()
  const language = translations.language
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
        label: t("editor:shell.switchPreset.label", { layout: l.name }),
        description: t("editor:shell.switchPreset.description", { layout: l.name }),
        category: switchType.category,
        type: switchType,
        preset: {
          name: l.name,
          parameters: layoutSwitchParameters({ layout: l.id, page: null, history: "push" }),
        },
      }))
  }, [switchType, view.profile.layouts, model.id, saved.id, t])
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
  // Where the editor was in this layout, when it comes back from another
  // view (the shell keeps it; nothing outside the shell).
  const shell = useOptionalEditorSelection()
  const placeKey = `${profileId}:${saved.id}`
  const [initialPlace] = useState(() => shell?.place(placeKey))
  const editing = useLayoutEditing(
    model,
    saved,
    (control, target, trigger) => incompatibleFor(control, target, trigger) === null,
    initialPlace,
  )
  const { draft, pageId, selected, editable, selection, commands } = editing
  const rememberPlace = shell?.rememberPlace
  const trail = editing.trail
  useEffect(() => {
    rememberPlace?.(placeKey, { pageId, trail, selection })
  }, [rememberPlace, placeKey, pageId, trail, selection])
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
  // Read again when a plugin changed (installed, enabled, disabled,
  // removed): its icons and backgrounds come and go with it.
  const visualLibrary = useVisualLibrary(
    modules
      .filter((m) => m.id.includes("."))
      .map((m) => `${m.id}:${String(m.enabled)}`)
      .join(","),
  )
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
  // The key size the canvas shows the device at (device-viewport.tsx: its
  // zoom), once the zoom has rested.
  const [unit, setUnit] = useState<number>(KEY_SIZE.fallback)
  // Key images are drawn for the size they are shown at (inside the key's
  // border, device-canvas.tsx) on this display, never scaled up from the
  // device's pixels, within the sizes the renderer draws. Only the
  // preview: the device draws its own images.
  const pixelRatio = useDevicePixelRatio()
  const keyPixels = previewPixels(displayPixels(unit - 2 * keyBorderPixels(unit), pixelRatio))
  const { faces, images, incompatible, reports, navigation } = usePreviewFaces(
    profileId,
    draft,
    pageId,
    `${library.icons.map((i) => i.id).join(",")}|${String(visualLibrary.version)}`,
    liveStates.live,
    keyPixels,
  )
  // Scrolling titles play as on the device (unless motion is reduced).
  const animations = useMarqueeFrames(profileId, draft, pageId, reports, liveStates.live, keyPixels)
  const iconState: IconState = {
    ...library,
    usage: (icon) => effectiveUsage(icon.usages, icon.id, editing.baseline, draft),
  }
  // What the Inspector's key header and its popovers read: the Canvas's
  // images of the keys, the icons and the one way to restyle the selection.
  const keyLook: KeyLook = {
    draft,
    pageId,
    defaults: defaultAppearanceOf(model),
    images,
    animations,
    faces,
    reports,
    icons: iconState,
    library: visualLibrary,
    typeOf: (control) => {
      const target = shownBinding(draft, pageId, control)?.target
      return target ? typeOfTarget(target) : undefined
    },
    restyle: commands.restyle,
  }
  // Without the Workflows plugin, workflows on keys still run but are read-only here.
  const workflowsActive = useIsPluginActive(WORKFLOWS_PLUGIN)
  const groups = useMemo(
    () =>
      targetGroups(
        profile,
        draft,
        pageId,
        (type) => pluginActionUnavailable(modules, type),
        workflowsActive,
      ),
    // The groups' labels are translated: rebuilt when the language changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [profile, draft, pageId, modules, language, workflowsActive],
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
    // The folder's system Back is the agent's: nothing goes on it, and it
    // does not move.
    if (
      isSystemBack(model, draft, pageId, control) ||
      (source.type === "control" && isSystemBack(model, draft, pageId, source.control))
    ) {
      return t("editor:systemBack.dropRefused")
    }
    // A folder key only opens its folder: no action, workflow or page goes
    // on it (moving it, or another key onto it, swaps them).
    if (source.type !== "control" && isFolderControl(draft, pageId, control)) {
      return t("editor:folderControl.dropRefused")
    }
    if (
      source.type === "target" &&
      opensFolderBesides(model, draft, pageId, control, source.target)
    ) {
      return t("editor:folderControl.needsFreeKey")
    }
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
  /**
   * What is dragged, as the announcement names it: its name, or which
   * generic wording to use when it has none.
   */
  const draggedName = (dragged: unknown): { item: string } | "action" | "this" => {
    const source = dragged as DragSource | undefined
    if (source?.type === "actionType") {
      const name = source.preset?.name ?? typeById.get(source.actionType)?.name
      return name ? { item: name } : "action"
    }
    if (source?.type === "target") {
      return { item: targetLabel(source.target, actions, workflows, draft) }
    }
    if (source?.type === "control") return { item: controlLabel(source.control) }
    return "this"
  }
  /** "… cannot go on <control>. <why>", for the announcement. */
  const cannotGo = (control: ControlId, dragged: unknown, reason: string) => {
    const name = draggedName(dragged)
    const values = { control: controlLabel(control), reason }
    if (name === "action") return t("editor:shell.drop.refusedAction", values)
    if (name === "this") return t("editor:shell.drop.refusedThis", values)
    return t("editor:shell.drop.refused", { ...values, item: name.item })
  }
  const refusal = (control: ControlId, dragged: unknown): string | null => {
    const why = dropRejection(control, dragged)
    return why ? cannotGo(control, dragged, why) : null
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
    const message = cannotGo(control, dragged, why)
    const name = draggedName(dragged)
    const values = { control: controlLabel(control) }
    toast.error(
      name === "action"
        ? t("editor:shell.drop.notPutAction", values)
        : name === "this"
          ? t("editor:shell.drop.notPutThis", values)
          : t("editor:shell.drop.notPut", { ...values, item: name.item }),
      { description: why },
    )
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
      return why
        ? t("editor:shell.drop.notPlaced", { reason: why })
        : defaultAnnouncements.onDragEnd(event)
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
      toast.error(t("editor:shell.addFailed", { name: preset?.name ?? type.name }), {
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
      toast.error(t("editor:shell.multiActionFailed"), { description: errorMessage(e) })
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
    toast.error(t("editor:shell.gestureDrop.title"), {
      description: t("editor:shell.gestureDrop.description"),
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
        ].map((key) => [key, t("editor:shell.noPreviousLayout")]),
      ),
    [pageBindings, actions, profiles.activeProfileId, profileId, device, draft.id, t],
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
  // The drawn faces' icons, for the key header's preview as for the Canvas.
  keyLook.symbols = icons
  // Labels such as {project.key} show the profile's active project, as on
  // the device; nothing to choose here (projects are an optional module).
  const previewed = projects.find((p) => p.id === view.activeProjectId)
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
      const labels = bindingsFor(draft, pageId, control).map((b) =>
        t("editor:canvas.summaryEntry", {
          trigger: triggerLabel(b.trigger),
          label: render(b.label ?? targetLabel(b.target, actions, workflows, draft)),
        }),
      )
      if (labels.length > 0) map.set(controlKey(control), labels.join(" · "))
    }
    return map
    // `t` changes with the language, as do the labels of the triggers.
  }, [model, draft, pageId, actions, workflows, previewed, view.profile.name, t])

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
      // The keyboard's double-click: editor navigation only.
      event.preventDefault()
      navigate(control)
    }
  }

  // The device shows and runs the draft while the editor is open, and the
  // editor follows where the device goes (use-live-preview.ts).
  const live = useLivePreview({
    agent,
    device: device ?? null,
    profileId,
    layoutId: draft.id,
    pageId,
    trail,
    draft,
    onDevice: (shown) => {
      if (shown.profile !== profileId || !shown.layout) return
      const page = shown.stack.at(-1)
      const through = shown.stack.slice(0, -1)
      if (shown.layout === draft.id) {
        const moved = page !== pageId || through.join("/") !== trail.join("/")
        if (page && moved && findPage(draft, page)) commands.openAt(page, through)
        return
      }
      // Another layout of the profile: followed unless something is
      // unsaved (the notice above the device offers to open it).
      const other = switchableLayouts(view.profile.layouts, model.id).find(
        (l) => l.id === shown.layout,
      )
      if (!shell || dirty || !other) return
      shell.rememberPlace(`${profileId}:${other.id}`, {
        pageId: page && findPage(other, page) ? page : other.homePage,
        trail: through.filter((id) => findPage(other, id)),
        selection: { controls: [], anchor: null, focus: null },
      })
      shell.selectLayout(other.id)
    },
  })

  const save = async (): Promise<boolean> => {
    setErrors([])
    // The device has the draft that is saved: nothing to draw afterwards.
    live.flush()
    try {
      const saved = await api.saveLayout(profileId, draft)
      toastSaved(
        profiles.activeProfileId === profileId && appliedByAgent(saved.agentReload)
          ? t("editor:shell.saved.sent")
          : t("editor:shell.saved.saved"),
        saved.agentReload,
      )
      const kept = saved.incompatible ?? []
      if (kept.length > 0) {
        toast.warning(t("editor:shell.saved.keptNotRun", { count: kept.length }), {
          description: t("editor:shell.saved.keptNotRunDetail", {
            list: kept.map(readableTriggers).join("; "),
          }),
        })
      }
      onSaved()
      refresh()
      return true
    } catch (e) {
      setErrors(errorMessage(e).split("\n"))
      return false
    }
  }

  // The double-click of the preview: editor navigation only, resolved from
  // data (preview-navigation.ts); it never runs or simulates anything.
  const dialogs = useContext(DialogsContext)
  const layoutsOfModel = switchableLayouts(view.profile.layouts, model.id)
  const moveOf = (control: ControlId): EditorMove | null =>
    isSystemBack(model, draft, pageId, control)
      ? { type: "back" }
      : controlMove(
          draft,
          pageId,
          control,
          actions,
          (id) => typeById.get(id),
          navigation.get(controlKey(control)),
        )
  /** Shows another layout of the profile, asking first over unsaved changes. */
  const showLayout = async (to: string, page: string | null, back: boolean) => {
    const target = layoutsOfModel.find((l) => l.id === to)
    if (!shell || !target) return
    if (to === draft.id) {
      if (page && findPage(draft, page)) commands.openPage(page)
      return
    }
    if (dirty) {
      const discard = dialogs
        ? await dialogs.destructive({
            title: t("editor:editing.discard.title"),
            description: t("editor:editing.discard.description"),
            confirmLabel: t("editor:editing.discard.confirm"),
          })
        : shell.guard()
      if (!discard) return
      shell.setDirty(false)
    }
    // The other layout opens at its page (else its home page), with no
    // folder trail; zoom and pan are the model's, so they stay.
    const shown = page && findPage(target, page) ? page : target.homePage
    shell.rememberPlace(`${profileId}:${to}`, {
      pageId: shown,
      trail: [],
      selection: { controls: [], anchor: null, focus: null },
    })
    if (back) popLayoutHistory(profileId, model.id)
    else pushLayoutHistory(profileId, model.id, draft.id)
    shell.selectLayout(to)
  }
  const navigate = (control: ControlId) => {
    const move = moveOf(control)
    if (!move) return
    switch (move.type) {
      case "enter":
        commands.enter(move.page)
        return
      case "back":
        if (canGoBack(draft, pageId, editing.trail)) commands.back()
        return
      case "page": {
        const to = adjacentPage(draft, pageId, move.direction, move.wrapAround)
        if (to) commands.openPage(to)
        return
      }
      case "layout":
        void showLayout(move.layout, move.page, false)
        return
      case "layoutBack": {
        const previous = peekLayoutHistory(profileId, model.id)
        if (previous) void showLayout(previous, null, true)
        return
      }
    }
  }

  const simulation = useDeviceSimulation(
    {
      profileId,
      layoutId: draft.id,
      pageId,
      device: device ?? null,
      agent,
      activeProfileId: profiles.activeProfileId,
    },
    dirty,
    save,
    // The device runs the draft: a test runs what is on screen.
    live.live,
  )
  /** "Remove assignment": asks first, as it removes bindings (undo still works). */
  const removeAssignments = async () => {
    if (selected.length === 0) return
    if (dialogs) {
      const subject =
        selected.length === 1 && selected[0]
          ? controlLabel(selected[0])
          : t("editor:controlMenu.controls", { count: selected.length })
      const ok = await dialogs.destructive({
        title: t("editor:preview.remove.title", { subject }),
        description: t("editor:preview.remove.description"),
        confirmLabel: t("editor:controlMenu.clear"),
      })
      if (!ok) return
    }
    commands.clear()
  }
  const focusInspector = () => {
    const inspector = document.querySelector<HTMLElement>(
      "[data-slot=device-editor] [data-slot=inspector]",
    )
    const field = inspector?.querySelector<HTMLElement>(
      "input:not([disabled]), textarea:not([disabled]), button:not([disabled]), [role=combobox]",
    )
    ;(field ?? inspector)?.focus()
  }

  const folder = selection.focus ? folderOf(draft, pageId, selection.focus) : undefined
  // The folder's protected system Back on the canvas, with what it is.
  const systemBack = systemBackControl(model, draft, pageId)
  const systemControls = useMemo(
    () =>
      systemBack
        ? new Map([[controlKey(systemBack), t("editor:systemBack.canvasDescription")]])
        : undefined,
    // `t` changes with the language.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [systemBack?.kind, systemBack?.index, t],
  )
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
      <div data-slot="device-editor" className="relative flex min-h-0 flex-1 max-[859px]:flex-col">
        {/*
          The workspace: one dark surface (`dark workspace-scope`, see
          index.css) in either app theme, in three parts that never move
          each other: the editing bar, the device canvas (a fixed share of
          the height, zoomed and panned, never scrolling: device-viewport.tsx)
          with its page navigation, and below it the inspector, the only
          part that scrolls.
          A scroll container must also be the containing block of its
          content (`relative`): otherwise absolutely positioned descendants,
          such as the visually hidden file input of the icon controls, are not
          clipped there and stretch the next container up instead, which then
          scrolls past the editor into blank space.
        */}
        <div
          data-slot="device-editor-main"
          className="dark workspace-scope relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-workspace text-workspace-foreground"
        >
          <div
            role="toolbar"
            aria-label={t("editor:bar.label")}
            data-slot="editing-bar"
            className="z-10 box-content flex h-(--toolbar-height) shrink-0 items-center gap-1 border-b bg-workspace px-3"
          >
            <p
              className="min-w-0 flex-1 truncate text-xs text-muted-foreground"
              data-slot="editing-state"
            >
              {dirty ? (
                <span className="inline-flex items-center gap-1.5 font-medium text-foreground">
                  <span aria-hidden className="size-1.5 rounded-full bg-warning" />
                  {t("editor:bar.unsaved")}
                </span>
              ) : (
                t("editor:bar.allSaved")
              )}
              <LiveBadge status={live.status} problem={live.problem} />
            </p>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("editor:bar.undo")}
              title={t("navigation:toolbar.withShortcut", {
                label: t("editor:bar.undo"),
                shortcut: shortcutLabel("Z"),
              })}
              disabled={!editing.canUndo}
              onClick={commands.undo}
            >
              <Undo2 />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("editor:bar.redo")}
              title={t("navigation:toolbar.withShortcut", {
                label: t("editor:bar.redo"),
                shortcut: shortcutLabel("Z", true),
              })}
              disabled={!editing.canRedo}
              onClick={commands.redo}
            >
              <Redo2 />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={!dirty}
              onClick={() => {
                commands.revert()
                setErrors([])
              }}
            >
              <RotateCcw /> {t("editor:bar.revert")}
            </Button>
            <Button
              size="sm"
              disabled={!dirty}
              title={t("navigation:toolbar.withShortcut", {
                label: t("editor:bar.save"),
                shortcut: shortcutLabel("S"),
              })}
              onClick={() => void save()}
            >
              <Save /> {t("editor:bar.save")}
            </Button>
          </div>
          <p className="sr-only" role="status" aria-live="assertive" data-slot="drop-announcement">
            {rejected?.message ?? ""}
          </p>
          {hasControls ? (
            <DeviceViewport
              model={model}
              onKeySizeSettled={setUnit}
              status={<LiveValuesNote status={liveStates.status} live={liveStates.live} />}
              footer={
                <PageNavigationDock
                  draft={draft}
                  pageId={pageId}
                  trail={editing.trail}
                  onOpen={commands.openPage}
                  onNavigate={commands.openAt}
                  onAdd={(name) => {
                    commands.addPage(name)
                  }}
                  onRename={(name) => {
                    commands.renamePage(name, pageId)
                  }}
                  navigator={(close) => (
                    <PageNavigator
                      profileId={profileId}
                      draft={draft}
                      dirty={dirty}
                      pageId={pageId}
                      newPageName={() => newPageName(draft)}
                      onClose={close}
                      commands={{
                        open: commands.openPage,
                        rename: (page, name) => {
                          commands.renamePage(name, page)
                        },
                        remove: commands.deletePage,
                        addIn: (folder, name) => {
                          commands.addPage(name, folder)
                        },
                        setAutoReturn: (page, on) => {
                          commands.setAutoReturn(on, page)
                        },
                        stored: (layout, page) => {
                          if (layout) commands.reset(layout, page)
                          onSaved()
                          refresh()
                        },
                      }}
                    />
                  )}
                />
              }
              renderCanvas={(keySize, visible) => (
                <DeviceCanvas
                  model={model}
                  keySize={keySize}
                  visible={visible}
                  images={images}
                  animations={animations}
                  faces={faces}
                  unavailable={noWayBack}
                  system={systemControls}
                  icons={icons}
                  captions={summaries}
                  summaries={summaries}
                  selection={selection.controls}
                  focused={selection.focus}
                  onSelect={commands.select}
                  onOpen={navigate}
                  onControlKeyDown={onControlKeyDown}
                  droppable
                  draggable
                  dropRejection={dropRejection}
                  rejected={rejected?.control ?? null}
                  wrapControl={(control, element) => {
                    const protectedBack = isSystemBack(model, draft, pageId, control)
                    return (
                      <ControlMenu
                        label={controlLabel(control)}
                        groups={groupsFor(control)}
                        state={{
                          count: selected.length,
                          hasBindings: editable.some((c) => bound.has(controlKey(c))),
                          canPaste: editing.clipboard !== null,
                          folderName: (() => {
                            const id = folderOf(draft, pageId, control)
                            return id ? findPage(draft, id)?.name : undefined
                          })(),
                          canCreateFolder: isFree(draft, pageId, control) && !protectedBack,
                          systemBack: protectedBack,
                          folderControl: isFolderControl(draft, pageId, control),
                          goesBack:
                            moveOf(control)?.type === "back" &&
                            canGoBack(draft, pageId, editing.trail),
                          testUnavailable:
                            !protectedBack &&
                            !bound.has(controlKey(control)) &&
                            !navigation.get(controlKey(control))?.automatic
                              ? t("editor:preview.why.noAction")
                              : selected.length > 1
                                ? t("editor:preview.why.oneControl")
                                : simulation.unavailable,
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
                          clear: () => {
                            void removeAssignments()
                          },
                          goBack: commands.back,
                          test: (kind) => {
                            void simulation.run(control, kind)
                          },
                          configure: () => {
                            commands.select(control, "only")
                            // After the inspector shows the control.
                            setTimeout(focusInspector, 0)
                          },
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
                    )
                  }}
                />
              )}
            />
          ) : (
            <div className="flex-none border-b px-4 pt-5 pb-4">
              <EmptyState
                variant="plain"
                title={t("editor:shell.noControls", { model: model.name })}
              />
            </div>
          )}
          {/*
            The inspector region: the editor's only scroll container (and
            the containing block of its content, see above).
          */}
          {/*
            The inspector region: a frosted curtain (index.css,
            `.inspector-curtain`) over the top of the scrolling content once
            it is scrolled; it never scrolls itself, takes no space and no
            pointer events. Above it: the deselect button (z-10) and the
            page dock on the divider (z-20).
          */}
          <div data-slot="inspector-region" className="relative flex min-h-0 flex-1 flex-col">
            <div aria-hidden data-slot="inspector-curtain" className="inspector-curtain">
              <span />
              <span />
              <span />
            </div>
            <div
              data-slot="inspector-scroll"
              // Below the bar that straddles the divider (device-viewport.tsx):
              // half its height is kept free at the top.
              className="relative min-h-0 flex-1 overflow-y-auto px-4 pt-[calc(var(--canvas-divider-half,0px)+0.75rem)] pb-6"
              onScroll={(event) => {
                // Only whether it is scrolled at all (no state, no render): the
                // curtain fades in from 2px so fractional offsets don't flicker.
                event.currentTarget.parentElement?.toggleAttribute(
                  "data-scrolled",
                  event.currentTarget.scrollTop > 2,
                )
              }}
            >
              <InspectorDeselect
                selection={selected}
                focus={selection.focus}
                onClose={commands.clearSelection}
              />
              <div
                className={cn(
                  "mx-auto grid w-full max-w-3xl gap-3",
                  // A notice above the inspector keeps the deselect button's width free.
                  selected.length > 0 && "[&>:first-child:not([data-slot=inspector])]:pr-9",
                )}
              >
                {(notices !== undefined || errors.length > 0) && (
                  <div
                    data-slot="workspace-notices"
                    className="grid w-full max-w-3xl gap-2 empty:hidden"
                  >
                    {errors.length > 0 && (
                      <ErrorNotice error={errors.map(readableTriggers).join(" ")} />
                    )}
                    {notices}
                  </div>
                )}
                {hasControls && bound.size === 0 && (
                  <div
                    data-slot="page-empty"
                    className="mx-auto grid max-w-md gap-0.5 text-center text-xs text-muted-foreground"
                  >
                    <p className="font-medium text-foreground">
                      {isFolderPage ? t("editor:shell.folderEmpty") : t("editor:shell.pageEmpty")}
                    </p>
                    <p>{t("editor:shell.emptyHint")}</p>
                  </div>
                )}
                {isFolderPage && !systemBack && !hasWayBack(draft, pageId) && (
                  <p
                    className="mx-auto max-w-md text-center text-xs text-muted-foreground"
                    role="note"
                  >
                    {t("editor:shell.noBackKey")}
                  </p>
                )}
                <KeyLookContext.Provider value={keyLook}>
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
                  />
                </KeyLookContext.Provider>
              </div>
            </div>
          </div>
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
      <DeletePageDialog
        deletion={editing.deleting ? pageDeletion(draft, editing.deleting) : null}
        onCancel={commands.cancelDeletePage}
        onConfirm={commands.confirmDeletePage}
      />
      {isStepWorkflow(openSteps) && (
        <ModalShell
          presentation="dialog"
          size="large"
          icon={ListOrdered}
          title={t("editor:shell.multiAction.title")}
          description={t("editor:shell.multiAction.description")}
          onClose={() => {
            setStepsOf(null)
          }}
        >
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
        </ModalShell>
      )}
    </DndContext>
  )
}

const LIVE_LABELS: Partial<Record<LiveStatus, TranslationKey>> = {
  live: "editor:live.live",
  suspended: "editor:live.suspended",
  connecting: "editor:live.connecting",
  disconnected: "editor:live.disconnected",
  unsupported: "editor:live.unsupported",
  otherProfile: "editor:live.otherProfile",
  refused: "editor:live.refused",
}

/** Whether the device shows the draft, in the editing bar. */
function LiveBadge({ status, problem }: { status: LiveStatus; problem: string | null }) {
  const { t } = useTranslation()
  const label = LIVE_LABELS[status]
  if (!label) return null
  return (
    <span
      data-slot="live-preview-status"
      data-status={status}
      title={problem ?? undefined}
      className="ms-2 inline-flex items-center gap-1.5"
    >
      <span
        aria-hidden
        className={cn(
          "size-1.5 rounded-full",
          status === "live" ? "bg-success" : "bg-muted-foreground/50",
        )}
      />
      {t(label)}
    </span>
  )
}
