// Folder keys as the agent and saving see them (`deckpilot_core::folder_control`,
// docs/data-model.md "Folder controls").
//
// The shared rule: a key on page P is a folder key when one of its bindings
// opens a page F whose folder in the page tree (stored or inferred) is P.
// Only page and control ids decide it, never names, labels or icons. When
// several bindings open folders of P, the press wins, then the click, then
// the first. Dials and touch zones are never folder keys.
//
// A folder key is a core navigation control: exactly one binding, press →
// open F. The editor offers nothing else on it (no gestures, no actions)
// and leaves it out of bulk assignments, pastes and Library drops. What a
// layout from before the rule had on such a key is in `layout.recovered`:
// never run, moved to a free key or discarded from the key's Inspector.

import { keyTimings, setKeyTimings } from "@/components/gesture-editor/model"
import { systemBackControl } from "./folder-back"
import { page as findPage, pageParents, sameControl } from "./layout-model"
import type {
  ControlBinding,
  ControlId,
  DeviceLayout,
  DeviceModel,
  RecoveredBindings,
} from "./types"

/** The trigger of a folder key's only binding (`FOLDER_CONTROL_TRIGGER`). */
export const FOLDER_CONTROL_TRIGGER = { type: "press" } as const

const RANK: Partial<Record<string, number>> = { press: 0, click: 1 }

/**
 * The folder `control` opens when it is a folder key of `pageId`
 * (`DeviceLayout::folder_control`); `undefined` otherwise.
 */
export function folderControlOf(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  parents: Map<string, string> = pageParents(layout),
): string | undefined {
  if (control.kind !== "key") return undefined
  const p = findPage(layout, pageId)
  if (!p) return undefined
  let best: { rank: number; folder: string } | undefined
  for (const binding of p.bindings) {
    const target = binding.target
    if (
      !sameControl(binding.control, control) ||
      target.type !== "openPage" ||
      target.page === pageId ||
      parents.get(target.page) !== pageId
    ) {
      continue
    }
    const rank = RANK[binding.trigger.type] ?? 2
    if (!best || rank < best.rank) best = { rank, folder: target.page }
  }
  return best?.folder
}

/** Whether `control` is a folder key of `pageId`. */
export function isFolderControl(layout: DeviceLayout, pageId: string, control: ControlId): boolean {
  return folderControlOf(layout, pageId, control) !== undefined
}

/** `controls` without the folder keys of `pageId`: what gesture and action edits may change. */
export function withoutFolderControls(
  layout: DeviceLayout,
  pageId: string,
  controls: ControlId[],
): ControlId[] {
  const parents = pageParents(layout)
  return controls.filter((c) => folderControlOf(layout, pageId, c, parents) === undefined)
}

/** Gestures recovered from the folder key `control` of `pageId` (not a kept title). */
function gesturesOf(entry: RecoveredBindings, pageId: string, control: ControlId): boolean {
  return (
    entry.page === pageId && sameControl(entry.control, control) && entry.reason !== "folderLabel"
  )
}

/** The recovered bindings of the folder key `control` of `pageId`. */
export function recoveredOf(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): ControlBinding[] {
  return (layout.recovered ?? [])
    .filter((entry) => gesturesOf(entry, pageId, control))
    .flatMap((entry) => entry.bindings)
}

function others(layout: DeviceLayout, pageId: string, control: ControlId): RecoveredBindings[] {
  return (layout.recovered ?? []).filter((entry) => !gesturesOf(entry, pageId, control))
}

/**
 * The title the folder key `control` of `pageId` had of its own before
 * the folder's name became its one title (schema 13,
 * `DeviceLayout::recovered_folder_label`).
 */
export function recoveredFolderLabel(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): string | undefined {
  for (const entry of layout.recovered ?? []) {
    if (
      entry.reason === "folderLabel" &&
      entry.page === pageId &&
      sameControl(entry.control, control)
    ) {
      const label = entry.bindings.find((b) => b.label)?.label
      if (label) return label
    }
  }
  return undefined
}

/**
 * "Use as folder name" (`adopt`) or "Discard"
 * (`DeviceLayout::resolve_folder_label`): forgets the kept title, with
 * `adopt` after making it the folder's name.
 */
