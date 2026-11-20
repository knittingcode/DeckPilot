// What a double-click on a control of the device preview does to the
// editor's view (docs/editor-navigation-and-simulation.md, section 1):
// resolved from data only — the binding's target, the action type's
// declared `editorNavigation` from the catalog, or the preview face's
// `navigation` (the agent's automatic Back key) — never from labels, and
// never by running anything.

import { ancestors, bindingsFor, page as findPage, topLevelPages } from "@/lib/layout-model"
import type {
  ActionInstance,
  ActionParameters,
  ActionType,
  ControlBinding,
  ControlId,
  ControlNavigation,
  DeviceLayout,
  EditorNavigation,
  NavigationOperation,
  ParameterValue,
} from "@/lib/types"

/** What the editor does for a double-click. */
export type EditorMove =
  /** Shows a page, remembering where it was opened from (a folder). */
  | { type: "enter"; page: string }
  /** Back: the page it was opened from, else the folder the page is in. */
  | { type: "back" }
  /** The next or previous top-level page. */
  | { type: "page"; direction: "next" | "previous"; wrapAround: boolean }
  /** Another layout of the profile (and a page of it). */
  | { type: "layout"; layout: string; page: string | null }
  /** The layout the editor showed before (its own history). */
  | { type: "layoutBack" }

const OPERATIONS: readonly NavigationOperation[] = [
  "folder.open",
  "folder.back",
  "page.next",
  "page.previous",
  "layout.switch",
  "layout.back",
]

/** The parameter `wrapAround` of the page actions (`deck.next-page`, `deck.previous-page`). */
export const WRAP_AROUND = "wrapAround"

function isTextParameter(type: ActionType, key: string | undefined): boolean {
  return key !== undefined && type.parameters.some((p) => p.key === key && p.kind.type === "text")
}

/**
 * The type's `editorNavigation` when it checks out, else `null`: an
 * operation of the allowlist, `targetParameter` exactly for `folder.open`
 * and `layout.switch` and naming a text parameter of the action, and
 * `pageParameter` only for `layout.switch` (as `EditorNavigation::check`
 * of the core). The backend checks plugin declarations already; this is
 * the editor's own guard, so a declaration that does not check out never
 * navigates.
 */
export function checkedNavigation(type: ActionType | undefined): EditorNavigation | null {
  const navigation = type?.editorNavigation
  if (!type || !navigation || !OPERATIONS.includes(navigation.operation)) return null
  const { operation, targetParameter, pageParameter } = navigation
  const needsTarget = operation === "folder.open" || operation === "layout.switch"
  if (needsTarget !== (targetParameter !== undefined)) return null
  if (needsTarget && !isTextParameter(type, targetParameter)) return null
  if (pageParameter !== undefined) {
    if (operation !== "layout.switch" || !isTextParameter(type, pageParameter)) return null
  }
  return navigation
}

function textOf(value: ParameterValue | undefined): string {
  return value && (value.type === "text" || value.type === "choice") ? value.value.trim() : ""
}

/** A parameter of an action as a binding runs it: its overrides first. */
function parameter(
  action: ActionInstance,
  overrides: ActionParameters | undefined,
  key: string,
): ParameterValue | undefined {
  return overrides?.[key] ?? action.parameters[key]
}

/**
 * Whether a page action continues at the other end: its `wrapAround`
 * parameter, else the type's default for it; without either, no wraparound.
 */
function wrapAroundOf(
  type: ActionType,
  action: ActionInstance,
  overrides: ActionParameters | undefined,
): boolean {
  const value = parameter(action, overrides, WRAP_AROUND)
  if (value?.type === "boolean") return value.value
  const fallback = type.parameters.find((p) => p.key === WRAP_AROUND)?.default
  return fallback?.type === "boolean" ? fallback.value : false
}

/** What `binding` does to the editor's view; `null` when it does nothing there. */
export function moveOfBinding(
  layout: DeviceLayout,
  binding: ControlBinding,
  actions: ActionInstance[],
  typeOf: (id: string) => ActionType | undefined,
): EditorMove | null {
  const { target } = binding
  if (target.type === "openPage")
    return findPage(layout, target.page) ? { type: "enter", page: target.page } : null
  if (target.type === "back") return { type: "back" }
  if (target.type !== "action") return null
  const action = actions.find((a) => a.id === target.action)
  const type = action ? typeOf(action.actionType) : undefined
  const navigation = checkedNavigation(type)
  if (!action || !type || !navigation) return null
  const read = (key: string | undefined) =>
    key ? textOf(parameter(action, target.overrides, key)) : ""
  switch (navigation.operation) {
    case "folder.open": {
      const page = read(navigation.targetParameter)
      return page && findPage(layout, page) ? { type: "enter", page } : null
    }
    case "folder.back":
      return { type: "back" }
    case "page.next":
    case "page.previous":
      return {
        type: "page",
        direction: navigation.operation === "page.next" ? "next" : "previous",
        wrapAround: wrapAroundOf(type, action, target.overrides),
      }
    case "layout.switch": {
      const to = read(navigation.targetParameter)
      return to
        ? { type: "layout", layout: to, page: read(navigation.pageParameter) || null }
        : null
    }
    case "layout.back":
      return { type: "layoutBack" }
  }
}

