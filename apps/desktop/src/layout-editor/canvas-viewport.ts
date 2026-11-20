// The Device Editor's canvas viewport: pure geometry of zooming and panning
// the device preview (layout-editor/device-viewport.tsx). The viewport has
// a fixed size and never scrolls; the device is drawn at a key size that
// follows the zoom (so the renderer draws key images at the size they are
// shown, never scaled raster) and moved by a translation.
//
// 100 % ("actual size") is a key of `ACTUAL_KEY` CSS pixels: 72, the
// editor's natural key size (`KEY_SIZE.fallback`), which is also the
// native key image of the Stream Deck MK.2. Every other zoom is that key
// size times the zoom, rounded to whole CSS pixels.

import {
  DIAL_SIZE,
  EDGE_ROOM,
  KEY_GAP,
  KEY_SIZE,
  deviceSize,
  type DeviceGeometry,
} from "@/components/device-geometry"

/** The key size at 100 %, in CSS pixels. */
export const ACTUAL_KEY = KEY_SIZE.fallback
/** Smallest and largest zoom (10 % … 400 %). */
export const ZOOM_LIMITS = { min: 0.1, max: 4 } as const
/**
 * Fit to screen never draws keys larger than this (CSS pixels, about
 * 144 %): the largest key of the editor's earlier preview. A small device
 * (Mini) in a large window fits larger than 100 %, which reads better than
 * a small device in the middle of empty space; beyond that, zoom in.
 */
export const FIT_MAX_KEY = KEY_SIZE.max
/** The steps of the zoom buttons and `+`/`-`. */
export const ZOOM_STEPS = [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.25, 1.5, 2, 3, 4] as const
/** At least this much of the device (CSS pixels) stays in view while panning. */
export const PAN_MARGIN = 48
/** Room kept around the device by Fit to screen, in CSS pixels. */
export const FIT_MARGIN = 16
/** Distance of one arrow-key pan (CSS pixels); four times with Shift. */
export const ARROW_PAN = 48
/** Smallest and largest key image the renderer draws (PREVIEW_KEY_PIXELS in layouts.rs). */
export const PREVIEW_KEY_PIXELS = { min: 16, max: 512 } as const
/** From how many keys on, keys outside the viewport are drawn as placeholders. */
export const CULL_THRESHOLD = 128

export interface Size {
  width: number
  height: number
}

/** Where the device is: its zoom and the viewport position of its top-left corner. */
export interface View {
  zoom: number
  x: number
  y: number
}

export interface Insets {
  top: number
  right: number
  bottom: number
  left: number
}

const finite = (value: number, fallback: number) => (Number.isFinite(value) ? value : fallback)

/** `zoom` within the limits; 100 % for NaN, Infinity and the like. */
export function clampZoom(zoom: number): number {
  if (!Number.isFinite(zoom) || zoom <= 0) return 1
  return Math.min(ZOOM_LIMITS.max, Math.max(ZOOM_LIMITS.min, zoom))
}

/** The key size (whole CSS pixels) at a zoom. */
export function keySizeAt(zoom: number): number {
  return Math.max(1, Math.round(ACTUAL_KEY * clampZoom(zoom)))
}

/** The zoom as the toolbar shows it, in whole percent. */
export function zoomPercent(zoom: number): number {
  return Math.round(clampZoom(zoom) * 100)
}

/**
 * The size of the drawn device (CSS pixels) at key size `key`, with the
 * same whole-pixel rounding as DeviceCanvas (gaps and edge room rounded,
 * the strip's padding not).
 */
