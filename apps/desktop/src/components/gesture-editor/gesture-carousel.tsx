import { useDndMonitor, type DragMoveEvent } from "@dnd-kit/core"
import { ChevronLeft, ChevronRight } from "lucide-react"
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type ReactNode,
  type RefObject,
} from "react"

import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

import {
  CARD_GAP,
  DRAG_EDGE_DELAY_MS,
  DRAG_EDGE_ZONE,
  WHEEL_LOCK_MS,
  WHEEL_STEP,
  carouselGeometry,
} from "./carousel-geometry"

/** Where a drag's pointer is now (none for a keyboard drag). */
function pointerOf(event: DragMoveEvent): { x: number; y: number } | null {
  const start = event.activatorEvent
  let x: number | undefined
  let y: number | undefined
  if ("clientX" in start && typeof start.clientX === "number") {
    x = start.clientX
    y = (start as MouseEvent).clientY
  } else if ("touches" in start) {
    const touch = (start as TouchEvent).touches[0]
    x = touch?.clientX
    y = touch?.clientY
  }
  if (x === undefined || y === undefined) return null
  return { x: x + event.delta.x, y: y + event.delta.y }
}

/**
 * While something is dragged and rests at the left or right edge of the
 * cards, the next card in that direction slides in after a moment (and
 * again while it stays). Only inside a drag-and-drop context.
 */
function DragEdgeAdvance({
  area,
  viewport,
  onStep,
}: {
  area: RefObject<HTMLElement | null>
  viewport: RefObject<HTMLElement | null>
  onStep: (direction: -1 | 1) => boolean
}) {
  const zone = useRef<-1 | 0 | 1>(0)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const stop = () => {
    zone.current = 0
    if (timer.current) clearTimeout(timer.current)
    timer.current = null
  }
  const arm = (direction: -1 | 1) => {
    timer.current = setTimeout(() => {
      timer.current = null
      // Keep going while the drag rests there and there is more.
      if (zone.current === direction && onStep(direction)) arm(direction)
    }, DRAG_EDGE_DELAY_MS)
  }
  useEffect(() => stop, [])
  useDndMonitor({
    onDragMove: (event) => {
      const at = pointerOf(event)
      const outer = area.current?.getBoundingClientRect()
      const inner = viewport.current?.getBoundingClientRect()
      let next: -1 | 0 | 1 = 0
      if (at && outer && inner && at.y >= outer.top && at.y <= outer.bottom) {
        if (at.x >= outer.left && at.x <= inner.left + DRAG_EDGE_ZONE) next = -1
        else if (at.x <= outer.right && at.x >= inner.right - DRAG_EDGE_ZONE) next = 1
      }
      if (next === zone.current) return
      stop()
      zone.current = next
      if (next !== 0) arm(next)
    },
    onDragEnd: stop,
    onDragCancel: stop,
  })
  return null
}

/**
 * Cards side by side in one row: all of them where they fit, otherwise a
 * window of whole cards moved one card at a time by the arrows, a
 * horizontal swipe or Shift+wheel, by focus entering a card out of view
 * and by a drag resting at an edge. No scrollbar: the row is translated
 * inside a clipping container. Every card stays mounted and the row is as
 * tall as its tallest card, in view or not (flex stretch), so moving the
 * cards never changes the height; content changes in any card do.
 */
/**
 * A previous/next control: a rail as tall as the cards (the clickable area
 * stretches, the icon stays centred), shown only while the cards overflow.
 */
function Rail({
  slot,
  label,
  disabled,
  onClick,
  children,
}: {
  slot: string
  label: string
  disabled: boolean
  onClick: () => void
  children: ReactNode
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-nav"
      data-slot={slot}
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className="h-auto w-9 self-stretch rounded-lg bg-muted/40 hover:bg-accent focus-visible:ring-[3px]"
    >
      {children}
    </Button>
  )
}

