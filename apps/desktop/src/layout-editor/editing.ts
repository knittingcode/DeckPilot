// Pure editing logic of the layout editor: selection, keyboard navigation,
// clipboard, move/swap, duplicate and undo history. Everything is derived
// from the device model's controls; no geometry is hard-coded.

import {
  bindingsFor,
  controlKey,
  controlsOf,
  defaultTrigger,
  page as findPage,
  sameControl,
  sameTrigger,
  setBinding,
  supportedTriggers,
} from "@/lib/layout-model"
import type {
  BindingTarget,
  ControlBinding,
  ControlId,
  DeviceLayout,
  DeviceModel,
} from "@/lib/types"

// ---------------------------------------------------------------------------
// Control order and navigation

/** Every control in visual order: keys (row-major), touch zones, dials. */
export function orderedControls(model: DeviceModel): ControlId[] {
  const { keys, touchZones, encoders } = controlsOf(model)
  return [...keys.map((k) => k.control), ...touchZones, ...encoders]
}

/** Controls grouped in visual rows, top to bottom. */
export function controlRows(model: DeviceModel): ControlId[][] {
  const { keys, touchZones, encoders } = controlsOf(model)
  const rows: ControlId[][] = []
  for (const { control, row } of keys) {
    ;(rows[row] ??= []).push(control)
  }
  if (touchZones.length > 0) rows.push(touchZones)
  if (encoders.length > 0) rows.push(encoders)
  return rows.filter((r) => r.length > 0)
}

export type Direction = "up" | "down" | "left" | "right"

/** The control reached from `from` with an arrow key; `from` at an edge. */
export function neighbour(model: DeviceModel, from: ControlId, direction: Direction): ControlId {
  const rows = controlRows(model)
  const rowIndex = rows.findIndex((r) => r.some((c) => sameControl(c, from)))
  if (rowIndex < 0) return rows[0]?.[0] ?? from
  const row = rows[rowIndex] ?? []
  const column = row.findIndex((c) => sameControl(c, from))
  if (direction === "left") return row[Math.max(0, column - 1)] ?? from
  if (direction === "right") return row[Math.min(row.length - 1, column + 1)] ?? from
  const target = rows[rowIndex + (direction === "up" ? -1 : 1)]
  if (!target) return from
  // Rows can differ in length (keys above a touch strip): keep the position.
  const position = (column + 0.5) / row.length
  return target[Math.min(target.length - 1, Math.floor(position * target.length))] ?? from
}

// ---------------------------------------------------------------------------
// Selection

export interface Selection {
  /** Selected controls, in the order they were selected. */
  controls: ControlId[]
  /** The control range selection starts from. */
  anchor: ControlId | null
  /** The control acted on last; shown in the inspector. */
  focus: ControlId | null
}

export const emptySelection: Selection = { controls: [], anchor: null, focus: null }

export function isSelected(selection: Selection, control: ControlId): boolean {
  return selection.controls.some((c) => sameControl(c, control))
}

export function selectOnly(control: ControlId): Selection {
  return { controls: [control], anchor: control, focus: control }
}

/** Cmd/Ctrl-click: adds or removes one control. */
export function toggleSelected(selection: Selection, control: ControlId): Selection {
  if (isSelected(selection, control)) {
    const controls = selection.controls.filter((c) => !sameControl(c, control))
    const focus = controls.at(-1) ?? null
    return { controls, anchor: focus, focus }
  }
  return { controls: [...selection.controls, control], anchor: control, focus: control }
}

/** Shift-click: every control between the anchor and `control`. */
export function extendSelection(
  model: DeviceModel,
  selection: Selection,
  control: ControlId,
): Selection {
  const anchor = selection.anchor ?? control
  const order = orderedControls(model)
  const a = order.findIndex((c) => sameControl(c, anchor))
  const b = order.findIndex((c) => sameControl(c, control))
  if (a < 0 || b < 0) return selectOnly(control)
  const controls = order.slice(Math.min(a, b), Math.max(a, b) + 1)
  return { controls, anchor, focus: control }
}

export type SelectMode = "only" | "toggle" | "extend"

export function select(
  model: DeviceModel,
  selection: Selection,
  control: ControlId,
  mode: SelectMode,
): Selection {
  if (mode === "toggle") return toggleSelected(selection, control)
  if (mode === "extend") return extendSelection(model, selection, control)
  return selectOnly(control)
}

/** Selected controls in visual order. */
export function selectedInOrder(model: DeviceModel, selection: Selection): ControlId[] {
  return orderedControls(model).filter((c) => isSelected(selection, c))
}

