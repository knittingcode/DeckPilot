// Folder pages: the protected system Back key and Auto Exit, as the agent
// sees them (`deckpilot_core::folder`, docs/data-model.md "Pages and
// folders").
//
// The shared rule: a page inside a folder whose `back.position` is
// `"reserved"` has its Back on the model's first key with a display (key 0,
// top left on a rectangular grid; never a dial or a touch zone). The agent
// shows and runs that key itself, so the editor never puts anything on it.
// A folder page without the setting, or with `"legacy"`, keeps the
// automatic Back on its first free key, as before.

import { keyTimings, setKeyTimings } from "@/components/gesture-editor/model"
import { bindingsFor, page as findPage, pageParents, sameControl } from "./layout-model"
import type {
  AutoExit,
  BackPosition,
  ControlBinding,
  ControlId,
  DeviceLayout,
  DeviceModel,
  LayoutPage,
} from "./types"

/** Auto Exit takes whole seconds in this range (`AutoExit::MIN/MAX_SECONDS`). */
export const AUTO_EXIT_MIN_SECONDS = 1
export const AUTO_EXIT_MAX_SECONDS = 3600
/** What a new folder stores (`AutoExit::DEFAULT_SECONDS`). */
export const AUTO_EXIT_DEFAULT_SECONDS = 10

/** Auto Exit of a new folder: off, with the default time. */
export const DEFAULT_AUTO_EXIT: AutoExit = { enabled: false, seconds: AUTO_EXIT_DEFAULT_SECONDS }

/** The Back of a new folder page: on the reserved key. */
export const RESERVED_BACK = { position: "reserved" } as const

/** Keys with a display, in the model's order (Rust `capabilities.controls()`). */
function displayKeys(model: DeviceModel): ControlId[] {
  const keys = model.capabilities.keys
  if (!keys?.image) return []
  return Array.from({ length: keys.rows * keys.columns }, (_, index) => ({
    kind: "key" as const,
    index,
  }))
}

/**
 * The control a reserved Back takes on `model`: its first key with a
 * display; `null` for a model without one (`reserved_back_control`).
 */
export function reservedBackControl(model: DeviceModel): ControlId | null {
  return displayKeys(model)[0] ?? null
}

/** Whether `pageId` is inside a folder (a folder page); never the home page. */
export function isFolderPage(layout: DeviceLayout, pageId: string): boolean {
  return pageParents(layout).has(pageId)
}

/**
 * Where the Back of `pageId` is: `null` for a page in no folder, `"legacy"`
 * for a folder page without the setting.
 */
export function backPosition(layout: DeviceLayout, pageId: string): BackPosition | null {
  if (!isFolderPage(layout, pageId)) return null
  return findPage(layout, pageId)?.back?.position ?? "legacy"
}

/** The protected system Back of `pageId`, if it has one on `model`. */
export function systemBackControl(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
): ControlId | null {
  return backPosition(layout, pageId) === "reserved" ? reservedBackControl(model) : null
}

/** Whether `control` is the protected system Back of `pageId`. */
export function isSystemBack(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): boolean {
  const back = systemBackControl(model, layout, pageId)
  return back !== null && sameControl(back, control)
}

/** `controls` without the system Back of `pageId`: what bulk edits may change. */
export function withoutSystemBack(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
): ControlId[] {
  const back = systemBackControl(model, layout, pageId)
  return back ? controls.filter((c) => !sameControl(c, back)) : controls
}

/**
 * The key the agent shows as an automatic Back on a legacy folder page
 * without a way back (`DeviceLayout::automatic_back`): its first free key
 * with a display. `null` elsewhere.
 */
export function legacyAutomaticBack(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
): ControlId | null {
  if (backPosition(layout, pageId) !== "legacy") return null
  const p = findPage(layout, pageId)
  if (!p) return null
  const folders = new Set<string>()
  const parents = pageParents(layout)
  for (let id = parents.get(pageId); id !== undefined && !folders.has(id); id = parents.get(id)) {
    folders.add(id)
  }
  const wayBack = p.bindings.some(
    (b) => b.target.type === "back" || (b.target.type === "openPage" && folders.has(b.target.page)),
  )
  if (wayBack) return null
  return displayKeys(model).find((c) => bindingsFor(layout, pageId, c).length === 0) ?? null
}

function mapPage(
  layout: DeviceLayout,
  pageId: string,
  change: (page: LayoutPage) => LayoutPage,
): DeviceLayout {
  return { ...layout, pages: layout.pages.map((p) => (p.id === pageId ? change(p) : p)) }
}

/**
 * "Move Back to the first key" (`DeviceLayout::move_back_to_reserved`):
 * the reserved key's bindings other than Back, with its gesture timings,
 * move to the first free key with a display; Back bindings on it stay
 * (the system Back shows their label and look). The page gets the
 * reserved Back. `null` when nothing can change: a page in no folder, a
 * model without a key for Back, or a reserved key that runs something with
 * no free key to move it to.
 */
export function moveBackToReservedKey(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
): { layout: DeviceLayout; movedTo: ControlId | null } | null {
  if (!isFolderPage(layout, pageId)) return null
  const reserved = reservedBackControl(model)
  const p = findPage(layout, pageId)
  if (!reserved || !p) return null
  const on = (control: ControlId) => (b: { control: ControlId }) => sameControl(b.control, control)
  const moves = (b: ControlBinding) => on(reserved)(b) && b.target.type !== "back"
  let movedTo: ControlId | null = null
  let next = layout
  if (p.bindings.some(moves)) {
    const free = displayKeys(model).find(
      (c) => !sameControl(c, reserved) && !p.bindings.some(on(c)),
    )
    if (!free) return null
    movedTo = free
    const timings = keyTimings(layout, pageId, reserved)
    next = mapPage(next, pageId, (page) => ({
      ...page,
      bindings: page.bindings.map((b) => (moves(b) ? { ...b, control: free } : b)),
    }))
    next = setKeyTimings(
      next,
      pageId,
      free,
      timings
        ? { doubleClickMs: timings.doubleClickMs, longPressMs: timings.longPressMs }
        : undefined,
    )
    next = setKeyTimings(next, pageId, reserved, undefined)
  }
  next = mapPage(next, pageId, (page) => ({ ...page, back: { ...RESERVED_BACK } }))
  return { layout: next, movedTo }
}

/** The Auto Exit of a page as the editor shows it: stored, else off with the default time. */
export function autoExitOf(page: LayoutPage | undefined): AutoExit {
  return page?.autoExit ?? DEFAULT_AUTO_EXIT
}

/** Stores the Auto Exit of a page. */
export function setAutoExit(
  layout: DeviceLayout,
  pageId: string,
  autoExit: AutoExit,
): DeviceLayout {
  return mapPage(layout, pageId, (page) => ({ ...page, autoExit: { ...autoExit } }))
}

/** Why a typed Auto Exit time is refused, as a translation key; `null`: it is valid. */
export function checkAutoExitSeconds(
  value: string | number,
): "editor:folderSettings.invalid.whole" | "editor:folderSettings.invalid.range" | null {
  const text = String(value).trim()
  if (!/^\d+$/.test(text)) return "editor:folderSettings.invalid.whole"
  const seconds = Number(text)
  if (seconds < AUTO_EXIT_MIN_SECONDS || seconds > AUTO_EXIT_MAX_SECONDS) {
    return "editor:folderSettings.invalid.range"
  }
  return null
}