/** What a preview face's resolved navigation does (the automatic Back key, mainly). */
export function moveOfFace(
  layout: DeviceLayout,
  navigation: ControlNavigation | undefined,
): EditorMove | null {
  if (!navigation || !OPERATIONS.includes(navigation.operation)) return null
  switch (navigation.operation) {
    case "folder.open":
      return navigation.page && findPage(layout, navigation.page)
        ? { type: "enter", page: navigation.page }
        : null
    case "folder.back":
      return { type: "back" }
    case "page.next":
    case "page.previous":
      // The face does not say whether it wraps: it does not, as the page navigation.
      return {
        type: "page",
        direction: navigation.operation === "page.next" ? "next" : "previous",
        wrapAround: false,
      }
    case "layout.switch":
      return navigation.layout
        ? { type: "layout", layout: navigation.layout, page: navigation.page ?? null }
        : null
    case "layout.back":
      return { type: "layoutBack" }
  }
}

/**
 * What a double-click on `control` of `pageId` does: of its bindings that
 * navigate, the press binding, then the click binding, then the first (as
 * `folderOf`). A control without bindings may still be the agent's
 * automatic Back key: its preview face says so (`face`).
 */
export function controlMove(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  actions: ActionInstance[],
  typeOf: (id: string) => ActionType | undefined,
  face?: ControlNavigation,
): EditorMove | null {
  const bindings = bindingsFor(layout, pageId, control)
  if (bindings.length === 0) return moveOfFace(layout, face)
  const moves = bindings.flatMap((binding) => {
    const move = moveOfBinding(layout, binding, actions, typeOf)
    return move ? [{ binding, move }] : []
  })
  const preferred =
    moves.find((m) => m.binding.trigger.type === "press") ??
    moves.find((m) => m.binding.trigger.type === "click") ??
    moves[0]
  return preferred?.move ?? null
}

/** Whether Back has somewhere to go from `pageId`: a trail, or a folder it is in. */
export function canGoBack(layout: DeviceLayout, pageId: string, trail: string[]): boolean {
  return trail.length > 0 || ancestors(layout, pageId).length > 0
}

/**
 * The top-level page the device's Next/Previous page shows from `pageId`
 * (a page in a folder counts as its top-level page); `undefined` at an end
 * without wraparound, or with fewer than two pages.
 */
export function adjacentPage(
  layout: DeviceLayout,
  pageId: string,
  direction: "next" | "previous",
  wrapAround: boolean,
): string | undefined {
  const pages = topLevelPages(layout)
  if (pages.length < 2) return undefined
  const top = ancestors(layout, pageId).at(-1) ?? pageId
  const index = pages.indexOf(top)
  if (index < 0) return undefined
  const step = direction === "next" ? 1 : -1
  const next = index + step
  if (next >= 0 && next < pages.length) return pages[next]
  return wrapAround ? pages[(next + pages.length) % pages.length] : undefined
}

/*
 * The editor's own layout history, for `layout.back`: the layouts it left
 * by a double-click on a Switch layout key, per profile and model, most
 * recent last. Not the device's history (that is the agent's); it lasts
 * as long as the window.
 */
const layoutHistory = new Map<string, string[]>()

function historyKey(profileId: string, model: string) {
  return `${profileId}:${model}`
}

/** Remembers that the editor left `layout` for another one. */
export function pushLayoutHistory(profileId: string, model: string, layout: string) {
  const key = historyKey(profileId, model)
  const stack = (layoutHistory.get(key) ?? []).filter((id) => id !== layout)
  layoutHistory.set(key, [...stack, layout].slice(-32))
}

/** The layout the editor showed before, without taking it; `undefined`: none. */
export function peekLayoutHistory(profileId: string, model: string): string | undefined {
  return layoutHistory.get(historyKey(profileId, model))?.at(-1)
}

/** Takes the layout the editor showed before. */
export function popLayoutHistory(profileId: string, model: string): string | undefined {
  const key = historyKey(profileId, model)
  const stack = layoutHistory.get(key) ?? []
  const last = stack.at(-1)
  layoutHistory.set(key, stack.slice(0, -1))
  return last
}

/** Forgets every layout history (tests). */
export function resetLayoutHistory() {
  layoutHistory.clear()
}
