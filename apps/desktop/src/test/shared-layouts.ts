// Shared layouts shaped like the backend's, for tests.

import type { LayoutAssignment, SharedLayoutSummary } from "@/lib/types"

export function assignment(
  profileId: string,
  profileName: string,
  change: Partial<LayoutAssignment> = {},
): LayoutAssignment {
  return { profileId, profileName, position: 0, isDefault: false, ...change }
}

export function sharedLayout(change: Partial<SharedLayoutSummary> = {}): SharedLayoutSummary {
  return {
    id: "layout-1",
    name: "Desk",
    model: "elgato.streamdeck.mk2",
    modelName: "Stream Deck MK.2",
    pages: 2,
    bindings: 9,
    updatedAt: 1_700_000_000,
    profiles: [],
    compatibility: [
      {
        model: "elgato.streamdeck.mk2",
        modelName: "Stream Deck MK.2",
        fit: { type: "native" },
        explanation: "Made for this model",
      },
      {
        model: "elgato.streamdeck.xl",
        modelName: "Stream Deck XL",
        fit: { type: "fits" },
        explanation: "Every key fits",
      },
      {
        model: "elgato.streamdeck.mini",
        modelName: "Stream Deck Mini",
        fit: { type: "missing", controls: [{ kind: "key", index: 7 }], triggers: 1 },
        explanation: "Misses 1 key (1 trigger)",
      },
    ],
    ...change,
  }
}

/** A layout whose device model is not in the catalog. */
export function unknownModelLayout(change: Partial<SharedLayoutSummary> = {}): SharedLayoutSummary {
  return sharedLayout({
    id: "layout-unknown",
    name: "Old pad",
    model: "acme.pad",
    modelName: null,
    compatibility: [
      {
        model: "acme.pad",
        modelName: "acme.pad",
        fit: { type: "unknownModel" },
        explanation: "This device model is not in the catalog",
      },
    ],
    ...change,
  })
}
