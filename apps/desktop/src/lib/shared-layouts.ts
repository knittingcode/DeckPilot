import type { ModelFit, SharedLayoutSummary } from "./types"

/** Whether the keys of a layout work on a model as they are. */
export function fitsModel(fit: ModelFit): boolean {
  return fit.type === "native" || fit.type === "fits"
}

/** The layout's own device model is not in the catalog. */
export function hasUnknownModel(layout: SharedLayoutSummary): boolean {
  return layout.modelName === null || layout.compatibility[0]?.fit.type === "unknownModel"
}

/** The layout's device in words: the model's name, or its id when unknown. */
export function modelLabel(layout: SharedLayoutSummary): string {
  return layout.modelName ?? layout.model
}

/** The default name of a copy. */
export function copyName(name: string): string {
  return `${name} copy`
}
