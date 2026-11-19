// Keys that move a device between the layouts of a profile: the `deck`
// module's Switch layout and Back to previous layout (docs/data-model.md,
// "Switching layouts").

import { controlKey, sameControl, sameTrigger } from "./layout-model"
import type {
  ActionInstance,
  ActionParameters,
  ControlBinding,
  DeviceInfo,
  DeviceLayout,
  ParameterValue,
  Usage,
} from "./types"

export const SWITCH_LAYOUT = "deck.switch-layout"
export const PREVIOUS_LAYOUT = "deck.previous-layout"

/** What Back to previous layout does after a switch. */
export type LayoutHistory = "push" | "replace" | "clear"

export const HISTORY_CHOICES: { value: LayoutHistory; label: string; detail: string }[] = [
  {
    value: "push",
    label: "Back returns here",
    detail: "Back to previous layout returns to the layout this key left.",
  },
  {
    value: "replace",
    label: "Back skips this layout",
    detail: "Back to previous layout returns to where the device was before this layout.",
  },
  {
    value: "clear",
    label: "Clear the way back",
    detail: "Back to previous layout has nowhere to go afterwards.",
  },
]

/** Where a Switch layout action goes: the layout, a page (`null`: home page) and Back. */
export interface LayoutSwitch {
  layout: string
  page: string | null
  history: LayoutHistory
}

function text(value: ParameterValue | undefined): string {
  return value && (value.type === "text" || value.type === "choice") ? value.value.trim() : ""
}

/** The destination of a Switch layout action; overrides of a binding come first. */
export function layoutSwitchOf(
  action: Pick<ActionInstance, "parameters">,
  overrides: ActionParameters = {},
): LayoutSwitch {
  const value = (key: string) => text(overrides[key] ?? action.parameters[key])
  const history = value("history")
  return {
    layout: value("layout"),
    page: value("page") || null,
    history: history === "replace" || history === "clear" ? history : "push",
  }
}

/** The parameters of a Switch layout action to `destination`; the home page is left out. */
export function layoutSwitchParameters(destination: LayoutSwitch): ActionParameters {
  return {
    layout: { type: "text", value: destination.layout },
    ...(destination.page ? { page: { type: "text", value: destination.page } } : {}),
    history: { type: "choice", value: destination.history },
  }
}

function sameSwitch(a: LayoutSwitch, b: LayoutSwitch): boolean {
  return a.layout === b.layout && a.page === b.page && a.history === b.history
}

/**
 * A Switch layout action of the profile that goes exactly to
 * `destination` and has no label of its own: reused instead of making
 * another one.
 */
export function existingLayoutSwitch(
  actions: ActionInstance[],
  destination: LayoutSwitch,
): ActionInstance | undefined {
  return actions.find(
    (action) =>
      action.actionType === SWITCH_LAYOUT &&
      !action.label &&
      sameSwitch(layoutSwitchOf(action), destination),
  )
}

/** The profile's layouts a Switch layout key of `model` can show, in the profile's order. */
export function switchableLayouts(layouts: DeviceLayout[], model: string): DeviceLayout[] {
  return layouts.filter((l) => l.model === model)
}

/**
 * Whether an action is used anywhere but by `binding` of `pageId` in
 * `draft`: other keys of the layout being edited (as edited), keys of the
 * profile's other layouts and workflow steps (as saved).
 */
export function usedElsewhere(
  actionId: string,
  draft: DeviceLayout,
  pageId: string,
  binding: Pick<ControlBinding, "control" | "trigger">,
  usages: Record<string, Usage[]>,
): boolean {
  const saved = (usages[actionId] ?? []).some(
    (usage) => usage.type !== "layoutBinding" || usage.layout !== draft.id,
  )
  if (saved) return true
  return draft.pages.some((page) =>
    page.bindings.some(
      (b) =>
        b.target.type === "action" &&
        b.target.action === actionId &&
        !(
          page.id === pageId &&
          sameControl(b.control, binding.control) &&
          sameTrigger(b.trigger, binding.trigger)
        ),
    ),
  )
}

/** The layout a device shows, from the agent's status; `null` when it shows none. */
export function shownLayout(device: DeviceInfo | null | undefined): string | null {
  if (!device || device.state !== "connected") return null
  const { layout } = device.layout as { layout?: string }
  return layout ?? null
}

/**
 * The controls of `bindings` that run Back to previous layout while
 * `device` shows `layoutId` with nothing to go back to: on the device they
 * say "No previous layout", so the preview shows them unavailable too.
 * Empty unless the device shows that layout (in the profile being edited).
 */
export function backUnavailable(
  bindings: ControlBinding[],
  actions: ActionInstance[],
  device: DeviceInfo | null | undefined,
  layoutId: string,
): Set<string> {
  const keys = new Set<string>()
  if (shownLayout(device) !== layoutId || device?.previousLayout) return keys
  for (const binding of bindings) {
    if (binding.target.type !== "action") continue
    if (!["press", "click"].includes(binding.trigger.type)) continue
    const id = binding.target.action
    if (actions.find((a) => a.id === id)?.actionType === PREVIOUS_LAYOUT) {
      keys.add(controlKey(binding.control))
    }
  }
  return keys
}