export function GestureCarousel({
  label,
  count,
  renderCard,
  dragAware = false,
}: {
  /** The accessible name of the group of cards. */
  label: string
  count: number
  /** Card `index`; `visible`: inside the window (out of view it takes no drops). */
  renderCard: (index: number, visible: boolean) => ReactNode
  /** Inside a drag-and-drop context: a drag at an edge moves the cards. */
  dragAware?: boolean
}) {
  const { t } = useTranslation()
  const area = useRef<HTMLDivElement>(null)
  const viewport = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState<number | null>(null)
  const [start, setStart] = useState(0)

  useEffect(() => {
    const element = area.current
    if (!element || typeof ResizeObserver === "undefined") return
    const observer = new ResizeObserver((entries) => {
      const box = entries[0]?.contentRect
      if (box) setWidth((current) => (current === box.width ? current : box.width))
    })
    observer.observe(element)
    return () => {
      observer.disconnect()
    }
  }, [])

  const geometry = width === null ? null : carouselGeometry(width, count)
  const visible = geometry?.visible ?? count
  const last = Math.max(count - visible, 0)
  const first = Math.min(start, last)
  const overflowing = geometry?.overflowing ?? false

  // The latest values, for the listeners below.
  const live = useRef({ first, last, overflowing })
  useLayoutEffect(() => {
    live.current = { first, last, overflowing }
  })
  /** One card on; `false` at the end. */
  const step = useCallback((direction: -1 | 1): boolean => {
    const { first: from, last: to } = live.current
    const next = Math.min(Math.max(from + direction, 0), to)
    if (next === from) return false
    live.current = { ...live.current, first: next }
    setStart(next)
    return true
  }, [])

  // A horizontal swipe or Shift+wheel moves one card per gesture; the
  // vertical wheel keeps scrolling the inspector. Not passive, so the
  // browser's own horizontal navigation does not also happen.
  useEffect(() => {
    const element = area.current
    if (!element) return
    let total = 0
    let lockedUntil = 0
    const onWheel = (event: WheelEvent) => {
      if (!live.current.overflowing) return
      const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1
      const horizontal =
        Math.abs(event.deltaX) > Math.abs(event.deltaY)
          ? event.deltaX
          : event.shiftKey
            ? event.deltaY
            : 0
      if (horizontal === 0) return
      event.preventDefault()
      const now = performance.now()
      if (now < lockedUntil) return
      if (Math.sign(horizontal) !== Math.sign(total)) total = 0
      total += horizontal * scale
      if (Math.abs(total) < WHEEL_STEP) return
      step(total > 0 ? 1 : -1)
      total = 0
      lockedUntil = now + WHEEL_LOCK_MS
    }
    element.addEventListener("wheel", onWheel, { passive: false })
    return () => {
      element.removeEventListener("wheel", onWheel)
    }
  }, [step])

  // Focus entering a card out of view (Tab) brings that card in.
  const onFocus = (event: FocusEvent<HTMLDivElement>) => {
    if (!overflowing) return
    const item = (event.target as Element).closest<HTMLElement>("[data-carousel-item]")
    const index = Number(item?.dataset.carouselItem)
    if (!Number.isInteger(index)) return
    if (index < first) setStart(index)
    else if (index >= first + visible) setStart(index - visible + 1)
  }

  const offset = geometry ? first * (geometry.cardWidth + CARD_GAP) : 0
  return (
    <div
      ref={area}
      role="group"
      aria-label={label}
      aria-roledescription={overflowing ? t("editor:gesture.carousel.role") : undefined}
      data-slot="gesture-cards"
      data-overflowing={overflowing ? "" : undefined}
      className="relative flex w-full min-w-0 items-stretch gap-1"
    >
      {overflowing && (
        <Rail
          slot="gesture-cards-previous"
          label={t("editor:gesture.carousel.previous")}
          disabled={first === 0}
          onClick={() => {
            step(-1)
          }}
        >
          <ChevronLeft />
        </Rail>
      )}
      <div
        ref={viewport}
        data-slot="gesture-cards-viewport"
        // `clip`, not `hidden`: focus moving into a card out of view must not
        // scroll this box (the row is placed by its translation alone).
        className="relative min-w-0 flex-1 self-stretch overflow-clip"
        onFocus={onFocus}
      >
        <div
          data-slot="gesture-cards-track"
          className="flex items-stretch motion-safe:transition-transform motion-safe:duration-200 motion-safe:ease-out"
          style={{
            gap: CARD_GAP,
            transform: offset ? `translateX(${-offset}px)` : undefined,
          }}
        >
          {Array.from({ length: count }, (_, index) => {
            const shown = index >= first && index < first + visible
            return (
              <div
                key={index}
                data-carousel-item={index}
                data-visible={shown ? "" : undefined}
                className={cn("flex min-w-0 shrink-0 flex-col", !geometry && "flex-1 basis-0")}
                style={geometry ? { width: geometry.cardWidth } : undefined}
              >
                {renderCard(index, shown)}
              </div>
            )
          })}
        </div>
      </div>
      {overflowing && (
        <Rail
          slot="gesture-cards-next"
          label={t("editor:gesture.carousel.next")}
          disabled={first >= last}
          onClick={() => {
            step(1)
          }}
        >
          <ChevronRight />
        </Rail>
      )}
      {dragAware && overflowing && (
        <DragEdgeAdvance area={area} viewport={viewport} onStep={step} />
      )}
    </div>
  )
}
