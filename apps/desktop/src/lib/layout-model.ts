// Pure helpers for the layout designer. Everything is derived from the
// device model's capabilities; no geometry is hard-coded.

import type {
  ActionInstance,
  BindingTarget,
  ControlBinding,
  ControlId,
  DeviceLayout,
  DeviceModel,
  LayoutPage,
  Trigger,
  Workflow,
} from "./types"

export interface ModelControls {
  keys: { control: ControlId; row: number; column: number }[]
  columns: number
  encoders: ControlId[]
  touchZones: ControlId[]
}

/** Every control of a model, with grid positions for keys (row-major). */
export function controlsOf(model: DeviceModel): ModelControls {
  const grid = model.capabilities.keys
  const keys = grid
    ? Array.from({ length: grid.rows * grid.columns }, (_, index) => ({
        control: { kind: "key" as const, index },
        row: Math.floor(index / grid.columns),
        column: index % grid.columns,
      }))
    : []
  const encoders = Array.from(
    { length: model.capabilities.encoders?.count ?? 0 },
    (_, index): ControlId => ({ kind: "encoder", index }),
  )
  const touchZones = Array.from(
    { length: model.capabilities.touch?.zones ?? 0 },
    (_, index): ControlId => ({ kind: "touchZone", index }),
  )
  return { keys, columns: grid?.columns ?? 0, encoders, touchZones }
}

export function controlKey(control: ControlId): string {
  return `${control.kind}:${control.index}`
}

export function sameControl(a: ControlId, b: ControlId): boolean {
  return a.kind === b.kind && a.index === b.index
}

export function triggerKey(trigger: Trigger): string {
  return trigger.type === "touch" ? `touch:${trigger.gesture}` : trigger.type
}

export function sameTrigger(a: Trigger, b: Trigger): boolean {
  return triggerKey(a) === triggerKey(b)
}

/** Triggers the model can produce on a control (mirrors backend validation). */
export function supportedTriggers(model: DeviceModel, control: ControlId): Trigger[] {
  switch (control.kind) {
    case "key":
      return [{ type: "press" }, { type: "release" }]
    case "encoder": {
      const triggers: Trigger[] = [{ type: "rotateClockwise" }, { type: "rotateCounterClockwise" }]
      if (model.capabilities.encoders?.pressable) {
        triggers.push({ type: "press" }, { type: "release" })
      }
      return triggers
    }
    case "touchZone":
      return (model.capabilities.touch?.gestures ?? []).map((gesture) => ({
        type: "touch",
        gesture,
      }))
  }
}

/** The gesture triggers, in the order the editor shows them. */
export const GESTURE_TRIGGERS: Trigger[] = [
  { type: "click" },
  { type: "doubleClick" },
  { type: "longPress" },
]

export function isGestureTrigger(trigger: Trigger): boolean {
  return trigger.type === "click" || trigger.type === "doubleClick" || trigger.type === "longPress"
}

/**
 * Whether a control can take gesture triggers: it must press and release
 * (keys and pressable dials), as the backend validates.
 */
export function acceptsGestures(model: DeviceModel, control: ControlId): boolean {
  const triggers = supportedTriggers(model, control)
  return triggers.some((t) => t.type === "press") && triggers.some((t) => t.type === "release")
}

/** Every trigger a binding of the control may use: raw edges and gestures. */
export function bindableTriggers(model: DeviceModel, control: ControlId): Trigger[] {
  const raw = supportedTriggers(model, control)
  return acceptsGestures(model, control) ? [...raw, ...GESTURE_TRIGGERS] : raw
}

const TRIGGER_LABELS: Record<string, string> = {
  click: "Click",
  doubleClick: "Double click",
  longPress: "Long press",
  press: "Press",
  release: "Release",
  rotateClockwise: "Turn right",
  rotateCounterClockwise: "Turn left",
  "touch:tap": "Tap",
  "touch:longPress": "Long press",
  "touch:swipe": "Swipe",
}

