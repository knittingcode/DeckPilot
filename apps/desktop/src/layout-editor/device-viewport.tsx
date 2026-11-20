import { Maximize, ZoomIn, ZoomOut } from "lucide-react"
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
  type Ref,
} from "react"

import { deviceGeometry } from "@/components/device-geometry"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { devicePixelRatio } from "@/hooks/use-device-pixel-ratio"
import { useTranslation } from "@/i18n"
import { controlsOf } from "@/lib/layout-model"
import type { DeviceModel } from "@/lib/types"

import {
  ARROW_PAN,
  CULL_THRESHOLD,
  ZOOM_LIMITS,
  TOOLBAR_OFFSET,
  clampPan,
  dividerHalf,
  fitAvoiding,
  fitInsets,
  toolbarLift,
  toolbarRect,
  fromSaved,
  keySizeAt,
  loadView,
  panBy,
  revealDelta,
  saveView,
  snap,
  stepZoom,
  toSaved,
  visibleRange,
  wheelPan,
  wheelUnit,
  wheelZoomFactor,
  zoomAt,
  zoomPercent,
  type Size,
  type View,
  type VisibleRange,
} from "./canvas-viewport"

/** How long the zoom must rest before key images are drawn again at the new size. */
export const SETTLE_MS = 150
/** How long the view must rest before it is remembered. */
const PERSIST_MS = 300

const ARROWS: Record<string, [number, number]> = {
  ArrowLeft: [1, 0],
  ArrowRight: [-1, 0],
  ArrowUp: [0, 1],
  ArrowDown: [0, -1],
}

/** Zoom presets of the percentage menu. */
const PRESETS = [0.5, 2, 4]

function editable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  )
}

/** Whether a (React) event comes from inside the canvas' own element, not a portal. */
function inCanvas(event: { currentTarget: Element; target: EventTarget }): boolean {
  return event.target instanceof Node && event.currentTarget.contains(event.target)
}

function sameRange(a: VisibleRange | null, b: VisibleRange | null): boolean {
  if (!a || !b) return a === b
  return (
    a.rows[0] === b.rows[0] &&
    a.rows[1] === b.rows[1] &&
    a.columns[0] === b.columns[0] &&
    a.columns[1] === b.columns[1]
  )
}

interface GestureEvent extends UIEvent {
  scale: number
  clientX: number
  clientY: number
}

/**
 * The Device Editor's canvas: a region of fixed height (never scrolling,
 * whatever the device) in which the device is zoomed and panned, with the
 * page navigation and a small zoom toolbar overlaid at its bottom, neither
 * of them scaled.
 *
 * Zooming changes the key size the device is laid out at (`renderCanvas`
 * gets it in whole CSS pixels), so keys and their images are drawn at the
 * size they are shown — never a scaled raster. `onKeySizeSettled` reports
 * it once the zoom has rested (`SETTLE_MS`), for the key images to be
 * rendered again at that size; meanwhile the previous ones are stretched.
 * Panning is a translation of the device, in whole device pixels, written
 * straight to the element (no React render per frame).
 *
 * Input (see canvas-viewport.ts for the geometry):
 * - drag the background, Space+drag anywhere, middle-button drag: pan;
 * - wheel / two-finger scroll: pan; Ctrl/Cmd+wheel and pinch: zoom at the
 *   pointer (also Safari's `gesturechange`);
 * - with the canvas focused: `+`/`-` zoom, `0` 100 %, `1` (or Shift+1) fit;
 *   arrows pan while the background itself has focus (on a key they move
 *   the selection, DeviceCanvas);
 * - a key receiving keyboard focus outside the view is panned into it.
 * Pans never select a key or start a drag: they start only on the
 * background, with Space or with the middle button, and a click that ends
 * a pan does not reach the key.
 */