export function resolveFolderLabel(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  adopt: boolean,
): DeviceLayout {
  const label = recoveredFolderLabel(layout, pageId, control)
  if (label === undefined) return layout
  const folder = folderControlOf(layout, pageId, control)
  const kept = (layout.recovered ?? []).filter(
    (entry) =>
      !(
        entry.reason === "folderLabel" &&
        entry.page === pageId &&
        sameControl(entry.control, control)
      ),
  )
  const renamed: DeviceLayout =
    adopt && folder
      ? { ...layout, pages: layout.pages.map((p) => (p.id === folder ? { ...p, name: label } : p)) }
      : layout
  return withRecovered(renamed, kept)
}

/**
 * The title a key shows when it opens a folder: the folder page's name, its
 * one title (the Rust resolver is `device_runtime`'s face of `OpenPage`).
 * `undefined` for any other key.
 */
export function folderKeyTitle(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): string | undefined {
  const folder = folderControlOf(layout, pageId, control)
  return folder === undefined ? undefined : (findPage(layout, folder)?.name ?? "")
}

function withRecovered(layout: DeviceLayout, recovered: RecoveredBindings[]): DeviceLayout {
  const next: DeviceLayout = { ...layout, recovered }
  // An empty list is not stored (`skip_serializing_if`).
  if (recovered.length === 0) delete next.recovered
  return next
}

/**
 * "Discard" (`DeviceLayout::discard_recovered`): forgets the recovered
 * bindings of the folder key.
 */
export function discardRecovered(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): DeviceLayout {
  return withRecovered(layout, others(layout, pageId, control))
}

/**
 * "Move to a free key" (`DeviceLayout::restore_recovered`): the recovered
 * bindings of the folder key, with their gesture timings, go to the page's
 * first free key with a display (no binding, not the folder's reserved
 * Back) and leave the list. `null` when there is nothing to move or no
 * key is free; nothing changes then.
 */
export function restoreRecovered(
  model: DeviceModel,
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): { layout: DeviceLayout; movedTo: ControlId } | null {
  const entries = (layout.recovered ?? []).filter((entry) => gesturesOf(entry, pageId, control))
  const p = findPage(layout, pageId)
  if (entries.length === 0 || !p) return null
  const keys = model.capabilities.keys
  if (!keys?.image) return null
  const reserved = systemBackControl(model, layout, pageId)
  const free = Array.from({ length: keys.rows * keys.columns }, (_, index) => ({
    kind: "key" as const,
    index,
  })).find(
    (c) =>
      !(reserved && sameControl(reserved, c)) && !p.bindings.some((b) => sameControl(b.control, c)),
  )
  if (!free) return null
  const moved = entries.flatMap((entry) => entry.bindings.map((b) => ({ ...b, control: free })))
  const timings = entries.find((entry) => entry.gestures)?.gestures
  let next: DeviceLayout = {
    ...layout,
    pages: layout.pages.map((page) =>
      page.id === pageId ? { ...page, bindings: [...page.bindings, ...moved] } : page,
    ),
  }
  // The key's own timings go with them; stale ones of the free key go.
  if (timings || keyTimings(next, pageId, free)) {
    next = setKeyTimings(
      next,
      pageId,
      free,
      timings
        ? { doubleClickMs: timings.doubleClickMs, longPressMs: timings.longPressMs }
        : undefined,
    )
  }
  return { layout: withRecovered(next, others(layout, pageId, control)), movedTo: free }
}

/**
 * Whether `control` is a folder key of `pageId` with more than its one
 * binding: what saving refuses (`invalid.folderControl.extraBinding`), so
 * no edit of the editor may leave a key like that.
 */
export function brokenFolderControl(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): boolean {
  if (folderControlOf(layout, pageId, control) === undefined) return false
  const p = findPage(layout, pageId)
  return (p?.bindings.filter((b) => sameControl(b.control, control)).length ?? 0) > 1
}

/**
 * A folder key whose one binding opens its folder on another trigger (a
 * click, a gesture's slot) gets it on the press, as saving requires;
 * anything else is returned as it is.
 */
export function pressFolderControl(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): DeviceLayout {
  if (folderControlOf(layout, pageId, control) === undefined) return layout
  const p = findPage(layout, pageId)
  const own = p?.bindings.filter((b) => sameControl(b.control, control)) ?? []
  const only = own[0]
  if (own.length !== 1 || !only || only.trigger.type === "press") return layout
  return {
    ...layout,
    pages: layout.pages.map((page) =>
      page.id === pageId
        ? {
            ...page,
            bindings: page.bindings.map((b) =>
              b === only ? { ...b, trigger: { ...FOLDER_CONTROL_TRIGGER } } : b,
            ),
          }
        : page,
    ),
  }
}