export function triggerLabel(trigger: Trigger): string {
  return TRIGGER_LABELS[triggerKey(trigger)] ?? triggerKey(trigger)
}

export function controlLabel(control: ControlId): string {
  const n = control.index + 1
  switch (control.kind) {
    case "key":
      return `Key ${n}`
    case "encoder":
      return `Dial ${n}`
    case "touchZone":
      return `Touch zone ${n}`
  }
}

export function page(layout: DeviceLayout, pageId: string): LayoutPage | undefined {
  return layout.pages.find((p) => p.id === pageId)
}

export function bindingsFor(layout: DeviceLayout, pageId: string, control: ControlId) {
  return page(layout, pageId)?.bindings.filter((b) => sameControl(b.control, control)) ?? []
}

function mapPage(
  layout: DeviceLayout,
  pageId: string,
  change: (page: LayoutPage) => LayoutPage,
): DeviceLayout {
  return { ...layout, pages: layout.pages.map((p) => (p.id === pageId ? change(p) : p)) }
}

/** Adds or replaces the binding for the same control and trigger. */
export function setBinding(
  layout: DeviceLayout,
  pageId: string,
  binding: ControlBinding,
): DeviceLayout {
  return mapPage(layout, pageId, (p) => ({
    ...p,
    bindings: [
      ...p.bindings.filter(
        (b) =>
          !(sameControl(b.control, binding.control) && sameTrigger(b.trigger, binding.trigger)),
      ),
      binding,
    ],
  }))
}

export function removeBinding(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  trigger: Trigger,
): DeviceLayout {
  return mapPage(layout, pageId, (p) => ({
    ...p,
    bindings: p.bindings.filter(
      (b) => !(sameControl(b.control, control) && sameTrigger(b.trigger, trigger)),
    ),
  }))
}

/**
 * The first supported trigger that is still free on a control. A control
 * that already uses gestures gets a gesture (never a raw edge, which would
 * mix both kinds), falling back to replacing its click.
 */
export function defaultTrigger(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): Trigger | undefined {
  const used = bindingsFor(layout, pageId, control)
  const supported = supportedTriggers(model, control)
  const triggers = used.some((b) => isGestureTrigger(b.trigger))
    ? [...GESTURE_TRIGGERS, ...supported.filter((t) => t.type !== "press" && t.type !== "release")]
    : supported
  return triggers.find((t) => !used.some((b) => sameTrigger(b.trigger, t))) ?? triggers[0]
}

export function addPage(layout: DeviceLayout, id: string, name: string): DeviceLayout {
  return { ...layout, pages: [...layout.pages, { id, name, bindings: [] }] }
}

export function renamePage(layout: DeviceLayout, pageId: string, name: string): DeviceLayout {
  return mapPage(layout, pageId, (p) => ({ ...p, name }))
}

/** Removes a page and every binding that opens it. The home page stays. */
export function deletePage(layout: DeviceLayout, pageId: string): DeviceLayout {
  if (pageId === layout.homePage) return layout
  return {
    ...layout,
    pages: layout.pages
      .filter((p) => p.id !== pageId)
      .map((p) => ({
        ...p,
        bindings: p.bindings.filter(
          (b) => !(b.target.type === "openPage" && b.target.page === pageId),
        ),
      })),
  }
}

export function targetLabel(
  target: BindingTarget,
  actions: ActionInstance[],
  workflows: Workflow[],
  layout: DeviceLayout,
): string {
  switch (target.type) {
    case "action":
      return actions.find((a) => a.id === target.action)?.name ?? "Missing action"
    case "workflow":
      return workflows.find((w) => w.id === target.workflow)?.name ?? "Missing workflow"
    case "openPage":
      return `Open “${page(layout, target.page)?.name ?? "missing page"}”`
    case "back":
      return "Back"
  }
}

/** Number of bindings on a page that the editor will save. */
export function bindingCount(layout: DeviceLayout): number {
  return layout.pages.reduce((sum, p) => sum + p.bindings.length, 0)
}