// ---------------------------------------------------------------------------
// Bindings of whole controls

type ControlBindings = Omit<ControlBinding, "control">[]

/** A control's bindings without the control; label and appearance travel along. */
function strip(bindings: ControlBinding[]): ControlBindings {
  return bindings.map(({ trigger, target, label, appearance }) => ({
    trigger,
    target,
    label: label ?? null,
    ...(appearance ? { appearance } : {}),
  }))
}

/** Removes every binding of the given controls on a page. */
export function clearControls(
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
): DeviceLayout {
  return {
    ...layout,
    pages: layout.pages.map((p) =>
      p.id === pageId
        ? {
            ...p,
            bindings: p.bindings.filter((b) => !controls.some((c) => sameControl(c, b.control))),
          }
        : p,
    ),
  }
}

/** Whether a target can be placed on a page of the layout. */
function targetFits(layout: DeviceLayout, pageId: string, target: BindingTarget): boolean {
  if (target.type !== "openPage") return true
  return target.page !== pageId && findPage(layout, target.page) !== undefined
}

/**
 * Replaces the bindings of `control` with `bindings`, keeping only triggers
 * the control supports and targets valid on the page. Returns the layout
 * and the number of bindings that had to be left out.
 */
function placeBindings(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  bindings: ControlBindings,
): { layout: DeviceLayout; skipped: number } {
  const supported = supportedTriggers(model, control)
  let next = clearControls(layout, pageId, [control])
  let skipped = 0
  for (const binding of bindings) {
    if (
      !supported.some((t) => sameTrigger(t, binding.trigger)) ||
      !targetFits(next, pageId, binding.target)
    ) {
      skipped += 1
      continue
    }
    next = setBinding(next, pageId, { ...binding, control })
  }
  return { layout: next, skipped }
}

/** Assigns a target to each control on its first free supported trigger. */
export function assignTarget(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
  target: BindingTarget,
): DeviceLayout {
  if (!targetFits(layout, pageId, target)) return layout
  let next = layout
  for (const control of controls) {
    const trigger = defaultTrigger(model, next, pageId, control)
    if (!trigger) continue
    // Replacing a binding keeps how the key looks.
    const replaced = bindingsFor(next, pageId, control).find((b) => sameTrigger(b.trigger, trigger))
    next = setBinding(next, pageId, {
      control,
      trigger,
      target,
      label: null,
      ...(replaced?.appearance ? { appearance: replaced.appearance } : {}),
    })
  }
  return next
}

/** The page a control opens with its press (or first) binding, if any. */
export function folderOf(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): string | undefined {
  const opening = bindingsFor(layout, pageId, control).filter(
    (b): b is ControlBinding & { target: { type: "openPage"; page: string } } =>
      b.target.type === "openPage",
  )
  const preferred = opening.find((b) => b.trigger.type === "press") ?? opening[0]
  return preferred && findPage(layout, preferred.target.page) ? preferred.target.page : undefined
}

// ---------------------------------------------------------------------------
// Clipboard

export interface Clipboard {
  /** Copied controls with their position relative to the first one. */
  entries: { offset: number; control: ControlId; bindings: ControlBindings }[]
}

/** Copies the bindings of the selected controls (empty controls included). */
export function copyControls(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
): Clipboard | null {
  const order = orderedControls(model)
  const indexed = controls
    .map((control) => ({ control, index: order.findIndex((c) => sameControl(c, control)) }))
    .filter((c) => c.index >= 0)
    .sort((a, b) => a.index - b.index)
  const first = indexed[0]
  if (!first) return null
  return {
    entries: indexed.map(({ control, index }) => ({
      offset: index - first.index,
      control,
      bindings: strip(bindingsFor(layout, pageId, control)),
    })),
  }
}

export interface PasteResult {
  layout: DeviceLayout
  /** Controls that received bindings (or were cleared by an empty entry). */
  controls: ControlId[]
  /** Bindings left out because the control or page cannot take them. */
  skipped: number
}

/**
 * Pastes the clipboard. One copied control pasted onto several selected
 * controls fills each of them; otherwise the copied controls keep their
 * relative positions, starting at the first target. Pasting replaces the
 * bindings of the receiving controls.
 */