export function canvasSize(geometry: DeviceGeometry, key: number): Size {
  const k = Math.max(1, finite(key, ACTUAL_KEY))
  const pad = Math.round(k * EDGE_ROOM)
  const gap = Math.round(k * KEY_GAP)
  const sectionGap = Math.round(k * KEY_GAP * 1.5)
  const grid = geometry.columns * k + (geometry.columns - 1) * gap
  const sections: number[] = []
  if (geometry.rows > 0) sections.push(geometry.rows * k + (geometry.rows - 1) * gap)
  if (geometry.zones > 0) {
    const inner = grid - 0.2 * k
    const zone = (inner - (geometry.zones - 1) * 0.1 * k) / geometry.zones
    sections.push(zone / 2 + 0.2 * k)
  }
  if (geometry.dials > 0) sections.push(DIAL_SIZE * k)
  const height =
    sections.reduce((sum, h) => sum + h, 0) + Math.max(sections.length - 1, 0) * sectionGap
  return { width: grid + pad * 2, height: height + pad * 2 }
}

/** The zoom at which the device fills `viewport` less `insets`, within the limits. */
export function fitZoom(geometry: DeviceGeometry, viewport: Size, insets: Insets): number {
  const width = viewport.width - insets.left - insets.right
  const height = viewport.height - insets.top - insets.bottom
  if (!(width > 0) || !(height > 0)) return 1
  const keys = deviceSize(geometry)
  let key = Math.floor(Math.min(width / keys.width, height / keys.height, FIT_MAX_KEY))
  // Rounding of gaps can make the device a pixel or two larger than the estimate.
  while (key > 1) {
    const size = canvasSize(geometry, key)
    if (size.width <= width && size.height <= height) break
    key--
  }
  return clampZoom(key / ACTUAL_KEY)
}

/**
 * The fitting insets: a margin on every side, plus `reserved` pixels at the
 * bottom for controls inside the viewport (none in the editor, whose
 * controls have a strip of their own below it).
 */
export function fitInsets(reserved = 0): Insets {
  const b = Math.max(0, finite(reserved, 0))
  return { top: FIT_MARGIN, right: FIT_MARGIN, bottom: b + FIT_MARGIN, left: FIT_MARGIN }
}

/**
 * What the bar on the canvas/inspector divider reserves on either side of
 * the line: half its height (whole pixels, rounded up). The bar is moved up
 * by half its height, so its centre is on the line (`dividerBar`).
 */
export function dividerHalf(barHeight: number): number {
  return Math.max(0, Math.ceil(finite(barHeight, 0) / 2))
}

/**
 * Where a bar of `bar` size sits on a divider at `lineY` across `width`:
 * centred horizontally, its centre on the line.
 */
export function dividerBar(bar: Size, width: number, lineY: number) {
  const top = lineY - bar.height / 2
  const left = (width - bar.width) / 2
  return { top, left, bottom: top + bar.height, right: left + bar.width }
}

/** The device fitted and centred in the space `insets` leave. */
export function fitView(geometry: DeviceGeometry, viewport: Size, insets: Insets): View {
  const zoom = fitZoom(geometry, viewport, insets)
  return centred(geometry, viewport, insets, zoom)
}

/** A rectangle in viewport coordinates (CSS pixels). */
export interface Rect {
  left: number
  top: number
  right: number
  bottom: number
}

