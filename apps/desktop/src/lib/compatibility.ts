// Whether an action can be bound to a control, and the reason in words,
// for the device editor (docs/device-controls.md). The backend decides:
// `save_layout` refuses a new incompatible binding and `preview_layout`
// reports the stored ones. This mirrors its rules only so the editor can
// disable what would be refused and explain why before anything is saved.

import type {
  ActionControls,
  ActionType,
  ControlId,
  DeviceModel,
  IncompatibilityReason,
  InputCapability,
  Trigger,
} from "./types"

export type ControlKind = ControlId["kind"]

/** The control kinds in the order the backend lists them. */
const KINDS: ControlKind[] = ["key", "encoder", "touchZone"]

/** Inputs in the backend's order (`InputCapability`'s declaration order). */
const INPUTS: InputCapability[] = [
  "press",
  "release",
  "rotate-clockwise",
  "rotate-counter-clockwise",
  "tap",
  "long-touch",
  "swipe",
  "click",
  "double-click",
  "long-press",
]

/** The declaration of an action without one: keys only (`ActionControls::legacy`). */
export const LEGACY_CONTROLS: ActionControls = {
  key: ["press", "release", "click", "double-click", "long-press"],
}

/** What an action can be bound to: its declaration, else the legacy default. */
export function effectiveControls(type: Pick<ActionType, "controls">): ActionControls {
  return type.controls ?? LEGACY_CONTROLS
}

/** The input a binding's trigger reacts to (`Trigger::input_capability`). */
export function triggerInput(trigger: Trigger): InputCapability {
  switch (trigger.type) {
    case "press":
      return "press"
    case "release":
      return "release"
    case "rotateClockwise":
      return "rotate-clockwise"
    case "rotateCounterClockwise":
      return "rotate-counter-clockwise"
    case "click":
      return "click"
    case "doubleClick":
      return "double-click"
    case "longPress":
      return "long-press"
    case "touch":
      return trigger.gesture === "tap"
        ? "tap"
        : trigger.gesture === "swipe"
          ? "swipe"
          : "long-touch"
  }
}

/** What one control can do (`ControlCapabilities`), derived from the model. */
export interface ControlCapabilities {
  kind: ControlKind
  /** Hardware events the control reports. */
  inputs: InputCapability[]
  /** Gestures DeckPilot derives from them (click, double click, long press). */
  derived: InputCapability[]
}

const GESTURES: InputCapability[] = ["click", "double-click", "long-press"]

/** The capabilities of `control` on `model`; `null` when the model has no such control. */
export function controlCapabilities(
  model: DeviceModel,
  control: ControlId,
): ControlCapabilities | null {
  const { keys, encoders, touch } = model.capabilities
  switch (control.kind) {
    case "key":
      if (!keys || control.index >= keys.rows * keys.columns) return null
      return { kind: "key", inputs: ["press", "release"], derived: GESTURES }
    case "encoder": {
      if (!encoders || control.index >= encoders.count) return null
      const rotate: InputCapability[] = ["rotate-clockwise", "rotate-counter-clockwise"]
      return encoders.pressable
        ? { kind: "encoder", inputs: [...rotate, "press", "release"], derived: GESTURES }
        : { kind: "encoder", inputs: rotate, derived: [] }
    }
    case "touchZone":
      if (!touch || control.index >= touch.zones) return null
      return {
        kind: "touchZone",
        inputs: touch.gestures.map((g) => triggerInput({ type: "touch", gesture: g })),
        derived: [],
      }
  }
}

function supports(capabilities: ControlCapabilities, input: InputCapability): boolean {
  return capabilities.inputs.includes(input) || capabilities.derived.includes(input)
}

function sorted<T>(values: Iterable<T>, order: T[]): T[] {
  const set = new Set(values)
  return order.filter((v) => set.has(v))
}

/**
 * Whether `type` can run from a binding of a control with `capabilities`
 * for `trigger` (`deckpilot_core::compatibility`): `null` when it can,
 * else the first rule that fails, in the backend's order.
 */
export function bindingIncompatibility(
  type: Pick<ActionType, "controls">,
  capabilities: ControlCapabilities,
  trigger: Trigger,
): IncompatibilityReason | null {
  const controls = effectiveControls(type)
  const { kind } = capabilities
  const input = triggerInput(trigger)
  const supported = controls[kind]
  if (!supported) {
    return {
      type: "controlKindUnsupported",
      kind,
      supported: KINDS.filter((k) => controls[k] !== undefined),
    }
  }
  if (!supported.includes(input)) {
    return {
      type: "inputUnsupportedByAction",
      kind,
      input,
      supported: sorted(supported, INPUTS),
    }
  }
  if (!supports(capabilities, input)) return { type: "inputUnsupportedByControl", kind, input }
  return null
}

/**
 * Why `type` cannot be put on the control at all, whatever the trigger:
 * the action does not support the control's kind, or none of the inputs it
 * supports on it is one the control has. `null` when at least one trigger
 * works.
 */
export function actionIncompatibility(
  type: Pick<ActionType, "controls">,
  capabilities: ControlCapabilities,
): IncompatibilityReason | null {
  const controls = effectiveControls(type)
  const supported = controls[capabilities.kind]
  if (!supported) {
    return {
      type: "controlKindUnsupported",
      kind: capabilities.kind,
      supported: KINDS.filter((k) => controls[k] !== undefined),
    }
  }
  if (supported.some((input) => supports(capabilities, input))) return null
  const [first] = sorted(supported, INPUTS)
  return first ? { type: "inputUnsupportedByControl", kind: capabilities.kind, input: first } : null
}

// ---------------------------------------------------------------------------
// In words

