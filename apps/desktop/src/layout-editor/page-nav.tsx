import { ChevronLeft, ChevronRight, Ellipsis, Folder, Plus } from "lucide-react"
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react"

import { Button, buttonVariants } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { ancestors, page as findPage } from "@/lib/layout-model"
import { i18n, useTranslation } from "@/i18n"
import type { DeviceLayout } from "@/lib/types"
import { cn } from "@/lib/utils"

import { dockGeometry, dockOutline } from "./dock-geometry"
import { RenameField } from "./rename-field"
import { newPageName, pageNavigation, pagePath } from "./page-navigation"

const pageName = (name: string | undefined) => name || i18n.t("editor:pageNav.untitled")

interface PageNavProps {
  draft: DeviceLayout
  pageId: string
  /** Pages opened before the current one, oldest first; empty when opened directly. */
  trail: string[]
  onOpen: (pageId: string) => void
  /**
   * Shows `pageId` as reached through `trail` (a breadcrumb: the level
   * clicked, with the pages before it as the way back).
   */
  onNavigate: (pageId: string, trail: string[]) => void
  /** Adds a page in the folder of the page shown (the top level for a top-level page). */
  onAdd: (name: string) => void
  /** Renames the page shown (the breadcrumb's current segment, edited in place). */
  onRename: (name: string) => void
  /** The dock's panel under the toolbar: the Page Navigator; `close` folds it. */
  navigator: (close: () => void) => ReactNode
  /** A line under the navigation, e.g. the state of live values. */
  status?: ReactNode
}

/** The narrowest the rename field gets, in CSS pixels (4rem). */
const EDIT_MIN = 64
/** Clear space kept between the page navigation and the zoom toolbar. */
const ZOOM_GAP = 12

/**
 * The widest the current page's rename field may be: the canvas footer's
 * width, less the zoom toolbar on either side (the navigation is centred,
 * so its clearance counts twice), less the rest of the navigation bar.
 * Measured, never a fixed number.
 */
function editRoom(name: HTMLElement): number {
  const footer = name.closest<HTMLElement>("[data-slot=canvas-footer]")
  const nav = name.closest<HTMLElement>("[data-slot=page-nav]")
  if (!footer || !nav) return Math.max(EDIT_MIN, name.getBoundingClientRect().width)
  const zoom = footer.parentElement?.parentElement?.querySelector<HTMLElement>(
    "[data-slot=canvas-toolbar]",
  )
  const inset = Number.parseFloat(getComputedStyle(footer).paddingLeft) || 0
  const clearance = zoom ? zoom.getBoundingClientRect().width + ZOOM_GAP + inset : inset
  const rest = nav.getBoundingClientRect().width - name.getBoundingClientRect().width
  return Math.max(EDIT_MIN, Math.floor(footer.clientWidth - 2 * clearance - rest))
}

/** How far a breadcrumb segment may grow before its name is cut. */
const CRUMB = "max-w-40 min-w-0 shrink"
/** The current segment: the navigation controls' size, but not a control. */
const CURRENT = buttonVariants({ variant: null, size: "nav" })

/**
 * Where the editor is, from the top: the top-level page, the folders it
 * went through and the page shown. Ancestors are buttons that show that
 * level; the current page is not. When the names do not fit, the folders
 * between the first and the last collapse into a "…" menu, from the
 * outermost, as many as needed (measured, not counted); names that still
 * do not fit are cut with their full name as the tooltip.
 */
