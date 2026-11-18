import { Library, PanelRightClose, PanelRightOpen, X } from "lucide-react"
import {
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from "react"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

import {
  LIBRARY_OVERLAY_BELOW,
  LIBRARY_STEP,
  LIBRARY_WIDTH,
  clampLibraryWidth,
  loadLibraryCollapsed,
  loadLibraryWidth,
  saveLibraryCollapsed,
  saveLibraryWidth,
} from "./library-sidebar"

const NARROW = `(max-width: ${LIBRARY_OVERLAY_BELOW - 1}px)`

function subscribeNarrow(listener: () => void) {
  if (typeof window.matchMedia !== "function") return () => undefined
  const media = window.matchMedia(NARROW)
  media.addEventListener("change", listener)
  return () => {
    media.removeEventListener("change", listener)
  }
}

function isNarrow() {
  return typeof window.matchMedia === "function" && window.matchMedia(NARROW).matches
}

/** Whether the window is too narrow for the library beside the editor. */
function useNarrowEditor() {
  return useSyncExternalStore(subscribeNarrow, isNarrow, () => false)
}

/**
 * The Action Library at the right of the device editor: resizable (drag
 * or arrow keys on its edge) and collapsible, both remembered. In narrow
 * windows it opens as an overlay above the editor instead; dragging from
 * it onto the device still works, as it stays inside the editor's drag
 * and drop context.
 */
export function LibraryDock({ children }: { children: ReactNode }) {
  const narrow = useNarrowEditor()
  const [width, setWidth] = useState(loadLibraryWidth)
  const [collapsed, setCollapsedState] = useState(loadLibraryCollapsed)
  const [overlay, setOverlay] = useState(false)
  const drag = useRef<{ x: number; width: number } | null>(null)
  const id = useId()

  const setCollapsed = (value: boolean) => {
    setCollapsedState(value)
    saveLibraryCollapsed(value)
  }
  const resize = (next: number) => {
    const clamped = clampLibraryWidth(next)
    setWidth(clamped)
    saveLibraryWidth(clamped)
  }

  // Escape closes the overlay.
  useEffect(() => {
    if (!narrow || !overlay) return
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") setOverlay(false)
    }
    window.addEventListener("keydown", onKey)
    return () => {
      window.removeEventListener("keydown", onKey)
    }
  }, [narrow, overlay])

  const header = (action: ReactNode) => (
    <div className="flex h-11 shrink-0 items-center gap-2 border-b px-3">
      <Library className="size-4 text-muted-foreground" aria-hidden />
      <h2 className="flex-1 text-sm font-semibold">Library</h2>
      {action}
    </div>
  )

  if (narrow) {
    return (
      <>
        <Button
          variant="outline"
          size="sm"
          className="absolute top-3 right-3 z-20 shadow-sm"
          aria-expanded={overlay}
          aria-controls={id}
          onClick={() => {
            setOverlay(!overlay)
          }}
        >
          <Library /> Library
        </Button>
        {overlay && (
          <aside
            id={id}
            aria-label="Action Library"
            data-layout="overlay"
            className="absolute inset-y-0 right-0 z-30 flex min-h-0 w-[min(20rem,90%)] flex-col border-l bg-background shadow-xl"
          >
            {header(
              <Button
                variant="ghost"
                size="icon"
                className="size-7"
                aria-label="Close the library"
                onClick={() => {
                  setOverlay(false)
                }}
              >
                <X />
              </Button>,
            )}
            <div className="min-h-0 flex-1 overflow-y-auto p-3">{children}</div>
          </aside>
        )}
      </>
    )
  }

  if (collapsed) {
    return (
      <aside
        aria-label="Action Library"
        data-layout="collapsed"
        className="flex w-10 shrink-0 flex-col items-center border-l bg-sidebar py-2"
      >
        <Button
          variant="ghost"
          size="icon"
          className="size-7"
          aria-label="Show the library"
          aria-expanded={false}
          title="Show the library"
          onClick={() => {
            setCollapsed(false)
          }}
        >
          <PanelRightOpen />
        </Button>
      </aside>
    )
  }

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    drag.current = { x: event.clientX, width }
    event.currentTarget.setPointerCapture(event.pointerId)
  }
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return
    setWidth(clampLibraryWidth(drag.current.width + drag.current.x - event.clientX))
  }
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return
    resize(drag.current.width + drag.current.x - event.clientX)
    drag.current = null
  }
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? LIBRARY_STEP * 4 : LIBRARY_STEP
    // The handle is on the library's left edge: left makes it wider.
    if (event.key === "ArrowLeft") resize(width + step)
    else if (event.key === "ArrowRight") resize(width - step)
    else if (event.key === "Home") resize(LIBRARY_WIDTH.min)
    else if (event.key === "End") resize(LIBRARY_WIDTH.max)
    else return
    event.preventDefault()
  }

  return (
    <aside
      aria-label="Action Library"
      data-layout="sidebar"
      className="relative flex min-h-0 shrink-0 flex-col border-l bg-sidebar"
      style={{ width }}
    >
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the library"
        aria-valuemin={LIBRARY_WIDTH.min}
        aria-valuemax={LIBRARY_WIDTH.max}
        aria-valuenow={width}
        tabIndex={0}
        className={cn(
          "absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize touch-none outline-none",
          "hover:bg-ring/30 focus-visible:bg-ring/50",
        )}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onKeyDown={onKeyDown}
      />
      {header(
        <Button
          variant="ghost"
          size="icon"
          className="size-7"
          aria-label="Collapse the library"
          aria-expanded
          title="Collapse the library"
          onClick={() => {
            setCollapsed(true)
          }}
        >
          <PanelRightClose />
        </Button>,
      )}
      <div className="min-h-0 flex-1 overflow-y-auto p-3">{children}</div>
    </aside>
  )
}
