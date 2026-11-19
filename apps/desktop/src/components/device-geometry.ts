// The proportions of the device drawn by DeviceCanvas, in units of one key.
// Everything (gaps, padding, the touch strip, the dials) scales with the
// key size, so the preview keeps the hardware's look at every size.

import { controlsOf } from "@/lib/layout-model"
import type { DeviceModel } from "@/lib/types"

/** Gap between two keys, in keys. */
export const KEY_GAP = 0.2
/** Padding of the device body around the controls, in keys. */
export const BODY_PADDING = 0.35
/** Diameter of a dial, in keys. */
export const DIAL_SIZE = 0.9

/** The key size the editor's preview may take, in CSS pixels. */
export const KEY_SIZE = { min: 40, max: 104, fallback: 72 } as const

export interface DeviceGeometry {
  /** Key columns; the strip and dials span them. 4 for a device without keys. */
  columns: number
  rows: number
  /** Touch zones of the strip; 0 without one. */
  zones: number
  /** Dials; 0 without them. */
  dials: number
}

export function deviceGeometry(model: DeviceModel): DeviceGeometry {
  const { keys, columns, touchZones, encoders } = controlsOf(model)
  return {
    columns: columns > 0 ? columns : 4,
    rows: columns > 0 ? Math.ceil(keys.length / columns) : 0,
    zones: touchZones.length,
    dials: encoders.length,
  }
}

/** Width of the key grid (and so of the strip and the dial row), in keys. */
export function gridWidth(geometry: DeviceGeometry): number {
  return geometry.columns + (geometry.columns - 1) * KEY_GAP
}

/** Width and height of the whole device body, in keys. */
export function deviceSize(geometry: DeviceGeometry): { width: number; height: number } {
  const grid = gridWidth(geometry)
  const sections: number[] = []
  if (geometry.rows > 0) sections.push(geometry.rows + (geometry.rows - 1) * KEY_GAP)
  if (geometry.zones > 0) {
    // Zones are twice as wide as high, side by side across the grid, in a
    // strip with a small padding of its own.
    const inner = grid - 0.1 * 2
    const zone = (inner - (geometry.zones - 1) * 0.1) / geometry.zones
    sections.push(zone / 2 + 0.1 * 2)
  }
  if (geometry.dials > 0) sections.push(DIAL_SIZE)
  const height =
    sections.reduce((sum, h) => sum + h, 0) +
    Math.max(sections.length - 1, 0) * KEY_GAP * 1.5 +
    BODY_PADDING * 2
  return { width: grid + BODY_PADDING * 2, height }
}

/**
 * The key size (CSS pixels) at which the device fills the space it has:
 * the whole width, and at most `height` when given, within `bounds`. Below
 * the minimum the preview keeps the minimum and its container scrolls.
 */
export function keySize(
  space: { width: number; height?: number },
  geometry: DeviceGeometry,
  bounds: { min: number; max: number } = KEY_SIZE,
): number {
  const size = deviceSize(geometry)
  let fit = space.width / size.width
  if (space.height !== undefined && space.height > 0)
    fit = Math.min(fit, space.height / size.height)
  if (!Number.isFinite(fit) || fit <= 0) return bounds.min
  return Math.floor(Math.min(bounds.max, Math.max(bounds.min, fit)))
}