function Breadcrumbs({
  path,
  onNavigate,
  onRename,
}: {
  path: { id: string; name: string }[]
  onNavigate: (index: number) => void
  onRename: (name: string) => void
}) {
  const { t } = useTranslation()
  // The current segment edited in place: its page and the width it had.
  const [editing, setEditing] = useState<{ page: string; maxWidth: number } | null>(null)
  const currentId = path.at(-1)?.id
  const editingNow = editing !== null && editing.page === currentId ? editing : null
  const list = useRef<HTMLOListElement>(null)
  const pathKey = path.map((p) => p.id).join("/")
  const names = path.map((p) => p.name).join("/")
  const middle = Math.max(path.length - 2, 0)
  // How many of the folders between the first and the last are collapsed,
  // for the path it was measured for.
  const [fold, setFold] = useState({ key: pathKey, count: 0 })
  const collapsed = fold.key === pathKey ? Math.min(fold.count, middle) : 0

  // Collapses one more folder while a name is cut or the list overflows.
  useLayoutEffect(() => {
    const element = list.current
    if (!element || collapsed >= middle) return
    const cut = [...element.querySelectorAll<HTMLElement>("[data-crumb]")].some(
      (crumb) => crumb.scrollWidth > crumb.clientWidth + 1,
    )
    if (cut || element.scrollWidth > element.clientWidth + 1) {
      setFold({ key: pathKey, count: collapsed + 1 })
    }
  }, [fold, collapsed, middle, pathKey, names])
  // The room it has changed: measured again from nothing collapsed.
  useEffect(() => {
    const element = list.current
    const host =
      element?.closest<HTMLElement>("[data-slot=canvas-footer]") ?? element?.parentElement
    if (!element || !host || typeof ResizeObserver === "undefined") return
    let width = host.clientWidth
    const observer = new ResizeObserver(() => {
      if (host.clientWidth === width) return
      width = host.clientWidth
      setFold((current) => ({ ...current, count: 0 }))
    })
    observer.observe(host)
    return () => {
      observer.disconnect()
    }
  }, [])

  const hidden = path.slice(1, 1 + collapsed)
  const shown = path
    .map((p, index) => ({ ...p, index }))
    .filter((p) => p.index === 0 || p.index > collapsed)
  const separator = <ChevronRight className="size-3 shrink-0 text-muted-foreground" aria-hidden />
  return (
    <nav
      data-slot="page-breadcrumbs"
      aria-label={t("editor:breadcrumbs.label")}
      className="flex min-w-0 shrink items-center"
    >
      <ol
        ref={list}
        className="flex min-w-0 items-center gap-0.5 overflow-hidden whitespace-nowrap"
      >
        {shown.map((p, position) => {
          const last = p.index === path.length - 1
          return (
            <li
              key={p.id + String(p.index)}
              className="flex min-w-0 shrink items-center gap-0.5"
              data-slot="breadcrumb"
            >
              {position > 0 && separator}
              {position === 1 && hidden.length > 0 && (
                <>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-nav"
                        data-slot="breadcrumb-more"
                        className="text-muted-foreground"
                        aria-label={t("editor:breadcrumbs.hidden", { count: hidden.length })}
                        title={t("editor:breadcrumbs.hidden", { count: hidden.length })}
                      >
                        <Ellipsis aria-hidden />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="start" side="top" className="max-w-72 min-w-44">
                      <DropdownMenuLabel className="text-xs text-muted-foreground">
                        {t("editor:pageNav.location")}
                      </DropdownMenuLabel>
                      {hidden.map((h, offset) => (
                        <DropdownMenuItem
                          key={h.id}
                          onSelect={() => {
                            onNavigate(offset + 1)
                          }}
                        >
                          <Folder className="text-muted-foreground" aria-hidden />
                          <span className="truncate">{pageName(h.name)}</span>
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuContent>
                  </DropdownMenu>
                  {separator}
                </>
              )}
              {last ? (
                <span
                  className={cn("relative flex items-center", editingNow ? "shrink-0" : "min-w-0")}
                >
                  <span
                    data-crumb
                    // The text itself is cut (a block, centred by its line height).
                    className={cn(
                      CURRENT,
                      CRUMB,
                      "block cursor-text truncate leading-8",
                      editingNow && "hidden",
                    )}
                    aria-current="page"
                    data-slot="page-name"
                    title={`${pageName(p.name)} — ${t("editor:breadcrumbs.renameHint")}`}
                    onDoubleClick={(event) => {
                      event.stopPropagation()
                      setEditing({ page: p.id, maxWidth: editRoom(event.currentTarget) })
                    }}
                  >
                    {pageName(p.name)}
                  </span>
                  {editingNow && (
                    // In place of the name, as wide as the text (up to the
                    // room the bar has on the divider): the bar grows or
                    // shrinks around it and stays centred.
                    <RenameField
                      initial={p.name}
                      className="flex flex-none items-center"
                      fit={{ minWidth: EDIT_MIN, maxWidth: editingNow.maxWidth }}
                      onDone={(name) => {
                        setEditing(null)
                        if (name !== null) onRename(name)
                      }}
                    />
                  )}
                </span>
              ) : (
                <Button
                  variant="ghost"
                  size="nav"
                  data-slot="breadcrumb-link"
                  // An ancestor: lighter than the page shown (font weight only).
                  className={cn(CRUMB, "font-normal")}
                  title={pageName(p.name)}
                  onClick={() => {
                    onNavigate(p.index)
                  }}
                >
                  <span data-crumb className="min-w-0 truncate">
                    {pageName(p.name)}
                  </span>
                </Button>
              )}
            </li>
          )
        })}
      </ol>
    </nav>
  )
}

/** Kept free at either side of the central area (the inspector's deselect button). */
const DOCK_RESERVE = 48
/** Kept free under the expanded panel, to the bottom of the editor. */
const DOCK_MARGIN = 12

/** What the expanded dock measures: its toolbar and the room the panel has. */
interface DockMeasure {
  bar: { width: number; height: number; radius: number }
  /** The width the central area allows the panel (centred on the toolbar). */
  width: number
  /** The height the editor has below the toolbar. */
  height: number
}

/**
 * The room the panel has: as wide as the canvas footer allows around the
 * toolbar's centre, keeping `DOCK_RESERVE` free at either side (and the
 * zoom toolbar's width, where it reaches below the toolbar), and as tall
 * as the editor is below the toolbar. Measured, never a fixed number.
 */
function measureDock(bar: HTMLElement): DockMeasure {
  const r = bar.getBoundingClientRect()
  const footer = bar.closest<HTMLElement>("[data-slot=canvas-footer]")
  const zoom = footer?.parentElement?.parentElement?.querySelector<HTMLElement>(
    "[data-slot=canvas-toolbar]",
  )
  let width = window.innerWidth - 2 * DOCK_RESERVE
  if (footer) {
    const f = footer.getBoundingClientRect()
    const centre = r.left + r.width / 2
    let left = f.left + DOCK_RESERVE
    let right = f.right - DOCK_RESERVE
    const z = zoom?.getBoundingClientRect()
    if (z && z.width > 0 && z.bottom > r.bottom + 0.5) right = Math.min(right, z.left - ZOOM_GAP)
    left = Math.max(left, f.left)
    width = 2 * Math.min(centre - left, right - centre)
  }
  const main = bar.closest<HTMLElement>("[data-slot=device-editor-main]")
  const bottom = main ? main.getBoundingClientRect().bottom : window.innerHeight
  return {
    bar: {
      width: bar.offsetWidth,
      height: bar.offsetHeight,
      radius: Number.parseFloat(getComputedStyle(bar).borderTopLeftRadius) || 0,
    },
    width: Math.floor(width),
    height: Math.floor(bottom - r.bottom - DOCK_MARGIN),
  }
}

/** A pointer down here is not outside the dock: a row's menu, a dialog it opened. */
const OWN_LAYERS =
  "[data-radix-popper-content-wrapper],[role=menu],[role=dialog],[role=alertdialog]"

/**
 * The page navigation on the divider under the device preview, the same
 * at every level: where the editor is (breadcrumbs), previous and next
 * top-level page with the page shown, and a new page. The counter unfolds
 * the dock: the same bar, with the Page Navigator directly under it (one
 * surface, one border), overlaying the top of the inspector; the bar stays
 * where it is.
 */
export function PageNavigationDock({
  draft,
  pageId,
  trail,
  onOpen,
  onNavigate,
  onAdd,
  onRename,
  navigator,
  status,
}: PageNavProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [room, setRoom] = useState<DockMeasure | null>(null)
  const [panelHeight, setPanelHeight] = useState(0)
  const dock = useRef<HTMLDivElement>(null)
  const barRef = useRef<HTMLDivElement>(null)
  const panelRef = useRef<HTMLElement>(null)
  const panelId = useId()
  const counterId = useId()
  const inFolder = ancestors(draft, pageId).length > 0
  const { number, count, previous, next } = pageNavigation(draft, pageId)
  const current = findPage(draft, pageId)
  const name = pageName(current?.name)
  // A page inside a folder, or reached through one, shows the way there;
  // a top-level page only itself.
  const nested = trail.length > 0 || ancestors(draft, pageId).length > 0
  const path = nested ? pagePath(draft, pageId, trail) : current ? [current] : []
  const nameOf = (id: string | undefined) => pageName(id ? findPage(draft, id)?.name : undefined)

  // The new location, for screen readers (not on the first render).
  const [announcement, setAnnouncement] = useState("")
  const spoken = path.map((p) => pageName(p.name)).join(" › ")
  const shownFor = useRef(pageId)
  useEffect(() => {
    if (shownFor.current === pageId) return
    shownFor.current = pageId
    setAnnouncement(t("editor:breadcrumbs.nowShowing", { path: spoken }))
  }, [pageId, spoken, t])

  const close = useCallback(
    (focusCounter: boolean) => {
      setExpanded(false)
      if (focusCounter) {
        // Once the panel (and the focus in it) is gone.
        requestAnimationFrame(() => {
          document.getElementById(counterId)?.focus()
        })
      }
    },
    [counterId],
  )

  const fold = useCallback(() => {
    close(true)
  }, [close])

  // The toolbar and the room, while expanded: measured again when the
  // window, the toolbar (a longer name, more pages) or the tree changes.
  useLayoutEffect(() => {
    const bar = barRef.current
    if (!expanded || !bar) return
    const measure = () => {
      setRoom(measureDock(bar))
      setPanelHeight(panelRef.current?.offsetHeight ?? 0)
    }
    measure()
    window.addEventListener("resize", measure)
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure)
    observer?.observe(bar)
    if (panelRef.current) observer?.observe(panelRef.current)
    return () => {
      window.removeEventListener("resize", measure)
      observer?.disconnect()
    }
  }, [expanded])

  // A pointer down outside folds it (not in a row's menu or a dialog it opened).
  useEffect(() => {
    if (!expanded) return
    const down = (event: PointerEvent) => {
      const target = event.target
      if (!(target instanceof Element)) return
      if (dock.current?.contains(target) || target.closest(OWN_LAYERS)) return
      close(false)
    }
    document.addEventListener("pointerdown", down, true)
    return () => {
      document.removeEventListener("pointerdown", down, true)
    }
  }, [expanded, close])

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // Escape in a row's menu (a portal, bubbling here through React) is the menu's.
    if (event.key !== "Escape" || !expanded) return
    if (!(event.target instanceof Node) || !event.currentTarget.contains(event.target)) return
    event.preventDefault()
    event.stopPropagation()
    close(true)
  }

  // The panel: wider than the toolbar where there is room, centred under it.
  const geometry = room ? dockGeometry(room.bar.width, room.width) : null
  const outline =
    room && geometry && panelHeight > 0
      ? dockOutline(
          geometry,
          room.bar.width,
          room.bar.height,
          room.bar.height + panelHeight,
          room.bar.radius,
        )
      : null
  return (
    <div
      ref={dock}
      data-slot="page-navigation-dock"
      data-state={expanded ? "expanded" : "collapsed"}
      className="relative mx-auto w-max max-w-full"
      onKeyDown={onKeyDown}
      // Nothing in the dock pans or selects on the canvas.
      onPointerDown={(event) => {
        event.stopPropagation()
      }}
    >
      {expanded && geometry && outline && (
        // One outline for the toolbar and the wider panel under it (the
        // shoulders between them), drawn behind both from their sizes.
        <svg
          aria-hidden
          data-slot="page-dock-outline"
          className="pointer-events-none absolute top-0 overflow-visible"
          style={{ left: geometry.offset }}
          width={geometry.width}
          height={(room?.bar.height ?? 0) + panelHeight}
        >
          <path d={outline} className="fill-card stroke-border" strokeWidth={1} />
        </svg>
      )}
      {/* One compact bar (it straddles the canvas/inspector divider). */}
      <div
        ref={barRef}
        data-slot="page-nav"
        className={cn(
          "flex max-w-full min-w-0 items-center gap-1 rounded-lg border bg-card p-0.5",
          // Expanded, the outline draws its border and surface.
          expanded && outline && "relative border-transparent bg-transparent",
        )}
      >
        <Breadcrumbs
          path={path}
          onRename={onRename}
          onNavigate={(index) => {
            const target = path[index]
            if (!target) return
            onNavigate(
              target.id,
              path.slice(0, index).map((p) => p.id),
            )
          }}
        />
        <span className="mx-0.5 h-5 w-px shrink-0 bg-border" aria-hidden />
        <div
          role="toolbar"
          aria-label={t("editor:pageNav.label")}
          className="flex shrink-0 items-center gap-1"
        >
          <div data-slot="page-stepper" className="flex items-center gap-1 px-1">
            <Button
              variant="ghost"
              size="icon-nav"
              data-slot="page-previous"
              aria-label={t("editor:pageNav.previous")}
              title={
                previous
                  ? t("editor:pageNav.previousTo", { page: nameOf(previous) })
                  : t("editor:pageNav.first")
              }
              disabled={!previous}
              onClick={() => {
                if (previous) onOpen(previous)
              }}
            >
              <ChevronLeft />
            </Button>
            <Button
              id={counterId}
              variant="ghost"
              size="nav"
              data-slot="page-counter"
              className="font-medium tabular-nums"
              aria-label={t("editor:pageNav.positionNamed", { page: name, number, count })}
              title={t("editor:pageNav.positionTitle", { page: name, number, count })}
              aria-expanded={expanded}
              aria-controls={expanded ? panelId : undefined}
              onClick={() => {
                if (expanded) close(false)
                else setExpanded(true)
              }}
            >
              <span aria-hidden className="inline-flex items-center gap-0.5">
                <span>{number}</span>
                <span className="text-muted-foreground">/</span>
                <span className="text-muted-foreground">{count}</span>
              </span>
              <span className="sr-only">{t("editor:pageNav.position", { number, count })}</span>
            </Button>
            <Button
              variant="ghost"
              size="icon-nav"
              data-slot="page-next"
              aria-label={t("editor:pageNav.next")}
              title={
                next ? t("editor:pageNav.nextTo", { page: nameOf(next) }) : t("editor:pageNav.last")
              }
              disabled={!next}
              onClick={() => {
                if (next) onOpen(next)
              }}
            >
              <ChevronRight />
            </Button>
          </div>
          <span className="mx-0.5 h-5 w-px bg-border" aria-hidden />
          <Button
            variant="ghost"
            size="icon-nav"
            aria-label={t("editor:pageNav.add")}
            title={inFolder ? t("editor:navigator.addInFolder") : t("editor:pageNav.addTitle")}
            onClick={() => {
              onAdd(newPageName(draft))
            }}
          >
            <Plus />
          </Button>
        </div>
        {status}
      </div>
      {expanded && (
        // Directly under the toolbar, centred on it and wider where there
        // is room, over the top of the inspector (the divider's layer is
        // above it). The outline behind draws its surface and border.
        <section
          ref={panelRef}
          id={panelId}
          aria-label={t("editor:pageSettings.sheetTitle")}
          data-slot="page-navigator-panel"
          className={cn(
            "absolute top-full flex flex-col border border-t-0 p-1",
            outline ? "border-transparent" : "rounded-b-lg bg-card",
          )}
          style={{
            left: geometry?.offset ?? 0,
            width: geometry?.width ?? "100%",
            maxHeight: room ? Math.max(room.height, 64) : undefined,
          }}
        >
          {navigator(fold)}
        </section>
      )}
      <p className="sr-only" role="status" aria-live="polite" data-slot="page-announcement">
        {announcement}
      </p>
    </div>
  )
}