export function pasteControls(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  clipboard: Clipboard,
  targets: ControlId[],
): PasteResult {
  const order = orderedControls(model)
  const sortedTargets = order.filter((c) => targets.some((t) => sameControl(t, c)))
  const start = sortedTargets[0]
  const single = clipboard.entries.length === 1 ? clipboard.entries[0] : undefined
  const placements: { control: ControlId; bindings: ControlBindings }[] = []
  let skipped = 0
  if (single && sortedTargets.length > 1) {
    for (const control of sortedTargets) placements.push({ control, bindings: single.bindings })
  } else if (start) {
    const base = order.findIndex((c) => sameControl(c, start))
    for (const entry of clipboard.entries) {
      const control = order[base + entry.offset]
      if (control) placements.push({ control, bindings: entry.bindings })
      else skipped += entry.bindings.length
    }
  }
  let next = layout
  for (const { control, bindings } of placements) {
    const placed = placeBindings(model, next, pageId, control, bindings)
    next = placed.layout
    skipped += placed.skipped
  }
  return { layout: next, controls: placements.map((p) => p.control), skipped }
}

// ---------------------------------------------------------------------------
// Move, swap and duplicate

/**
 * Moves the bindings of `from` onto `to`; bindings already on `to` move to
 * `from` (a swap). Returns `null` when a binding would not be supported at
 * its new place, e.g. a dial turn moved onto a key.
 */
export function moveControl(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  from: ControlId,
  to: ControlId,
): DeviceLayout | null {
  if (sameControl(from, to)) return layout
  const source = strip(bindingsFor(layout, pageId, from))
  const dest = strip(bindingsFor(layout, pageId, to))
  const fits = (control: ControlId, bindings: ControlBindings) => {
    const supported = supportedTriggers(model, control)
    return bindings.every((b) => supported.some((t) => sameTrigger(t, b.trigger)))
  }
  if (!fits(to, source) || !fits(from, dest)) return null
  let next = clearControls(layout, pageId, [from, to])
  for (const b of source) next = setBinding(next, pageId, { ...b, control: to })
  for (const b of dest) next = setBinding(next, pageId, { ...b, control: from })
  return next
}

/**
 * Copies each control's bindings onto the next empty control of the same
 * kind after it. Returns the new layout and the controls that were filled.
 */
export function duplicateControls(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
): { layout: DeviceLayout; controls: ControlId[] } {
  const order = orderedControls(model)
  const sources = order.filter((c) => controls.some((s) => sameControl(s, c)))
  let next = layout
  const created: ControlId[] = []
  for (const source of sources) {
    const bindings = strip(bindingsFor(next, pageId, source))
    if (bindings.length === 0) continue
    const start = order.findIndex((c) => sameControl(c, source))
    const free = order
      .slice(start + 1)
      .find(
        (c) =>
          c.kind === source.kind &&
          !sources.some((s) => sameControl(s, c)) &&
          bindingsFor(next, pageId, c).length === 0,
      )
    if (!free) continue
    next = placeBindings(model, next, pageId, free, bindings).layout
    created.push(free)
  }
  return { layout: next, controls: created }
}

/** What is being dragged: a library entry or the bindings of a control. */
export type DragSource =
  { type: "target"; target: BindingTarget } | { type: "control"; control: ControlId }

/**
 * Drops a library entry (assigned to the first free trigger) or a control
 * (moved, swapping with the destination). `null` when the drop is refused.
 */
export function dropOnControl(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  source: DragSource,
  over: ControlId,
): DeviceLayout | null {
  if (source.type === "control") return moveControl(model, layout, pageId, source.control, over)
  if (!targetFits(layout, pageId, source.target)) return null
  return assignTarget(model, layout, pageId, [over], source.target)
}

/** Parses a key made by `controlKey`. */
export function parseControlKey(key: string): ControlId | null {
  const [kind, index] = key.split(":")
  const n = Number(index)
  if (!Number.isInteger(n) || n < 0) return null
  if (kind !== "key" && kind !== "encoder" && kind !== "touchZone") return null
  return { kind, index: n }
}

// ---------------------------------------------------------------------------
// Pages

/** The page "back" returns to from `pageId`: the first page opening it, else home. */
export function parentPage(layout: DeviceLayout, pageId: string): string {
  if (pageId === layout.homePage) return pageId
  const opener = layout.pages.find((p) =>
    p.bindings.some((b) => b.target.type === "openPage" && b.target.page === pageId),
  )
  return opener?.id ?? layout.homePage
}

/** Whether a folder page has a binding to leave it. */
export function hasBack(layout: DeviceLayout, pageId: string): boolean {
  return findPage(layout, pageId)?.bindings.some((b) => b.target.type === "back") ?? false
}

/** Keys of controls with at least one binding on a page. */
export function boundControls(layout: DeviceLayout, pageId: string): Set<string> {
  return new Set(findPage(layout, pageId)?.bindings.map((b) => controlKey(b.control)) ?? [])
}
