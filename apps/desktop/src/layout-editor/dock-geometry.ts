// The outline of the expanded page navigation dock (page-nav.tsx): the
// toolbar, unchanged, on top of a wider panel, one connected shape with
// symmetrical shoulders, drawn from measured sizes.

/** The panel's preferred width when expanded, in CSS pixels (26rem). */
export const PANEL_WIDTH = 416
/** The narrowest shoulder worth drawing; narrower ones join straight. */
export const MIN_SHOULDER = 16

export interface DockGeometry {
  /** The panel's width; at least the toolbar's. */
  width: number
  /** The panel's left edge against the toolbar's (≤ 0: it sticks out to the left). */
  offset: number
  /** Each side's step from the toolbar's edge to the panel's (0: a straight join). */
  shoulder: number
}

/**
 * The panel under a toolbar `bar` wide: the preferred width within `room`
 * (the width the central area allows), never narrower than the toolbar,
 * centred under it; a step too small to draw cleanly becomes a straight
 * join (the panel as wide as the toolbar).
 */
export function dockGeometry(bar: number, room: number, preferred = PANEL_WIDTH): DockGeometry {
  const wanted = Math.floor(Math.min(preferred, room))
  const shoulder = Math.floor((wanted - bar) / 2)
  if (shoulder < MIN_SHOULDER) return { width: bar, offset: 0, shoulder: 0 }
  return { width: bar + 2 * shoulder, offset: -shoulder, shoulder }
}

const n = (value: number) => Math.round(value * 100) / 100

/**
 * The SVG path of the outline, in the panel's coordinates (x from its left
 * edge, y from the toolbar's top): the toolbar `barWidth` × `barHeight`
 * centred on a panel `geometry.width` wide, `height` the two together.
 * Outer corners have `radius`; the shoulders' inner corners a concave
 * fillet. Drawn half a pixel in, so a 1px stroke stays crisp.
 */
export function dockOutline(
  geometry: DockGeometry,
  barWidth: number,
  barHeight: number,
  height: number,
  radius: number,
): string {
  const i = 0.5
  const W = geometry.width
  const H = height
  const R = Math.max(0, Math.min(radius, barHeight / 2, (H - barHeight) / 2))
  if (geometry.shoulder === 0) {
    // A straight join: one rounded rectangle.
    return [
      `M ${n(i + R)} ${i}`,
      `H ${n(W - i - R)}`,
      `A ${n(R)} ${n(R)} 0 0 1 ${n(W - i)} ${n(i + R)}`,
      `V ${n(H - i - R)}`,
      `A ${n(R)} ${n(R)} 0 0 1 ${n(W - i - R)} ${n(H - i)}`,
      `H ${n(i + R)}`,
      `A ${n(R)} ${n(R)} 0 0 1 ${i} ${n(H - i - R)}`,
      `V ${n(i + R)}`,
      `A ${n(R)} ${n(R)} 0 0 1 ${n(i + R)} ${i}`,
      "Z",
    ].join(" ")
  }
  const s = geometry.shoulder
  const b = barWidth
  const h = barHeight + i
  // The fillet: as round as the corners, within the shoulder and the bar.
  const f = Math.max(0, Math.min(R, s / 2, barHeight / 2))
  return [
    `M ${n(s + i + R)} ${i}`,
    `H ${n(s + b - i - R)}`,
    `A ${n(R)} ${n(R)} 0 0 1 ${n(s + b - i)} ${n(i + R)}`,
    `V ${n(h - f)}`,
    `A ${n(f)} ${n(f)} 0 0 0 ${n(s + b - i + f)} ${n(h)}`,
    `H ${n(W - i - R)}`,
    `A ${n(R)} ${n(R)} 0 0 1 ${n(W - i)} ${n(h + R)}`,
    `V ${n(H - i - R)}`,
    `A ${n(R)} ${n(R)} 0 0 1 ${n(W - i - R)} ${n(H - i)}`,
    `H ${n(i + R)}`,
    `A ${n(R)} ${n(R)} 0 0 1 ${i} ${n(H - i - R)}`,
    `V ${n(h + R)}`,
    `A ${n(R)} ${n(R)} 0 0 1 ${n(i + R)} ${n(h)}`,
    `H ${n(s + i - f)}`,
    `A ${n(f)} ${n(f)} 0 0 0 ${n(s + i)} ${n(h - f)}`,
    `V ${n(i + R)}`,
    `A ${n(R)} ${n(R)} 0 0 1 ${n(s + i + R)} ${i}`,
    "Z",
  ].join(" ")
}