export function DeviceViewport({
  model,
  renderCanvas,
  footer,
  status,
  onKeySizeSettled,
}: {
  model: DeviceModel
  /** The device at `keySize`; `visible`: the keys in view on a large grid (else `null`). */
  renderCanvas: (keySize: number, visible: VisibleRange | null) => ReactNode
  /** Overlaid at the bottom, unscaled: the page navigation. */
  footer?: ReactNode
  /** A quiet note at the left of the divider (the state of live values). */
  status?: ReactNode
  onKeySizeSettled?: (keySize: number) => void
}) {
  const { t } = useTranslation()
  const hint = useId()
  const geometry = useMemo(() => deviceGeometry(model), [model])
  const cull = useMemo(() => controlsOf(model).keys.length > CULL_THRESHOLD, [model])
  const viewport = useRef<HTMLDivElement>(null)
  const grid = useRef<HTMLDivElement>(null)
  const footerRef = useRef<HTMLDivElement>(null)
  const toolbarRef = useRef<HTMLDivElement>(null)

  // The view lives in refs: panning writes the transform directly.
  const view = useRef<View>({ zoom: 1, x: 0, y: 0 })
  const fitting = useRef(true)
  const size = useRef<Size>({ width: 0, height: 0 })
  const animate = useRef(false)
  const frame = useRef(0)
  const persistTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const [zoom, setZoom] = useState(1)
  const rendered = useRef(1)
  const [range, setRange] = useState<VisibleRange | null>(null)
  const [placed, setPlaced] = useState(false)

  // The page navigation and the zoom toolbar straddle the divider between
  // the canvas and the inspector (half above, half below): the lower half of
  // their height is kept free of keys at the bottom of the viewport.
  const half = useRef(0)
  const bottomInset = useCallback(() => half.current, [])
  const corner = useRef<HTMLDivElement>(null)
  /** The size of the page navigation on the divider. */
  const navSize = useCallback(() => {
    const nav = footerRef.current?.querySelector<HTMLElement>("[data-slot=canvas-nav]")
    return nav ? { width: nav.offsetWidth, height: nav.offsetHeight } : null
  }, [])
  /** `--canvas-overlay-inset` in pixels, as the corner resolves it. */
  const overlayInset = useCallback(() => {
    const element = corner.current
    const value = element ? Number.parseFloat(getComputedStyle(element).right) : Number.NaN
    return Number.isFinite(value) ? value : TOOLBAR_OFFSET
  }, [])
  /** Lifts the zoom toolbar only when the page navigation would be under it. */
  const placeToolbar = useCallback(() => {
    const element = corner.current
    const bar = toolbarRef.current
    if (!element || !bar) return
    const lift = toolbarLift(
      { width: bar.offsetWidth, height: bar.offsetHeight },
      size.current,
      navSize(),
      overlayInset(),
    )
    element.style.bottom = lift > 0 ? `calc(var(--canvas-overlay-inset) + ${lift}px)` : ""
  }, [navSize, overlayInset])

  /** Writes the view to the element: translation, culled range, data for the probe. */
  const apply = useCallback(() => {
    const v = view.current
    const element = grid.current
    if (element) {
      const ratio = devicePixelRatio()
      element.style.transform = `translate(${snap(v.x, ratio)}px, ${snap(v.y, ratio)}px)`
      element.toggleAttribute("data-animate", animate.current)
    }
    if (cull) {
      const next = visibleRange(v, geometry, size.current)
      setRange((current) => (sameRange(current, next) ? current : next))
    }
  }, [cull, geometry])

  const schedule = useCallback(() => {
    if (frame.current) return
    const run = () => {
      frame.current = 0
      // A new zoom is applied with its layout (below), not a frame before it.
      if (rendered.current === view.current.zoom) apply()
    }
    if (typeof requestAnimationFrame === "function") frame.current = requestAnimationFrame(run)
    else run()
  }, [apply])

  const persist = useCallback(() => {
    clearTimeout(persistTimer.current)
    persistTimer.current = setTimeout(() => {
      if (size.current.width > 0) {
        saveView(model.id, toSaved(view.current, size.current, fitting.current))
      }
    }, PERSIST_MS)
  }, [model.id])

  /** Moves to `next` (kept within the pan bounds). */
  const commit = useCallback(
    (next: View, options: { fit?: boolean; animate?: boolean; remember?: boolean } = {}) => {
      const clamped = clampPan(next, geometry, size.current)
      animate.current = Boolean(options.animate) && clamped.zoom === view.current.zoom
      view.current = clamped
      fitting.current = options.fit ?? false
      setZoom(clamped.zoom)
      // At the zoom already laid out (a pan, a fit at the same zoom) the
      // transform is written now: a style write without a render, and
      // pointer moves arrive at most once per frame anyway. Waiting for an
      // animation frame here lost every later update once one frame never
      // ran — WebKit runs none while the window is hidden (the main window
      // starts hidden until the configuration is unlocked) — and the device
      // could then no longer be dragged or fitted back into view. A new
      // zoom is applied with its layout (the effect below).
      if (rendered.current === clamped.zoom) apply()
      else schedule()
      if (options.remember !== false) persist()
    },
    [geometry, apply, schedule, persist],
  )

  const fit = useCallback(
    (options: { animate?: boolean; remember?: boolean } = {}) => {
      const bar = toolbarRef.current
      const exclusion = bar
        ? toolbarRect(
            { width: bar.offsetWidth, height: bar.offsetHeight },
            size.current,
            navSize(),
            overlayInset(),
          )
        : null
      commit(fitAvoiding(geometry, size.current, fitInsets(bottomInset()), exclusion), {
        ...options,
        fit: true,
      })
    },
    [commit, geometry, bottomInset, navSize, overlayInset],
  )

  /** The point zooms without a pointer keep in place: the centre above the footer. */
  const centre = useCallback(
    () => ({ x: size.current.width / 2, y: (size.current.height - bottomInset()) / 2 }),
    [bottomInset],
  )
  const zoomTo = useCallback(
    (next: number, point?: { x: number; y: number }) => {
      commit(zoomAt(view.current, next, point ?? centre(), geometry, size.current))
    },
    [commit, geometry, centre],
  )

  // Placement: the model's remembered view, else Fit to screen. Again when
  // the model changes (the editor is usually remounted for that).
  const place = useCallback(() => {
    const saved = loadView(model.id)
    if (saved?.mode === "manual" && size.current.width > 0 && size.current.height > 0) {
      commit(fromSaved(saved, geometry, size.current), { remember: false })
    } else if (size.current.width > 0 && size.current.height > 0) {
      fit({ remember: false })
    } else {
      // Not laid out (hidden, or jsdom): 100 %, fitted once it has a size.
      commit(
        { zoom: saved?.mode === "manual" ? saved.zoom : 1, x: 0, y: 0 },
        { fit: true, remember: false },
      )
    }
  }, [commit, fit, geometry, model.id])

  // The divider bar's height, measured: half of it is reserved below the
  // device (Fit to screen) and, through `--canvas-divider-half` on the
  // editor's main area, at the top of the inspector.
  useLayoutEffect(() => {
    const bar = footerRef.current
    const host = bar?.parentElement?.parentElement
    if (!bar || !host) return
    const measure = () => {
      const next = dividerHalf(bar.offsetHeight)
      // The navigation's width changes with the page (a folder's path).
      placeToolbar()
      if (next === half.current && host.style.getPropertyValue("--canvas-divider-half")) return
      half.current = next
      host.style.setProperty("--canvas-divider-half", `${next}px`)
      if (fitting.current && size.current.width > 0) fit({ remember: false })
    }
    measure()
    if (typeof ResizeObserver === "undefined") return
    const observer = new ResizeObserver(measure)
    observer.observe(bar)
    return () => {
      observer.disconnect()
      host.style.removeProperty("--canvas-divider-half")
    }
  }, [fit, placeToolbar])

  const settled = useRef(onKeySizeSettled)
  useLayoutEffect(() => {
    settled.current = onKeySizeSettled
  })

  useLayoutEffect(() => {
    const element = viewport.current
    if (!element) return
    size.current = { width: element.clientWidth, height: element.clientHeight }
    place()
    // The first size at once, so the first key images already have it.
    settled.current?.(keySizeAt(view.current.zoom))
    setPlaced(true)
    if (typeof ResizeObserver === "undefined") return
    const observer = new ResizeObserver(() => {
      const before = size.current
      const now = { width: element.clientWidth, height: element.clientHeight }
      if (now.width === before.width && now.height === before.height) return
      size.current = now
      placeToolbar()
      if (before.width <= 0 || before.height <= 0) place()
      else if (fitting.current) fit({ remember: false })
      else commit(view.current, { remember: false })
    })
    observer.observe(element)
    return () => {
      observer.disconnect()
    }
  }, [place, fit, commit, placeToolbar])

  // A zoom is laid out by this render: its translation goes with it.
  useLayoutEffect(() => {
    rendered.current = zoom
    apply()
  }, [zoom, apply])

  // Key images are drawn again at the new size once the zoom rests.
  const first = useRef(true)
  useEffect(() => {
    if (first.current) {
      first.current = false
      return
    }
    const timer = setTimeout(() => settled.current?.(keySizeAt(zoom)), SETTLE_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [zoom])

  useEffect(
    () => () => {
      if (frame.current && typeof cancelAnimationFrame === "function")
        cancelAnimationFrame(frame.current)
      clearTimeout(persistTimer.current)
    },
    [],
  )

  // Wheel and pinch: native listeners, since React's wheel listener is
  // passive and cannot keep the window from scrolling or zooming.
  const gesture = useRef<{ zoom: number } | null>(null)
  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const local = (event: { clientX: number; clientY: number }) => {
      const r = element.getBoundingClientRect()
      return { x: event.clientX - r.left, y: event.clientY - r.top }
    }
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      if (event.ctrlKey || event.metaKey) {
        if (gesture.current) return
        const delta = event.deltaY * wheelUnit(event.deltaMode, size.current.height)
        zoomTo(view.current.zoom * wheelZoomFactor(delta), local(event))
      } else {
        const { dx, dy } = wheelPan(event, size.current.height)
        commit(panBy(view.current, dx, dy, geometry, size.current))
      }
    }
    // Safari (WKWebView) reports a trackpad pinch as gesture events.
    const onGestureStart = (event: Event) => {
      event.preventDefault()
      gesture.current = { zoom: view.current.zoom }
    }
    const onGestureChange = (event: Event) => {
      event.preventDefault()
      const g = event as GestureEvent
      const base = gesture.current?.zoom ?? view.current.zoom
      zoomTo(base * (Number.isFinite(g.scale) ? g.scale : 1), local(g))
    }
    const onGestureEnd = (event: Event) => {
      event.preventDefault()
      gesture.current = null
    }
    element.addEventListener("wheel", onWheel, { passive: false })
    element.addEventListener("gesturestart", onGestureStart)
    element.addEventListener("gesturechange", onGestureChange)
    element.addEventListener("gestureend", onGestureEnd)
    return () => {
      element.removeEventListener("wheel", onWheel)
      element.removeEventListener("gesturestart", onGestureStart)
      element.removeEventListener("gesturechange", onGestureChange)
      element.removeEventListener("gestureend", onGestureEnd)
    }
  }, [commit, geometry, zoomTo])

  // Space held over the canvas: dragging anywhere pans.
  const inside = useRef(false)
  const space = useRef<{ used: boolean } | null>(null)
  useEffect(() => {
    const down = (event: globalThis.KeyboardEvent) => {
      if (event.code !== "Space" || event.repeat || editable(event.target)) return
      if (!inside.current) return
      space.current = { used: false }
      viewport.current?.setAttribute("data-space", "")
      // Space on the background would otherwise do nothing but scroll.
      if (event.target === viewport.current) event.preventDefault()
    }
    const up = (event: globalThis.KeyboardEvent) => {
      if (event.code !== "Space" || !space.current) return
      // Space that panned does not also press the focused key.
      if (space.current.used) {
        event.preventDefault()
        event.stopPropagation()
      }
      space.current = null
      viewport.current?.removeAttribute("data-space")
    }
    window.addEventListener("keydown", down, true)
    window.addEventListener("keyup", up, true)
    return () => {
      window.removeEventListener("keydown", down, true)
      window.removeEventListener("keyup", up, true)
    }
  }, [])

  const drag = useRef<{ id: number; x: number; y: number; from: View } | null>(null)
  // The click that ends a pan started on a key is not a click on the key.
  const swallowClick = useRef(false)

  // When a pointer last went down: focus that follows a click does not pan.
  const pointerAt = useRef(0)

  const onPointerDownCapture = (event: PointerEvent<HTMLDivElement>) => {
    swallowClick.current = false
    // Menus of the keys are portalled elsewhere in the document but their
    // events still bubble here through React: only the canvas itself pans.
    if (!inCanvas(event)) return
    pointerAt.current = event.timeStamp
    // Focus that a pointer gives shows no ring (WebKit counts programmatic
    // focus as focus-visible); the keyboard clears this again (onKeyDown).
    event.currentTarget.setAttribute("data-pointer-focus", "")
    const onControl = (event.target as Element).closest("[data-control]") !== null
    const middle = event.button === 1
    const primary = event.button === 0
    if (!(middle || (primary && (space.current !== null || !onControl)))) return
    // No text selection, no autoscroll, and nothing for the key below
    // (selection, dnd-kit's drag) when the pan starts on one.
    event.preventDefault()
    if (onControl) {
      event.stopPropagation()
      swallowClick.current = true
    }
    if (space.current) space.current.used = true
    const element = event.currentTarget
    if (!onControl) element.focus({ preventScroll: true, focusVisible: false })
    try {
      element.setPointerCapture(event.pointerId)
    } catch {
      // Not every pointer can be captured (synthetic events in tests).
    }
    drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, from: view.current }
    element.setAttribute("data-panning", "")
  }
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const d = drag.current
    if (!d || d.id !== event.pointerId) return
    commit(panBy(d.from, event.clientX - d.x, event.clientY - d.y, geometry, size.current))
  }
  /** Ends a pan, however it ends (pointerup, pointercancel, lost capture, window blur). */
  const stopPan = useCallback(() => {
    const d = drag.current
    if (!d) return
    drag.current = null
    const element = viewport.current
    if (!element) return
    element.removeAttribute("data-panning")
    try {
      if (element.hasPointerCapture(d.id)) element.releasePointerCapture(d.id)
    } catch {
      // Already released.
    }
  }, [])
  const endPan = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag.current || drag.current.id !== event.pointerId) return
    stopPan()
  }
  // Leaving the window mid-pan (or with Space held) leaves nothing stuck.
  useEffect(() => {
    const blur = () => {
      stopPan()
      space.current = null
      viewport.current?.removeAttribute("data-space")
    }
    window.addEventListener("blur", blur)
    return () => {
      window.removeEventListener("blur", blur)
    }
  }, [stopPan])
  const onClickCapture = (event: MouseEvent<HTMLDivElement>) => {
    if (!swallowClick.current) return
    swallowClick.current = false
    event.preventDefault()
    event.stopPropagation()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!inCanvas(event)) return
    // Keyboard use: focus shows its ring again.
    event.currentTarget.removeAttribute("data-pointer-focus")
    if (event.ctrlKey || event.metaKey || event.altKey || editable(event.target)) return
    const key = event.key
    if (key === "+" || key === "=") zoomTo(stepZoom(view.current.zoom, 1))
    else if (key === "-" || key === "_") zoomTo(stepZoom(view.current.zoom, -1))
    else if (key === "0") zoomTo(1)
    else if (key === "1" || key === "!" || event.code === "Digit1") fit({ animate: true })
    else if (ARROWS[key] && event.target === event.currentTarget) {
      const [dx, dy] = ARROWS[key]
      const step = event.shiftKey ? ARROW_PAN * 4 : ARROW_PAN
      commit(panBy(view.current, dx * step, dy * step, geometry, size.current), { animate: true })
    } else return
    event.preventDefault()
  }

  // A key focused (Tab, arrows) outside the view is brought into it.
  const onFocus = (event: FocusEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement
    if (!inCanvas(event) || !target.closest("[data-control]") || drag.current) return
    if (pointerAt.current && event.timeStamp - pointerAt.current < 500) return
    const box = event.currentTarget.getBoundingClientRect()
    const r = target.getBoundingClientRect()
    const { dx, dy } = revealDelta(
      {
        left: r.left - box.left,
        top: r.top - box.top,
        right: r.right - box.left,
        bottom: r.bottom - box.top,
      },
      size.current,
      12,
      bottomInset(),
    )
    if (dx !== 0 || dy !== 0)
      commit(panBy(view.current, dx, dy, geometry, size.current), { animate: true })
  }

  const percent = zoomPercent(zoom)
  return (
    <>
      <div
        data-slot="device-workspace"
        className="relative flex h-[clamp(220px,45%,520px)] min-h-[220px] w-full flex-none flex-col overflow-hidden"
      >
        {/*
        Only the device moves in the viewport (translated, never scaled);
        the canvas controls are on the divider below, a layer of their own.
      */}
        <div
          ref={viewport}
          data-slot="device-viewport"
          data-zoom={percent}
          data-key-size={keySizeAt(zoom)}
          tabIndex={0}
          role="group"
          aria-roledescription={t("editor:canvas.roleDescription")}
          aria-label={t("editor:canvas.viewport")}
          aria-describedby={hint}
          className={
            "relative min-h-0 flex-1 cursor-grab touch-none overflow-hidden outline-none select-none " +
            // A ring for keyboard focus only (not after a click or a pan).
            "focus-visible:not-data-[pointer-focus]:ring-2 focus-visible:not-data-[pointer-focus]:ring-ring focus-visible:not-data-[pointer-focus]:ring-inset " +
            "data-[panning]:cursor-grabbing [&[data-panning]_*]:cursor-grabbing [&[data-space]_*]:cursor-grab"
          }
          onPointerDownCapture={onPointerDownCapture}
          onPointerMove={onPointerMove}
          onPointerUp={endPan}
          onPointerCancel={endPan}
          onLostPointerCapture={endPan}
          onClickCapture={onClickCapture}
          onPointerEnter={() => {
            inside.current = true
          }}
          onPointerLeave={() => {
            inside.current = false
          }}
          onKeyDown={onKeyDown}
          onFocus={onFocus}
          onBlur={(event) => {
            // Focus that comes back by Tab shows its ring.
            if (!event.currentTarget.contains(event.relatedTarget))
              event.currentTarget.removeAttribute("data-pointer-focus")
          }}
        >
          {/*
          Clips the moved device without being scrollable (`overflow: clip`),
          so focusing a key outside the view cannot scroll it: such a key is
          panned into view instead (onFocus).
        */}
          <div
            data-slot="device-stage"
            className="absolute inset-0 overflow-hidden supports-[overflow:clip]:overflow-clip"
            onScroll={(event) => {
              event.currentTarget.scrollTop = 0
              event.currentTarget.scrollLeft = 0
            }}
          >
            <div
              ref={grid}
              data-slot="device-grid"
              className="absolute top-0 left-0 will-change-transform motion-safe:data-[animate]:transition-transform motion-safe:data-[animate]:duration-100 motion-safe:data-[animate]:ease-out"
              style={placed ? undefined : { visibility: "hidden" }}
            >
              {renderCanvas(keySizeAt(zoom), cull ? range : null)}
            </div>
          </div>
        </div>
        {/*
          The zoom toolbar: in the viewport's bottom-right corner, the same
          inset from its right and bottom edges (the bottom edge is the
          divider), anchored to the canvas region — not the moved device, the
          divider or the inspector. Lifted only in a window so narrow that
          the page navigation would be under it (placeToolbar).
        */}
        <div
          ref={corner}
          data-slot="canvas-toolbar-corner"
          className="absolute right-(--canvas-overlay-inset) bottom-(--canvas-overlay-inset)"
        >
          <CanvasToolbar
            ref={toolbarRef}
            percent={percent}
            zoom={zoom}
            onZoomIn={() => {
              zoomTo(stepZoom(view.current.zoom, 1))
            }}
            onZoomOut={() => {
              zoomTo(stepZoom(view.current.zoom, -1))
            }}
            onZoom={(z) => {
              zoomTo(z)
            }}
            onFit={() => {
              fit({ animate: true })
            }}
          />
        </div>
        <p id={hint} className="sr-only">
          {t("editor:canvas.viewportHint")}
        </p>
      </div>
      {/*
      The divider between the canvas and the inspector: a row of no height
      whose top edge is the line. The bar on it is moved up by half its own
      height, so the page navigation (centred) and the zoom toolbar (at the
      right) straddle the line exactly, above it in stacking order; neither
      moves with the device nor scrolls with the inspector.
    */}
      <div data-slot="canvas-divider" className="relative z-20 h-0 w-full flex-none">
        <div aria-hidden className="absolute inset-x-0 top-0 h-px -translate-y-1/2 bg-border" />
        <div
          ref={footerRef}
          data-slot="canvas-footer"
          className="pointer-events-none absolute inset-x-0 top-0 grid -translate-y-1/2 grid-cols-[1fr_auto_1fr] items-center gap-2 px-3"
        >
          <div className="pointer-events-auto min-w-0 justify-self-start truncate rounded-md bg-popover/65 px-2 py-0.5 empty:hidden">
            {status}
          </div>
          <div data-slot="canvas-nav" className="pointer-events-auto col-start-2 min-w-0">
            {footer}
          </div>
        </div>
      </div>
    </>
  )
}

