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

const TRIGGER_LABELS: Record<string, string> = {
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

/** The first supported trigger that is still free on a control. */
export function defaultTrigger(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): Trigger | undefined {
  const used = bindingsFor(layout, pageId, control)
  const triggers = supportedTriggers(model, control)
  return triggers.find((t) => !used.some((b) => sameTrigger(b.trigger, t))) ?? triggers[0]
}

export function addPage(layout: DeviceLayout, id: string, name: string): DeviceLayout {
  return { ...layout, pages: [...layout.pages, { id, name, bindings: [] }] }
}

export function renamePage(layout: DeviceLayout, pageId: string, name: string): DeviceLayout {
  return mapPage(layout, pageId, (p) => ({ ...p, name }))
}

/**
 * Removes a page and every binding that opens it. The home page stays.
 * Pages stored inside it move up to its folder (or the top level), so the
 * layout keeps a valid page tree.
 */
export function deletePage(layout: DeviceLayout, pageId: string): DeviceLayout {
  if (pageId === layout.homePage) return layout
  const folder = pageParents(layout).get(pageId)
  return {
    ...layout,
    pages: layout.pages
      .filter((p) => p.id !== pageId)
      .map((p) => {
        const bindings = p.bindings.filter(
          (b) => !(b.target.type === "openPage" && b.target.page === pageId),
        )
        // `undefined` is not serialized: the page is then top level.
        return p.parent === pageId ? { ...p, parent: folder, bindings } : { ...p, bindings }
      }),
  }
}

/** Whether making `parent` the folder of `page` would make a cycle. */
function createsCycle(parents: Map<string, string>, page: string, parent: string): boolean {
  let current: string | undefined = parent
  let steps = 0
  while (current !== undefined) {
    if (current === page || steps > parents.size) return true
    current = parents.get(current)
    steps += 1
  }
  return false
}

/**
 * The folder of every page that is in one, as the agent sees it
 * (`DeviceLayout::page_parents`): a valid stored parent wins; layouts
 * without `explicitTree` infer the rest in a breadth-first walk from the
 * home page through `openPage` bindings.
 */
export function pageParents(layout: DeviceLayout): Map<string, string> {
  const exists = new Set(layout.pages.map((p) => p.id))
  const parents = new Map<string, string>()
  for (const p of layout.pages) {
    if (
      p.parent &&
      p.id !== layout.homePage &&
      exists.has(p.parent) &&
      !createsCycle(parents, p.id, p.parent)
    ) {
      parents.set(p.id, p.parent)
    }
  }
  if (layout.explicitTree) return parents
  const explicit = new Set(parents.keys())
  const visited = new Set([layout.homePage])
  const queue = [layout.homePage]
  for (let current = queue.shift(); current !== undefined; current = queue.shift()) {
    const p = page(layout, current)
    if (!p) continue
    const opened = p.bindings.flatMap((b) => (b.target.type === "openPage" ? [b.target.page] : []))
    const children = layout.pages
      .filter((c) => explicit.has(c.id) && parents.get(c.id) === current)
      .map((c) => c.id)
    for (const target of [...opened, ...children]) {
      if (!exists.has(target) || visited.has(target)) continue
      visited.add(target)
      if (
        target !== layout.homePage &&
        !parents.has(target) &&
        !createsCycle(parents, target, current)
      ) {
        parents.set(target, current)
      }
      queue.push(target)
    }
  }
  return parents
}

/** The folders a page is in, the nearest first. */
export function ancestors(layout: DeviceLayout, pageId: string): string[] {
  const parents = pageParents(layout)
  const out: string[] = []
  for (let id = parents.get(pageId); id !== undefined && !out.includes(id); id = parents.get(id)) {
    out.push(id)
  }
  return out
}

/**
 * Whether a page inside a folder can be left on the device: it has a Back
 * binding or one that opens one of its folders. Pages in no folder need
 * none. Without one the agent adds an automatic Back on the first free key.
 */
export function hasWayBack(layout: DeviceLayout, pageId: string): boolean {
  const folders = ancestors(layout, pageId)
  if (folders.length === 0) return true
  const p = page(layout, pageId)
  return (
    !p ||
    p.bindings.some(
      (b) =>
        b.target.type === "back" ||
        (b.target.type === "openPage" && folders.includes(b.target.page)),
    )
  )
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