const KIND_NOUN: Record<ControlKind, string> = {
  key: "key",
  encoder: "dial",
  touchZone: "touch zone",
}

const KIND_ARTICLE: Record<ControlKind, string> = {
  key: "a key",
  encoder: "a dial",
  touchZone: "a touch zone",
}

/** How an input is called in the editor (the trigger labels, in lower case). */
export const INPUT_LABEL: Record<InputCapability, string> = {
  press: "press",
  release: "release",
  "rotate-clockwise": "turn right",
  "rotate-counter-clockwise": "turn left",
  tap: "tap",
  "long-touch": "long touch",
  swipe: "swipe",
  click: "click",
  "double-click": "double click",
  "long-press": "long press",
}

/** The kind of action an input belongs to: every press gesture is a press. */
const FAMILY: Record<InputCapability, string> = {
  press: "press",
  release: "press",
  click: "press",
  "double-click": "press",
  "long-press": "press",
  "rotate-clockwise": "turn",
  "rotate-counter-clockwise": "turn",
  tap: "tap",
  "long-touch": "long touch",
  swipe: "swipe",
}

/** "a", "a and b", "a, b and c" (or "or"). */
function join(items: string[], word: "and" | "or"): string {
  if (items.length <= 1) return items[0] ?? ""
  return `${items.slice(0, -1).join(", ")} ${word} ${items.at(-1) ?? ""}`
}

function capitalize(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/** What a control can do, e.g. "turn and press actions", "press actions only". */
export function controlSummary(capabilities: ControlCapabilities): string {
  const families = [
    ...new Set(
      sorted([...capabilities.inputs, ...capabilities.derived], TURN_FIRST).map((i) => FAMILY[i]),
    ),
  ]
  if (families.length === 0) return "no actions"
  return families.length === 1
    ? `${families[0] ?? ""} actions only`
    : `${join(families, "and")} actions`
}

/** Turns before presses: a dial is described by what it is for. */
const TURN_FIRST: InputCapability[] = [
  "rotate-clockwise",
  "rotate-counter-clockwise",
  ...INPUTS.filter((i) => i !== "rotate-clockwise" && i !== "rotate-counter-clockwise"),
]

/** "The selected Stream Deck MK.2 key" */
function selected(model: Pick<DeviceModel, "name">, kind: ControlKind): string {
  return `The selected ${model.name} ${KIND_NOUN[kind]}`
}

/**
 * The reason in words, for the selected control of `model`, e.g.
 * "Requires a dial. The selected Stream Deck MK.2 key supports press
 * actions only."
 */
export function describeIncompatibility(
  reason: IncompatibilityReason,
  model: Pick<DeviceModel, "name">,
  capabilities: ControlCapabilities | null,
): string {
  const summary = capabilities ? controlSummary(capabilities) : null
  switch (reason.type) {
    case "controlKindUnsupported": {
      const requires =
        reason.supported.length === 0
          ? "Cannot be put on any control."
          : `Requires ${join(
              reason.supported.map((k) => KIND_ARTICLE[k]),
              "or",
            )}.`
      return summary ? `${requires} ${selected(model, reason.kind)} supports ${summary}.` : requires
    }
    case "inputUnsupportedByAction": {
      const on = KIND_ARTICLE[reason.kind]
      const reacts = join(
        reason.supported.map((i) => INPUT_LABEL[i]),
        "and",
      )
      return `Does not react to ${INPUT_LABEL[reason.input]} on ${on}. On ${on} it reacts to ${reacts}.`
    }
    case "inputUnsupportedByControl": {
      const has = `${selected(model, reason.kind)} has no ${INPUT_LABEL[reason.input]}`
      return summary ? `${has}; it supports ${summary}.` : `${has}.`
    }
  }
}

/**
 * Why an action cannot go on the control at all, in words: what it
 * requires and what the control supports. For the library and drops.
 */
export function describeActionIncompatibility(
  type: Pick<ActionType, "controls">,
  reason: IncompatibilityReason,
  model: Pick<DeviceModel, "name">,
  capabilities: ControlCapabilities,
): string {
  if (reason.type !== "inputUnsupportedByControl") {
    return describeIncompatibility(reason, model, capabilities)
  }
  // No input the action takes on this kind is one the control has.
  const supported = sorted(effectiveControls(type)[reason.kind] ?? [], INPUTS)
  const requires = capitalize(
    `requires ${join(
      supported.map((i) => `a ${INPUT_LABEL[i]}`),
      "or",
    )} on ${KIND_ARTICLE[reason.kind]}.`,
  )
  return `${requires} ${selected(model, reason.kind)} supports ${controlSummary(capabilities)}.`
}

/**
 * Why `type` cannot be put on `control` of `model`, in words; `null` when
 * it can (or the model has no such control, which layout validation
 * reports).
 */
export function whyNotOnControl(
  type: Pick<ActionType, "controls">,
  model: DeviceModel,
  control: ControlId,
): string | null {
  const capabilities = controlCapabilities(model, control)
  if (!capabilities) return null
  const reason = actionIncompatibility(type, capabilities)
  return reason ? describeActionIncompatibility(type, reason, model, capabilities) : null
}

/**
 * Why `type` cannot run from `control` for `trigger`, in words; `null`
 * when it can.
 */
export function whyNotForTrigger(
  type: Pick<ActionType, "controls">,
  model: DeviceModel,
  control: ControlId,
  trigger: Trigger,
): string | null {
  const capabilities = controlCapabilities(model, control)
  if (!capabilities) return null
  const reason = bindingIncompatibility(type, capabilities, trigger)
  return reason ? describeIncompatibility(reason, model, capabilities) : null
}
