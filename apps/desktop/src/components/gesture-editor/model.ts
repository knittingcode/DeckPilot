// Pure logic of the gesture editor (docs/gestures-and-work-session.md,
// sections 2.2, 3 and D1/D2). A control is in raw mode (press/release, fired
// at once), in gesture mode (click, double click, long press) or empty.

import { bindingsFor, controlLabel, isGestureTrigger, sameControl } from "@/lib/layout-model"
import type {
  BindingTarget,
  ControlBinding,
  ControlId,
  DeviceLayout,
  Gesture,
  GestureTimings,
  KeyGestureSettings,
  LayoutPage,
  Trigger,
} from "@/lib/types"

export const GESTURES: Gesture[] = ["click", "doubleClick", "longPress"]

export const GESTURE_LABELS: Record<Gesture, string> = {
  click: "Single click",
  doubleClick: "Double click",
  longPress: "Long press",
}

/** Built-in defaults (`GestureTimings::default`), used until the setting is read. */
export const DEFAULT_TIMINGS: GestureTimings = { doubleClickMs: 300, longPressMs: 700 }

export type TimingField = keyof GestureTimings

/** Allowed ranges (`DOUBLE_CLICK_RANGE_MS`, `LONG_PRESS_RANGE_MS`). */
export const TIMING_LIMITS: Record<TimingField, { min: number; max: number; label: string }> = {
  doubleClickMs: { min: 150, max: 1000, label: "Double-click window" },
  longPressMs: { min: 300, max: 5000, label: "Long-press time" },
}

export type KeyMode = "empty" | "raw" | "gesture"

/** How the control's bindings on the page make it react to presses. */
export function keyMode(layout: DeviceLayout, pageId: string, control: ControlId): KeyMode {
  const bindings = pressBindings(layout, pageId, control)
  if (bindings.length === 0) return "empty"
  return bindings.some((b) => isGestureTrigger(b.trigger)) ? "gesture" : "raw"
}

/** Bindings of the control that react to pressing it (not dial turns or touches). */
function pressBindings(layout: DeviceLayout, pageId: string, control: ControlId) {
  return bindingsFor(layout, pageId, control).filter(
    (b) =>
      b.trigger.type === "press" || b.trigger.type === "release" || isGestureTrigger(b.trigger),
  )
}

/**
 * The trigger that stands for `gesture` on the control. In raw mode, and on
 * an empty control, a single click is the raw press (D2: immediate, as
 * before); everything else is the gesture trigger itself.
 */
export function triggerFor(mode: KeyMode, gesture: Gesture): Trigger {
  if (gesture === "click" && mode !== "gesture") return { type: "press" }
  return { type: gesture }
}

export function gestureBinding(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  gesture: Gesture,
): ControlBinding | undefined {
  const trigger = triggerFor(keyMode(layout, pageId, control), gesture)
  return bindingsFor(layout, pageId, control).find((b) => b.trigger.type === trigger.type)
}

/** Whether binding `gesture` first needs the raw press turned into a click (D2). */
export function needsConversion(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  gesture: Gesture,
): boolean {
  return gesture !== "click" && keyMode(layout, pageId, control) === "raw"
}

function mapPage(
  layout: DeviceLayout,
  pageId: string,
  change: (page: LayoutPage) => LayoutPage,
): DeviceLayout {
  return { ...layout, pages: layout.pages.map((p) => (p.id === pageId ? change(p) : p)) }
}

/**
 * Turns a raw control into a gesture control: its press binding becomes the
 * click binding (target, overrides, label and look kept). A release binding
 * cannot stay next to gestures, so it is removed and returned.
 */
export function convertToGestures(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): { layout: DeviceLayout; removed: ControlBinding[] } {
  const removed: ControlBinding[] = []
  const next = mapPage(layout, pageId, (page) => ({
    ...page,
    bindings: page.bindings.flatMap((b) => {
      if (!sameControl(b.control, control)) return [b]
      if (b.trigger.type === "press") return [{ ...b, trigger: { type: "click" } }]
      if (b.trigger.type === "release") {
        removed.push(b)
        return []
      }
      return [b]
    }),
  }))
  return { layout: next, removed }
}

/** Same target, ignoring the overrides of an action. */
export function sameTarget(a: BindingTarget | undefined, b: BindingTarget | undefined): boolean {
  if (!a || !b) return a === b
  if (a.type === "action" && b.type === "action") return a.action === b.action
  return JSON.stringify(a) === JSON.stringify(b)
}

/**
 * Sets (or with `null` removes) the binding of one gesture. The binding
 * keeps its label and look; overrides are kept while the action stays the
 * same and dropped when it changes (they belong to the action's type).
 * Converting a raw control is the caller's job (`needsConversion`).
 */
