/**
 * Editor state kept per viewer: the canvas viewport of each workflow and
 * which panels are open. Stored in this window's local storage only, so
 * saving a workflow or reopening the app keeps the view; storage that is
 * unavailable just means defaults.
 */

export interface Viewport {
  x: number
  y: number
  zoom: number
}

export interface Panels {
  catalog: boolean
  inspector: boolean
}

const VIEWPORT_KEY = "deckpilot.workflow-viewport."
const PANELS_KEY = "deckpilot.workflow-panels"
/** Below this window width the catalog starts closed. */
const COMPACT_WIDTH = 1280

function read<T>(key: string, valid: (value: unknown) => value is T): T | null {
  try {
    const text = localStorage.getItem(key)
    if (!text) return null
    const value: unknown = JSON.parse(text)
    return valid(value) ? value : null
  } catch {
    return null
  }
}

function write(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // Not remembered in this window.
  }
}

const isViewport = (v: unknown): v is Viewport =>
  typeof v === "object" &&
  v !== null &&
  ["x", "y", "zoom"].every((k) => Number.isFinite((v as Record<string, unknown>)[k])) &&
  (v as Viewport).zoom > 0

const isPanels = (v: unknown): v is Panels =>
  typeof v === "object" &&
  v !== null &&
  typeof (v as Panels).catalog === "boolean" &&
  typeof (v as Panels).inspector === "boolean"

export function loadViewport(workflow: string): Viewport | null {
  return read(VIEWPORT_KEY + workflow, isViewport)
}

export function saveViewport(workflow: string, viewport: Viewport) {
  write(VIEWPORT_KEY + workflow, viewport)
}

/** Panels for a window of `width`: compact windows start without the catalog. */
export function defaultPanels(width: number): Panels {
  return { catalog: width >= COMPACT_WIDTH, inspector: true }
}

export function loadPanels(width: number): Panels {
  return read(PANELS_KEY, isPanels) ?? defaultPanels(width)
}

export function savePanels(panels: Panels) {
  write(PANELS_KEY, panels)
}
