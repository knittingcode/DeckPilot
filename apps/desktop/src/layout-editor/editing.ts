// Pure editing logic of the layout editor: selection, keyboard navigation,
// clipboard, move/swap, duplicate and undo history. Everything is derived
// from the device model's controls; no geometry is hard-coded.

import { keyTimings, setKeyTimings } from "@/components/gesture-editor/model"
import {
  bindableTriggers,
  bindingsFor,
  controlKey,
  controlsOf,
  defaultTrigger,
  page as findPage,
  pageParents,
  sameControl,
  sameTrigger,
  setBinding,
} from "@/lib/layout-model"
import type {
  BindingTarget,
  ControlBinding,
  ControlId,
  DeviceLayout,
  DeviceModel,
  KeyGestureSettings,
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

/** A control's gesture timings without the control; they travel with its bindings. */
type ControlTimings = Omit<KeyGestureSettings, "control"> | undefined

function timingsOf(layout: DeviceLayout, pageId: string, control: ControlId): ControlTimings {
  const own = keyTimings(layout, pageId, control)
  if (!own) return undefined
  const { doubleClickMs, longPressMs } = own
  return { doubleClickMs, longPressMs }
}

/** A control's bindings without the control; label and appearance travel along. */
function strip(bindings: ControlBinding[]): ControlBindings {
  return bindings.map(({ trigger, target, label, appearance }) => ({
    trigger,
    target,
    label: label ?? null,
    ...(appearance ? { appearance } : {}),
  }))
}

/** Removes every binding (and the gesture timings) of the given controls on a page. */
export function clearControls(
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
): DeviceLayout {
  const cleared = (control: ControlId) => controls.some((c) => sameControl(c, control))
  return {
    ...layout,
    pages: layout.pages.map((p) =>
      p.id === pageId
        ? {
            ...p,
            bindings: p.bindings.filter((b) => !cleared(b.control)),
            ...(p.gestures ? { gestures: p.gestures.filter((g) => !cleared(g.control)) } : {}),
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
  timings?: ControlTimings,
): { layout: DeviceLayout; skipped: number } {
  const supported = bindableTriggers(model, control)
  let next = setKeyTimings(clearControls(layout, pageId, [control]), pageId, control, timings)
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

/** The page a control opens with its press, click (or first) binding, if any. */
export function folderOf(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): string | undefined {
  const opening = bindingsFor(layout, pageId, control).filter(
    (b): b is ControlBinding & { target: { type: "openPage"; page: string } } =>
      b.target.type === "openPage",
  )
  const preferred =
    opening.find((b) => b.trigger.type === "press") ??
    opening.find((b) => b.trigger.type === "click") ??
    opening[0]
  return preferred && findPage(layout, preferred.target.page) ? preferred.target.page : undefined
}

// ---------------------------------------------------------------------------
// Clipboard

export interface Clipboard {
  /** Copied controls with their position relative to the first one. */
  entries: {
    offset: number
    control: ControlId
    bindings: ControlBindings
    timings?: ControlTimings
  }[]
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
      timings: timingsOf(layout, pageId, control),
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
  const placements: { control: ControlId; bindings: ControlBindings; timings: ControlTimings }[] =
    []
  let skipped = 0
  if (single && sortedTargets.length > 1) {
    for (const control of sortedTargets) {
      placements.push({ control, bindings: single.bindings, timings: single.timings })
    }
  } else if (start) {
    const base = order.findIndex((c) => sameControl(c, start))
    for (const entry of clipboard.entries) {
      const control = order[base + entry.offset]
      if (control) placements.push({ control, bindings: entry.bindings, timings: entry.timings })
      else skipped += entry.bindings.length
    }
  }
  let next = layout
  for (const { control, bindings, timings } of placements) {
    const placed = placeBindings(model, next, pageId, control, bindings, timings)
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
    const supported = bindableTriggers(model, control)
    return bindings.every((b) => supported.some((t) => sameTrigger(t, b.trigger)))
  }
  if (!fits(to, source) || !fits(from, dest)) return null
  const sourceTimings = timingsOf(layout, pageId, from)
  const destTimings = timingsOf(layout, pageId, to)
  let next = clearControls(layout, pageId, [from, to])
  next = setKeyTimings(next, pageId, to, sourceTimings)
  next = setKeyTimings(next, pageId, from, destTimings)
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
    next = placeBindings(
      model,
      next,
      pageId,
      free,
      bindings,
      timingsOf(next, pageId, source),
    ).layout
    created.push(free)
  }
  return { layout: next, controls: created }
}

/**
 * What is being dragged: a library entry, the bindings of a control,
 * "New folder" (a folder that does not exist yet; see `createFolder`) or an
 * action type of the catalog (an action that may not exist yet; see
 * `catalog.ts`).
 */
export type DragSource =
  | { type: "target"; target: BindingTarget }
  | { type: "control"; control: ControlId }
  | { type: "newFolder" }
  | { type: "actionType"; actionType: string }

/**
 * Drops a library entry (assigned to the first free trigger) or a control
 * (moved, swapping with the destination). `null` when the drop is refused.
 * A new folder needs an id of its own (the caller makes it with
 * `createFolder`) and an action type an action: the caller creates or
 * finds it and drops that.
 */
export function dropOnControl(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  source: DragSource,
  over: ControlId,
): DeviceLayout | null {
  if (source.type === "newFolder" || source.type === "actionType") return null
  if (source.type === "control") return moveControl(model, layout, pageId, source.control, over)
  if (!targetFits(layout, pageId, source.target)) return null
  return assignTarget(model, layout, pageId, [over], source.target)
}

// ---------------------------------------------------------------------------
// Folders

/** A page name no page of the layout has: "Folder", "Folder 2", … */
export function freeFolderName(layout: DeviceLayout): string {
  const taken = new Set(layout.pages.map((p) => p.name.trim().toLowerCase()))
  if (!taken.has("folder")) return "Folder"
  let n = 2
  while (taken.has(`folder ${n}`)) n += 1
  return `Folder ${n}`
}

/** Whether a control has no binding on the page. */
export function isFree(layout: DeviceLayout, pageId: string, control: ControlId): boolean {
  return bindingsFor(layout, pageId, control).length === 0
}

/**
 * Creates a folder inside the page `pageId` and makes `control` open it:
 * a new page with the id `id`, stored as a page of `pageId` (so Back
 * returns there), and one binding on the control's first trigger. Other
 * keys and pages are untouched. `null` when the control already does
 * something: a bound control is never overwritten.
 */
export function createFolder(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  id: string,
): DeviceLayout | null {
  if (!findPage(layout, pageId) || findPage(layout, id) || !isFree(layout, pageId, control)) {
    return null
  }
  const trigger = defaultTrigger(model, layout, pageId, control)
  if (!trigger) return null
  const withPage: DeviceLayout = {
    ...layout,
    pages: [...layout.pages, { id, name: freeFolderName(layout), parent: pageId, bindings: [] }],
  }
  return setBinding(withPage, pageId, {
    control,
    trigger,
    target: { type: "openPage", page: id },
    label: null,
  })
}

/** What deleting a page takes with it, for the confirmation. */
export interface PageDeletion {
  name: string
  /** Controls of the page itself that do something. */
  controls: number
  /** Controls on other pages that open it; they are cleared of it. */
  openers: number
  /** Pages stored inside it; they move up to its own folder. */
  inside: string[]
  /** Where those pages go: the page's folder, or `null` for the top level. */
  folder: string | null
}

export function pageDeletion(layout: DeviceLayout, pageId: string): PageDeletion | null {
  const target = findPage(layout, pageId)
  if (!target || pageId === layout.homePage) return null
  const folder = pageParents(layout).get(pageId)
  return {
    name: target.name,
    controls: new Set(target.bindings.map((b) => controlKey(b.control))).size,
    openers: layout.pages
      .filter((p) => p.id !== pageId)
      .flatMap((p) => p.bindings)
      .filter((b) => b.target.type === "openPage" && b.target.page === pageId).length,
    inside: layout.pages.filter((p) => p.parent === pageId).map((p) => p.name),
    folder: folder ? (findPage(layout, folder)?.name ?? null) : null,
  }
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

/**
 * The page "back" returns to from `pageId` when it was not opened from a
 * folder key: its folder in the page tree, else home.
 */
export function parentPage(layout: DeviceLayout, pageId: string): string {
  if (pageId === layout.homePage) return pageId
  return pageParents(layout).get(pageId) ?? layout.homePage
}

/** Keys of controls with at least one binding on a page. */
export function boundControls(layout: DeviceLayout, pageId: string): Set<string> {
  return new Set(findPage(layout, pageId)?.bindings.map((b) => controlKey(b.control)) ?? [])
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`

/** What goes with a deleted page, in sentences, for the confirmation. */
export function deletionNotes(deletion: PageDeletion): string[] {
  const notes: string[] = []
  const { controls, openers, inside } = deletion
  if (controls > 0) {
    notes.push(
      `${plural(controls, "control")} on it ${controls === 1 ? "is" : "are"} deleted with it.`,
    )
  }
  if (openers > 0) {
    notes.push(
      `${plural(openers, "control")} on other pages that ${openers === 1 ? "opens" : "open"} it will do nothing afterwards.`,
    )
  }
  if (inside.length > 0) {
    const names = inside.map((name) => `“${name || "Untitled"}”`).join(", ")
    const where = deletion.folder ? `to “${deletion.folder}”` : "to the top level"
    notes.push(
      inside.length === 1
        ? `The page inside it (${names}) is kept and moves ${where}.`
        : `The pages inside it (${names}) are kept and move ${where}.`,
    )
  }
  if (notes.length === 0) notes.push("The page is empty and nothing opens it.")
  return notes
}