export function setGesture(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  gesture: Gesture,
  target: BindingTarget | null,
): DeviceLayout {
  const trigger = triggerFor(keyMode(layout, pageId, control), gesture)
  return mapPage(layout, pageId, (page) => {
    const index = page.bindings.findIndex(
      (b) => sameControl(b.control, control) && b.trigger.type === trigger.type,
    )
    const current = index >= 0 ? page.bindings[index] : undefined
    if (!target) {
      return { ...page, bindings: page.bindings.filter((_, i) => i !== index) }
    }
    const binding: ControlBinding = current
      ? { ...current, target: sameTarget(current.target, target) ? current.target : target }
      : { control, trigger, target, label: null }
    const bindings = [...page.bindings]
    if (index >= 0) bindings[index] = binding
    else bindings.push(binding)
    return { ...page, bindings }
  })
}

/** Replaces a binding of the control with the same trigger (label, overrides). */
export function updateBinding(
  layout: DeviceLayout,
  pageId: string,
  binding: ControlBinding,
): DeviceLayout {
  return mapPage(layout, pageId, (page) => ({
    ...page,
    bindings: page.bindings.map((b) =>
      sameControl(b.control, binding.control) && b.trigger.type === binding.trigger.type
        ? binding
        : b,
    ),
  }))
}

// ---------------------------------------------------------------------------
// When a single click fires (D1)

export type ClickTiming = "immediate" | "onRelease" | "afterWindow"

export function clickTiming(layout: DeviceLayout, pageId: string, control: ControlId): ClickTiming {
  const has = (gesture: Gesture) =>
    bindingsFor(layout, pageId, control).some((b) => b.trigger.type === gesture)
  if (keyMode(layout, pageId, control) !== "gesture") return "immediate"
  if (has("doubleClick")) return "afterWindow"
  if (has("longPress")) return "onRelease"
  return "immediate"
}

// ---------------------------------------------------------------------------
// Per-key timings (`LayoutPage.gestures`; only overrides are stored)

export function keyTimings(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
): KeyGestureSettings | undefined {
  return layout.pages
    .find((p) => p.id === pageId)
    ?.gestures?.find((g) => sameControl(g.control, control))
}

/** Replaces the control's timings; no values removes the entry. */
export function setKeyTimings(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  timings: Omit<KeyGestureSettings, "control"> | undefined,
): DeviceLayout {
  return mapPage(layout, pageId, (page) => {
    const others = (page.gestures ?? []).filter((g) => !sameControl(g.control, control))
    const entry: KeyGestureSettings = { control }
    if (timings?.doubleClickMs !== undefined) entry.doubleClickMs = timings.doubleClickMs
    if (timings?.longPressMs !== undefined) entry.longPressMs = timings.longPressMs
    const set = entry.doubleClickMs !== undefined || entry.longPressMs !== undefined
    const gestures = set ? [...others, entry] : others
    if (gestures.length > 0) return { ...page, gestures }
    // Keep pages that never had timings exactly as they were.
    if (page.gestures === undefined) return page
    return { ...page, gestures: [] }
  })
}

/** Sets or (with `undefined`) clears one timing of the control. */
export function setKeyTiming(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  field: TimingField,
  value: number | undefined,
): DeviceLayout {
  const current = keyTimings(layout, pageId, control)
  return setKeyTimings(layout, pageId, control, {
    doubleClickMs: current?.doubleClickMs,
    longPressMs: current?.longPressMs,
    [field]: value,
  })
}

/** Why a typed timing cannot be used; `null` when it can (empty: default). */
export function timingError(field: TimingField, text: string): string | null {
  const { min, max, label } = TIMING_LIMITS[field]
  if (text.trim() === "") return null
  const value = Number(text)
  if (!Number.isInteger(value)) return `${label} must be a whole number of milliseconds.`
  if (value < min || value > max) return `${label} must be between ${min} and ${max} ms.`
  return null
}

// ---------------------------------------------------------------------------
// Backend messages

function backendName(control: ControlId): string {
  const kind = control.kind === "touchZone" ? "touch zone" : control.kind
  return `${kind} ${control.index}`
}

/**
 * The messages of a failed save that concern one control of a page, made
 * readable: without the page prefix, the control named as in the editor.
 * Layout issues read "page `Name`: key 3 …" with zero-based indices.
 */
export function controlIssues(
  messages: string[],
  page: Pick<LayoutPage, "name">,
  control: ControlId,
): string[] {
  const prefix = `page \`${page.name}\`: `
  const name = new RegExp(`\\b${backendName(control)}(?!\\d)`)
  return messages
    .filter((m) => m.startsWith(prefix) && name.test(m.slice(prefix.length)))
    .map((m) => m.slice(prefix.length).replace(name, controlLabel(control)))
}