export function intersects(a: Rect, b: Rect): boolean {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/** The device's rectangle in the viewport at `view`. */
export function deviceRect(geometry: DeviceGeometry, view: View): Rect {
  const size = canvasSize(geometry, keySizeAt(view.zoom))
  return { left: view.x, top: view.y, right: view.x + size.width, bottom: view.y + size.height }
}

/**
 * Distance of the zoom toolbar from the viewport's right and bottom edges
 * (`--canvas-overlay-inset` in index.css, 1rem), the same on both.
 */
export const TOOLBAR_OFFSET = 16

/**
 * How far the zoom toolbar must move up from its corner for the page
 * navigation (`nav`, centred on the divider, the viewport's bottom edge)
 * to stay clear of it: 0 unless the viewport is so narrow that they share
 * horizontal space; then just above the navigation's upper half.
 */
export function toolbarLift(
  toolbar: Size,
  viewport: Size,
  nav: Size | null,
  inset = TOOLBAR_OFFSET,
): number {
  if (!nav || !(nav.width > 0)) return 0
  const left = viewport.width - inset - toolbar.width
  const navRight = (viewport.width + nav.width) / 2
  if (left >= navRight) return 0
  const navTop = dividerHalf(nav.height)
  return Math.max(0, navTop + inset / 4 - inset)
}

/**
 * Where the zoom toolbar sits: in the viewport's bottom-right corner,
 * `inset` from the right and from the bottom edge (the divider), lifted
 * only when the page navigation would be under it (`toolbarLift`).
 */
export function toolbarRect(
  toolbar: Size,
  viewport: Size,
  nav: Size | null = null,
  inset = TOOLBAR_OFFSET,
): Rect {
  const right = viewport.width - inset
  const bottom = viewport.height - inset - toolbarLift(toolbar, viewport, nav, inset)
  return { left: right - toolbar.width, top: bottom - toolbar.height, right, bottom }
}

/**
 * Fit to screen that keeps the device clear of `exclusion` (the zoom
 * toolbar's corner): the plain fit when it does not touch it, else the
 * larger of fitting above it or beside it. The device is only made smaller
 * when the corner would cover keys.
 */
export function fitAvoiding(
  geometry: DeviceGeometry,
  viewport: Size,
  insets: Insets,
  exclusion: Rect | null,
): View {
  const plain = fitView(geometry, viewport, insets)
  if (!exclusion || !intersects(deviceRect(geometry, plain), exclusion)) return plain
  const above = fitView(geometry, viewport, {
    ...insets,
    bottom: Math.max(insets.bottom, viewport.height - exclusion.top + FIT_MARGIN / 2),
  })
  const beside = fitView(geometry, viewport, {
    ...insets,
    right: Math.max(insets.right, viewport.width - exclusion.left + FIT_MARGIN / 2),
    left: Math.max(insets.left, viewport.width - exclusion.left + FIT_MARGIN / 2),
  })
  const fits = [above, beside].filter((v) => !intersects(deviceRect(geometry, v), exclusion))
  if (fits.length === 0) return above
  return fits.reduce((best, v) => (v.zoom > best.zoom ? v : best))
}

/** The device at `zoom`, centred in the space `insets` leave. */
export function centred(
  geometry: DeviceGeometry,
  viewport: Size,
  insets: Insets,
  zoom: number,
): View {
  const z = clampZoom(zoom)
  const size = canvasSize(geometry, keySizeAt(z))
  const width = viewport.width - insets.left - insets.right
  const height = viewport.height - insets.top - insets.bottom
  return clampPan(
    {
      zoom: z,
      x: Math.round(insets.left + (width - size.width) / 2),
      y: Math.round(insets.top + (height - size.height) / 2),
    },
    geometry,
    viewport,
  )
}

/**
 * `view` moved as little as needed for at least `PAN_MARGIN` pixels of the
 * device (or all of it, when smaller) to stay inside the viewport.
 */
export function clampPan(view: View, geometry: DeviceGeometry, viewport: Size): View {
  const zoom = clampZoom(view.zoom)
  const size = canvasSize(geometry, keySizeAt(zoom))
  const clamp = (value: number, length: number, room: number) => {
    const margin = Math.min(PAN_MARGIN, length)
    const low = margin - length
    const high = Math.max(low, room - margin)
    return Math.min(high, Math.max(low, finite(value, 0)))
  }
  return {
    zoom,
    x: clamp(view.x, size.width, Math.max(0, finite(viewport.width, 0))),
    y: clamp(view.y, size.height, Math.max(0, finite(viewport.height, 0))),
  }
}

/** `view` moved by `dx`, `dy` CSS pixels, within the pan bounds. */
export function panBy(
  view: View,
  dx: number,
  dy: number,
  geometry: DeviceGeometry,
  viewport: Size,
): View {
  return clampPan(
    { ...view, x: view.x + finite(dx, 0), y: view.y + finite(dy, 0) },
    geometry,
    viewport,
  )
}

/**
 * `view` zoomed to `zoom` around `point` (viewport coordinates): the part
 * of the device under the point stays under it (up to whole-pixel key
 * rounding), then kept within the pan bounds.
 */
export function zoomAt(
  view: View,
  zoom: number,
  point: { x: number; y: number },
  geometry: DeviceGeometry,
  viewport: Size,
): View {
  const next = clampZoom(zoom)
  const ratio = keySizeAt(next) / keySizeAt(view.zoom)
  const px = finite(point.x, viewport.width / 2)
  const py = finite(point.y, viewport.height / 2)
  return clampPan(
    { zoom: next, x: px - (px - view.x) * ratio, y: py - (py - view.y) * ratio },
    geometry,
    viewport,
  )
}

/** The next zoom step above (`1`) or below (`-1`) `zoom`. */
export function stepZoom(zoom: number, direction: 1 | -1): number {
  const z = clampZoom(zoom)
  const epsilon = 0.005
  if (direction > 0) return ZOOM_STEPS.find((s) => s > z + epsilon) ?? ZOOM_LIMITS.max
  return [...ZOOM_STEPS].reverse().find((s) => s < z - epsilon) ?? ZOOM_LIMITS.min
}

/** Pixels per wheel `deltaMode` unit (pixels, lines, pages). */
export function wheelUnit(deltaMode: number, pageHeight: number): number {
  if (deltaMode === 1) return 16
  if (deltaMode === 2) return Math.max(1, finite(pageHeight, 800))
  return 1
}

/**
 * The zoom factor of a Ctrl/Cmd+wheel or trackpad pinch (WebKit and
 * Chromium report a pinch as a wheel with `ctrlKey`): one mouse wheel notch
 * (≈100 px) zooms by about a quarter, a pinch by its small deltas smoothly.
 */
export function wheelZoomFactor(deltaY: number): number {
  const delta = Math.max(-40, Math.min(40, finite(deltaY, 0)))
  return Math.exp(-delta * 0.008)
}

/**
 * The pan of a plain wheel or two-finger scroll: the device moves against
 * the scroll, as content does; Shift turns a mouse wheel sideways.
 */
export function wheelPan(
  event: {
    deltaX: number
    deltaY: number
    deltaMode: number
    shiftKey: boolean
  },
  pageHeight: number,
): { dx: number; dy: number } {
  const unit = wheelUnit(event.deltaMode, pageHeight)
  let dx = finite(event.deltaX, 0) * unit
  let dy = finite(event.deltaY, 0) * unit
  if (event.shiftKey && dx === 0) {
    dx = dy
    dy = 0
  }
  return { dx: -dx, dy: -dy }
}

/**
 * How far to pan for `rect` (viewport coordinates, e.g. a key that got
 * keyboard focus) to be fully in view, `margin` pixels from the edges and
 * above the `bottom` inset; 0, 0 when it is.
 */
export function revealDelta(
  rect: { left: number; top: number; right: number; bottom: number },
  viewport: Size,
  margin = 12,
  bottom = 0,
): { dx: number; dy: number } {
  const axis = (start: number, end: number, room: number) => {
    if (end - start > room - 2 * margin) return margin - start
    if (start < margin) return margin - start
    if (end > room - margin) return room - margin - end
    return 0
  }
  return {
    dx: axis(rect.left, rect.right, viewport.width),
    dy: axis(rect.top, rect.bottom, viewport.height - Math.max(0, bottom)),
  }
}

/** The key rows and columns (inclusive) at least partly in view, with one of slack. */
export interface VisibleRange {
  rows: [number, number]
  columns: [number, number]
}

export function visibleRange(view: View, geometry: DeviceGeometry, viewport: Size): VisibleRange {
  const key = keySizeAt(view.zoom)
  const pad = Math.round(key * EDGE_ROOM)
  const pitch = key + Math.round(key * KEY_GAP)
  const span = (offset: number, room: number, count: number): [number, number] => {
    const start = Math.floor((-offset - pad) / pitch) - 1
    const end = Math.ceil((room - offset - pad) / pitch) + 1
    return [Math.max(0, start), Math.min(count - 1, end)]
  }
  return {
    rows: span(view.y, viewport.height, geometry.rows),
    columns: span(view.x, viewport.width, geometry.columns),
  }
}

/** Whether key `index` of a grid of `columns` is in `range` (always without one). */
export function inRange(range: VisibleRange | null, index: number, columns: number): boolean {
  if (!range) return true
  const row = Math.floor(index / columns)
  const column = index % columns
  return (
    row >= range.rows[0] &&
    row <= range.rows[1] &&
    column >= range.columns[0] &&
    column <= range.columns[1]
  )
}

/** `value` rounded to whole device pixels, so a translated device stays crisp. */
export function snap(value: number, pixelRatio: number): number {
  const ratio = Number.isFinite(pixelRatio) && pixelRatio > 0 ? pixelRatio : 1
  return Math.round(finite(value, 0) * ratio) / ratio
}

/** The key image width to ask the renderer for: within its limits. */
export function previewPixels(pixels: number): number {
  if (!Number.isFinite(pixels) || pixels <= 0) return PREVIEW_KEY_PIXELS.min
  return Math.min(PREVIEW_KEY_PIXELS.max, Math.max(PREVIEW_KEY_PIXELS.min, Math.round(pixels)))
}

// ---------------------------------------------------------------------------
// The view each device model was left at: an editor preference of this
// browser profile (localStorage), not part of layouts or the database.

/** The localStorage key; the version changes when the stored shape does. */
export const VIEW_STORAGE_KEY = "deckpilot.editor.canvasView.v1"

/**
 * A remembered view: Fit to screen, or a zoom and the device point (in
 * keys) at the viewport's centre, so it survives another window size.
 */
export type SavedView = { mode: "fit" } | { mode: "manual"; zoom: number; cx: number; cy: number }

function storage(): Storage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage
  } catch {
    return null
  }
}

