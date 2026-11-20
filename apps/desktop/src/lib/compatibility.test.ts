import { afterEach, describe, expect, it } from "vitest"

import { i18n } from "@/i18n"

import { deviceModel } from "@/test/fixtures"

import {
  actionIncompatibility,
  bindingIncompatibility,
  controlCapabilities,
  controlSummary,
  describeIncompatibility,
  triggerInput,
  whyNotForTrigger,
  whyNotOnControl,
} from "./compatibility"
import type { ActionControls, DeviceModel } from "./types"

const mk2 = deviceModel()
const plus: DeviceModel = deviceModel({
  id: "elgato.streamdeck.plus",
  name: "Stream Deck +",
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: true },
    touch: { zones: 4, image: null, gestures: ["tap", "longPress", "swipe"] },
  },
})
const stiffDials: DeviceModel = deviceModel({
  id: "vendor.knobs",
  name: "Knob Box",
  capabilities: { keys: null, encoders: { count: 2, pressable: false }, touch: null },
})

const key = { kind: "key", index: 0 } as const
const dial = { kind: "encoder", index: 0 } as const
const zone = { kind: "touchZone", index: 0 } as const

/** Actions as their declarations; `{}` has none (the legacy default). */
const legacy = {}
const keysOnly = {
  controls: {
    key: ["press", "release", "click", "double-click", "long-press"],
  } satisfies ActionControls,
}
const dialOnly = {
  controls: { encoder: ["rotate-clockwise", "rotate-counter-clockwise"] } satisfies ActionControls,
}
const stepper = {
  controls: {
    key: ["press", "release", "click", "double-click", "long-press"],
    encoder: ["rotate-clockwise", "rotate-counter-clockwise", "press", "click"],
  } satisfies ActionControls,
}
const dialPress = { controls: { encoder: ["press"] } satisfies ActionControls }

describe("control capabilities", () => {
  it("are derived from the model, as the backend derives them", () => {
    expect(controlCapabilities(mk2, key)).toEqual({
      kind: "key",
      inputs: ["press", "release"],
      derived: ["click", "double-click", "long-press"],
    })
    expect(controlCapabilities(plus, dial)?.inputs).toEqual([
      "rotate-clockwise",
      "rotate-counter-clockwise",
      "press",
      "release",
    ])
    expect(controlCapabilities(stiffDials, dial)).toEqual({
      kind: "encoder",
      inputs: ["rotate-clockwise", "rotate-counter-clockwise"],
      derived: [],
    })
    expect(controlCapabilities(plus, zone)?.inputs).toEqual(["tap", "long-touch", "swipe"])
    // Controls the model lacks.
    expect(controlCapabilities(mk2, dial)).toBeNull()
    expect(controlCapabilities(mk2, { kind: "key", index: 15 })).toBeNull()
  })

  it("map every trigger to one input", () => {
    expect(triggerInput({ type: "rotateCounterClockwise" })).toBe("rotate-counter-clockwise")
    expect(triggerInput({ type: "touch", gesture: "longPress" })).toBe("long-touch")
    expect(triggerInput({ type: "doubleClick" })).toBe("double-click")
  })

  it("are summed up by what they are for", () => {
    const caps = (model: DeviceModel, control: typeof key | typeof dial | typeof zone) => {
      const found = controlCapabilities(model, control)
      if (!found) throw new Error("no such control")
      return controlSummary(found)
    }
    expect(caps(mk2, key)).toBe("press actions only")
    expect(caps(plus, dial)).toBe("turn and press actions")
    expect(caps(stiffDials, dial)).toBe("turn actions only")
    expect(caps(plus, zone)).toBe("tap, long touch and swipe actions")
  })
})

describe("why an action cannot go on a control", () => {
  it("explains an encoder-only action on a Stream Deck MK.2 key", () => {
    expect(whyNotOnControl(dialOnly, mk2, key)).toBe(
      "Requires a dial. The selected Stream Deck MK.2 key supports press actions only.",
    )
  })

  it("explains a key-only action on a dial", () => {
    expect(whyNotOnControl(keysOnly, plus, dial)).toBe(
      "Requires a key. The selected Stream Deck + dial supports turn and press actions.",
    )
  })

  it("explains an action without a declaration (keys only) on a touch zone", () => {
    expect(whyNotOnControl(legacy, plus, zone)).toBe(
      "Requires a key. The selected Stream Deck + touch zone supports tap, long touch and swipe actions.",
    )
    expect(whyNotOnControl(legacy, plus, dial)).toBe(
      "Requires a key. The selected Stream Deck + dial supports turn and press actions.",
    )
    // On a key it works, as it always did.
    expect(whyNotOnControl(legacy, mk2, key)).toBeNull()
  })

  it("names every kind an action supports", () => {
    expect(whyNotOnControl(stepper, plus, zone)).toBe(
      "Requires a key or a dial. The selected Stream Deck + touch zone supports tap, long touch and swipe actions.",
    )
    expect(whyNotOnControl(stepper, plus, dial)).toBeNull()
  })

  it("explains a kind it supports without an input the control has", () => {
    expect(actionIncompatibility(dialPress, requireCaps(stiffDials, dial))).toEqual({
      type: "inputUnsupportedByControl",
      kind: "encoder",
      input: "press",
    })
    expect(whyNotOnControl(dialPress, stiffDials, dial)).toBe(
      "Requires a press on a dial. The selected Knob Box dial supports turn actions only.",
    )
    expect(whyNotOnControl(dialPress, plus, dial)).toBeNull()
  })

  it("says nothing about controls the model lacks (layout validation reports them)", () => {
    expect(whyNotOnControl(dialOnly, mk2, dial)).toBeNull()
  })
})