function CanvasToolbar({
  ref,
  percent,
  zoom,
  onZoomIn,
  onZoomOut,
  onZoom,
  onFit,
}: {
  ref?: Ref<HTMLDivElement>
  percent: number
  zoom: number
  onZoomIn: () => void
  onZoomOut: () => void
  onZoom: (zoom: number) => void
  onFit: () => void
}) {
  const { t } = useTranslation()
  const titled = (label: string, shortcut: string) =>
    t("navigation:toolbar.withShortcut", { label, shortcut })
  return (
    <div
      ref={ref}
      role="toolbar"
      aria-label={t("editor:canvas.toolbar")}
      data-slot="canvas-toolbar"
      // The background only is translucent (icons and text stay opaque);
      // opaque while pointed at or focused.
      className="flex items-center gap-0.5 rounded-lg border bg-popover/65 p-0.5 focus-within:bg-popover hover:bg-popover motion-safe:transition-[background-color] motion-safe:duration-150"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        data-canvas-action="zoom-out"
        aria-label={t("editor:canvas.zoomOut")}
        title={titled(t("editor:canvas.zoomOut"), "−")}
        disabled={zoom <= ZOOM_LIMITS.min + 1e-6}
        onClick={onZoomOut}
      >
        <ZoomOut />
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            data-canvas-action="zoom-menu"
            className="h-8 min-w-14 px-2 font-medium tabular-nums"
            aria-label={t("editor:canvas.zoomLevel", { percent })}
            title={t("editor:canvas.zoomMenu")}
          >
            {t("editor:canvas.percent", { percent })}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" side="top" className="min-w-48">
          <DropdownMenuItem
            data-canvas-action="actual"
            onSelect={() => {
              onZoom(1)
            }}
          >
            {t("editor:canvas.actualSize")}
            <DropdownMenuShortcut>0</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={onFit}>
            {t("editor:canvas.fit")}
            <DropdownMenuShortcut>1</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          {PRESETS.map((preset) => (
            <DropdownMenuItem
              key={preset}
              onSelect={() => {
                onZoom(preset)
              }}
            >
              {t("editor:canvas.percent", { percent: Math.round(preset * 100) })}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <Button
        variant="ghost"
        size="icon-sm"
        data-canvas-action="zoom-in"
        aria-label={t("editor:canvas.zoomIn")}
        title={titled(t("editor:canvas.zoomIn"), "+")}
        disabled={zoom >= ZOOM_LIMITS.max - 1e-6}
        onClick={onZoomIn}
      >
        <ZoomIn />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        data-canvas-action="fit"
        aria-label={t("editor:canvas.fit")}
        title={titled(t("editor:canvas.fit"), "1")}
        onClick={onFit}
      >
        <Maximize />
      </Button>
    </div>
  )
}