function readAll(store: Storage | null): Record<string, SavedView> {
  if (!store) return {}
  try {
    const parsed: unknown = JSON.parse(store.getItem(VIEW_STORAGE_KEY) ?? "{}")
    return parsed && typeof parsed === "object" ? (parsed as Record<string, SavedView>) : {}
  } catch {
    return {}
  }
}

/** The view remembered for `model`, when valid. */
export function loadView(model: string, store: Storage | null = storage()): SavedView | null {
  const saved = readAll(store)[model] as Partial<Record<string, unknown>> | undefined
  if (!saved || typeof saved !== "object") return null
  if (saved.mode === "fit") return { mode: "fit" }
  const { zoom, cx, cy } = saved
  if (
    saved.mode === "manual" &&
    typeof zoom === "number" &&
    typeof cx === "number" &&
    typeof cy === "number" &&
    [zoom, cx, cy].every(Number.isFinite)
  )
    return { mode: "manual", zoom: clampZoom(zoom), cx, cy }
  return null
}

/** Remembers `view` for `model`; storage that fails (private mode, quota) is ignored. */
export function saveView(model: string, view: SavedView, store: Storage | null = storage()) {
  if (!store) return
  try {
    const all = readAll(store)
    all[model] = view
    store.setItem(VIEW_STORAGE_KEY, JSON.stringify(all))
  } catch {
    // Not remembered: the editor still works.
  }
}

/** The view to remember for `view` in `viewport`. */
export function toSaved(view: View, viewport: Size, fit: boolean): SavedView {
  if (fit) return { mode: "fit" }
  const key = keySizeAt(view.zoom)
  return {
    mode: "manual",
    zoom: clampZoom(view.zoom),
    cx: (viewport.width / 2 - view.x) / key,
    cy: (viewport.height / 2 - view.y) / key,
  }
}

/** A remembered manual view in `viewport`: its zoom, its centre at the viewport's. */
export function fromSaved(
  saved: Extract<SavedView, { mode: "manual" }>,
  geometry: DeviceGeometry,
  viewport: Size,
): View {
  const zoom = clampZoom(saved.zoom)
  const key = keySizeAt(zoom)
  return clampPan(
    { zoom, x: viewport.width / 2 - saved.cx * key, y: viewport.height / 2 - saved.cy * key },
    geometry,
    viewport,
  )
}