describe("why a binding cannot run", () => {
  it("checks the rules in the backend's order", () => {
    expect(bindingIncompatibility(keysOnly, requireCaps(plus, dial), { type: "press" })).toEqual({
      type: "controlKindUnsupported",
      kind: "encoder",
      supported: ["key"],
    })
    expect(
      bindingIncompatibility(stepper, requireCaps(plus, dial), { type: "doubleClick" }),
    ).toEqual({
      type: "inputUnsupportedByAction",
      kind: "encoder",
      input: "double-click",
      supported: ["press", "rotate-clockwise", "rotate-counter-clockwise", "click"],
    })
    expect(
      bindingIncompatibility(
        { controls: { encoder: ["press", "rotate-clockwise"] } },
        requireCaps(stiffDials, dial),
        { type: "press" },
      ),
    ).toEqual({ type: "inputUnsupportedByControl", kind: "encoder", input: "press" })
    expect(
      bindingIncompatibility(stepper, requireCaps(plus, dial), { type: "rotateClockwise" }),
    ).toBeNull()
  })

  it("reads each reason as a sentence", () => {
    expect(whyNotForTrigger(stepper, plus, dial, { type: "doubleClick" })).toBe(
      "Does not react to double click on a dial. On a dial it reacts to press, turn right, turn left and click.",
    )
    expect(
      whyNotForTrigger({ controls: { encoder: ["press"] } }, stiffDials, dial, { type: "press" }),
    ).toBe("The selected Knob Box dial has no press; it supports turn actions only.")
    expect(whyNotForTrigger(stepper, plus, dial, { type: "rotateCounterClockwise" })).toBeNull()
  })

  it("explains the reasons the backend reports", () => {
    expect(
      describeIncompatibility(
        { type: "controlKindUnsupported", kind: "touchZone", supported: ["key"] },
        plus,
        requireCaps(plus, zone),
      ),
    ).toBe(
      "Requires a key. The selected Stream Deck + touch zone supports tap, long touch and swipe actions.",
    )
    // Without the control's capabilities (a model the app does not know).
    expect(
      describeIncompatibility(
        { type: "controlKindUnsupported", kind: "encoder", supported: [] },
        plus,
        null,
      ),
    ).toBe("Cannot be put on any control.")
  })
})

function requireCaps(model: DeviceModel, control: typeof key | typeof dial | typeof zone) {
  const found = controlCapabilities(model, control)
  if (!found) throw new Error("no such control")
  return found
}

describe("compatibility texts in Polish", () => {
  afterEach(async () => {
    await i18n.changeLanguage("en")
  })

  it("read as Polish sentences", async () => {
    await i18n.changeLanguage("pl")
    expect(whyNotOnControl(dialOnly, mk2, key)).toBe(
      "Wymaga pokrętła. Wybrany klawisz urządzenia Stream Deck MK.2 obsługuje tylko akcje naciśnięcia.",
    )
    expect(whyNotOnControl(stepper, plus, zone)).toBe(
      "Wymaga klawisza lub pokrętła. Wybrana strefa dotykowa urządzenia Stream Deck + obsługuje akcje stuknięcia, długiego dotknięcia i przesunięcia.",
    )
    expect(whyNotOnControl(dialPress, stiffDials, dial)).toBe(
      "Wymaga naciśnięcia na pokrętle. Wybrane pokrętło urządzenia Knob Box obsługuje tylko akcje obrotu.",
    )
    expect(whyNotForTrigger(stepper, plus, dial, { type: "doubleClick" })).toBe(
      "Nie reaguje na dwukrotne kliknięcie na pokrętle. Na pokrętle reaguje na naciśnięcie, obrót w prawo, obrót w lewo i kliknięcie.",
    )
    expect(
      whyNotForTrigger({ controls: { encoder: ["press"] } }, stiffDials, dial, { type: "press" }),
    ).toBe(
      "Wybrane pokrętło urządzenia Knob Box nie reaguje na naciśnięcie; obsługuje tylko akcje obrotu.",
    )
  })
})
