// The geometry of the gesture cards' row (gesture-carousel.tsx).

/** The narrowest card whose form is still readable (17rem). */
export const MIN_CARD_WIDTH = 272
/** Between two cards. */
export const CARD_GAP = 12
/** A navigation rail (36px) and its distance to the cards. */
const ARROW_GUTTER = 36 + 4
/** How long a drag rests at an edge before the next card slides in. */
export const DRAG_EDGE_DELAY_MS = 600
/** How far from the cards' edge a drag counts as resting there. */
export const DRAG_EDGE_ZONE = 40
/** Horizontal wheel distance that moves one card. */
export const WHEEL_STEP = 30
/** After a move, the rest of a swipe (its inertia) is ignored this long. */
export const WHEEL_LOCK_MS = 350

/** How many cards fit side by side, and how wide each is. */
export interface CarouselGeometry {
  /** Cards shown at once (all, when they fit). */
  visible: number
  cardWidth: number
  overflowing: boolean
}

/**
 * The cards' geometry in `width` px: all of them when each gets at least
 * `MIN_CARD_WIDTH`; otherwise as many whole cards as fit between the two
 * arrow buttons (at least one), the arrows taking their room first.
 */
export function carouselGeometry(width: number, count: number): CarouselGeometry {
  const fits = (n: number, room: number) => n * MIN_CARD_WIDTH + (n - 1) * CARD_GAP <= room
  if (count <= 1 || fits(count, width)) {
    const visible = Math.max(count, 1)
    return {
      visible,
      cardWidth: (width - (visible - 1) * CARD_GAP) / visible,
      overflowing: false,
    }
  }
  const room = Math.max(width - 2 * ARROW_GUTTER, 0)
  const visible = Math.max(1, Math.floor((room + CARD_GAP) / (MIN_CARD_WIDTH + CARD_GAP)))
  return { visible, cardWidth: (room - (visible - 1) * CARD_GAP) / visible, overflowing: true }
}
