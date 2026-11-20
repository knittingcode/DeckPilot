/**
 * Development only: answers the layout probe of the Vite dev server
 * (`dev/layout-probe-plugin.ts`) in the main window. Loaded by main.tsx when
 * `import.meta.env.DEV`; builds contain neither this module nor the plugin.
 *
 * Commands (fixed; arguments validated on both sides):
 * - `measure`: the scroll containers of the shell and the device editor,
 *   the document's own scrolling, and for each scroll container the element
 *   that reaches furthest below its content box (what makes it scroll);
 *   the inspector's top (its deselect button and first section); the
 *   gesture cards (their region, cards in view, rails).
 * - `gestures`: clicks the gesture cards' next or previous rail.
 * - `keys`: the control ids of the device canvas.
 * - `select`: clicks the device control with that id (as a user would).
 * - `scroll`: scrolls a known container to its top or bottom.
 * - `canvas`: a zoom of the device canvas (zoom-in, zoom-out, fit through
 *   its toolbar; actual: the `0` key), a pan (a wheel scroll) or a click on
 *   its background (pointer-focus), as a user would.
 * - `page`: the next or previous page, through the page navigation.
 * - `live`: the editor's live preview status, page, layout and selection.
 * - `dblclick`: double-clicks a device control (editor navigation only).
 * - `edit`: clear (the selected key's assignment), undo, redo, revert or
 *   save, through the editing bar.
 * - `dock`: unfolds or folds the page navigation dock (its page counter),
 *   or presses Escape in it; starts or cancels renaming the page shown
 *   (a double click on its breadcrumb, Escape), as a user would.
 */

import { i18n } from "@/i18n"

interface Box {
  top: number
  bottom: number
  height: number
  width: number
}

interface Rect {
  left: number
  top: number
  right: number
  bottom: number
}

function rect(element: Element): Rect {
  const r = element.getBoundingClientRect()
  const round = (n: number) => Math.round(n * 10) / 10
  return { left: round(r.left), top: round(r.top), right: round(r.right), bottom: round(r.bottom) }
}

function box(element: Element): Box {
  const r = element.getBoundingClientRect()
  return {
    top: Math.round(r.top),
    bottom: Math.round(r.bottom),
    height: Math.round(r.height),
    width: Math.round(r.width),
  }
}

function describe(element: Element): string {
  const slot = element.getAttribute("data-slot")
  if (slot) return `[data-slot=${slot}]`
  const label = element.getAttribute("aria-label")
  const cls = (element.getAttribute("class") ?? "").split(/\s+/).slice(0, 4).join(".")
  return `${element.tagName.toLowerCase()}${label ? `[aria-label=${label}]` : ""}${cls ? `.${cls}` : ""}`
}

/** The descendant whose bottom edge is lowest, relative to the container's scroll origin. */
function lowest(container: Element) {
  const origin = container.getBoundingClientRect().top - container.scrollTop
  let deepest: { element: string; bottom: number; position: string } | null = null
  for (const child of container.querySelectorAll("*")) {
    const style = getComputedStyle(child)
    if (style.display === "none" || style.position === "fixed") continue
    const bottom = Math.round(child.getBoundingClientRect().bottom - origin)
    if (!deepest || bottom > deepest.bottom) {
      deepest = { element: describe(child), bottom, position: style.position }
    }
  }
  return deepest
}

/**
 * Positioned descendants whose containing block lies outside the container:
 * its overflow does not clip them, so they extend the next scroll container
 * up (and do not scroll with this one).
 */
function escaping(container: Element) {
  const out: { element: string; bottom: number; containingBlock: string }[] = []
  for (const child of container.querySelectorAll<HTMLElement>("*")) {
    if (getComputedStyle(child).position !== "absolute") continue
    const block = child.offsetParent
    if (block && !container.contains(block)) {
      out.push({
        element: describe(child),
        bottom: Math.round(child.getBoundingClientRect().bottom),
        containingBlock: describe(block),
      })
    }
  }
  return { count: out.length, lowest: out.sort((a, b) => b.bottom - a.bottom).slice(0, 5) }
}

function scroller(element: Element | null) {
  if (!element) return null
  const style = getComputedStyle(element)
  return {
    element: describe(element),
    box: box(element),
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
    scrollTop: Math.round(element.scrollTop),
    overflowY: style.overflowY,
    scrolls: element.scrollHeight > element.clientHeight + 1,
    lowest: lowest(element),
    escaping: escaping(element),
  }
}

/** The device canvas: its region, viewport (never scrolling), zoom and the device's translation. */
function canvas() {
  const q = (selector: string) => document.querySelector<HTMLElement>(selector)
  const region = q('[data-slot="device-workspace"]')
  const viewport = q('[data-slot="device-viewport"]')
  if (!region || !viewport) return null
  const stage = q('[data-slot="device-stage"]')
  const grid = q('[data-slot="device-grid"]')
  const footer = q('[data-slot="canvas-footer"]')
  const toolbar = q('[data-slot="canvas-toolbar"]')
  const device = q('[data-slot="device-canvas"]')
  const divider = q('[data-slot="canvas-divider"]')
  const nav = q('[data-slot="page-nav"]')
  const inspector = q('[data-slot="inspector-scroll"]')
  const style = getComputedStyle(viewport)
  // Keys at least partly inside the viewport (the others are panned away).
  const view = viewport.getBoundingClientRect()
  const keys = [...viewport.querySelectorAll("[data-control]")]
    .map(rect)
    .filter(
      (r) =>
        r.right > view.left && r.left < view.right && r.bottom > view.top && r.top < view.bottom,
    )
  // The inspector's controls in view, at its top.
  const area = inspector?.getBoundingClientRect()
  const inspectorControls = inspector
    ? [
        ...inspector.querySelectorAll(
          "button, input, select, textarea, [role=combobox], [role=switch]",
        ),
      ]
        .map(rect)
        .filter((r) => area && r.bottom > area.top && r.top < area.bottom && r.right > r.left)
    : []
  return {
    dividerY: divider ? rect(divider).top : null,
    viewportRect: rect(viewport),
    // The zoom bar's distance from the viewport's right and bottom edges.
    toolbarInsets: toolbar
      ? (() => {
          const v = rect(viewport)
          const b = rect(toolbar)
          return {
            right: Math.round((v.right - b.right) * 10) / 10,
            bottom: Math.round((v.bottom - b.bottom) * 10) / 10,
          }
        })()
      : null,
    nav: nav ? rect(nav) : null,
    toolbarRect: toolbar ? rect(toolbar) : null,
    main: (() => {
      const m = q('[data-slot="device-editor-main"]')
      return m ? rect(m) : null
    })(),
    keys,
    inspectorControls,
    focus: {
      focused: document.activeElement === viewport,
      outlineStyle: style.outlineStyle,
      outlineWidth: style.outlineWidth,
      boxShadow: style.boxShadow,
      pointerFocus: viewport.hasAttribute("data-pointer-focus"),
    },
    region: box(region),
    regionScroll: {
      clientHeight: region.clientHeight,
      scrollHeight: region.scrollHeight,
      clientWidth: region.clientWidth,
      scrollWidth: region.scrollWidth,
    },
    viewport: box(viewport),
    clientWidth: viewport.clientWidth,
    clientHeight: viewport.clientHeight,
    scrollWidth: viewport.scrollWidth,
    scrollHeight: viewport.scrollHeight,
    scrollTop: viewport.scrollTop,
    scrollLeft: viewport.scrollLeft,
    stageScroll: stage ? { top: stage.scrollTop, left: stage.scrollLeft } : null,
    zoom: Number(viewport.dataset.zoom),
    keySize: Number(viewport.dataset.keySize),
    transform: grid?.style.transform ?? null,
    device: device ? box(device) : null,
    footer: footer ? box(footer) : null,
    toolbar: toolbar ? box(toolbar) : null,
    escaping: escaping(region),
  }
}

/**
 * The page navigation on the divider: its box and overflow, where its
 * centre is against the divider line, its stepper (previous, counter,
 * next), the zoom toolbar beside it, and — when open — the rename field of
 * the breadcrumb and the Page Navigator popover.
 */
function pageNav() {
  const nav = document.querySelector<HTMLElement>('[data-slot="page-nav"]')
  const divider = document.querySelector('[data-slot="canvas-divider"]')
  if (!nav || !divider) return null
  const r = nav.getBoundingClientRect()
  const line = divider.getBoundingClientRect()
  const footer = document.querySelector('[data-slot="canvas-footer"]')
  const zoom = document.querySelector('[data-slot="canvas-toolbar"]')
  const part = (slot: string) => {
    const element = nav.querySelector(`[data-slot="${slot}"]`)
    return element ? rect(element) : null
  }
  const input = nav.querySelector<HTMLInputElement>('[data-slot="rename-field"]')
  const popover = document.querySelector('[data-slot="page-navigator-panel"]')
  const scroll = popover?.querySelector('[data-slot="page-navigator-scroll"]')
  const dock = document.querySelector('[data-slot="page-navigation-dock"]')
  const style = (element: Element | null | undefined) => {
    if (!element) return null
    const s = getComputedStyle(element)
    return {
      borderTop: s.borderTopWidth,
      borderBottom: s.borderBottomWidth,
      radiusTop: s.borderTopLeftRadius,
      radiusBottom: s.borderBottomLeftRadius,
      boxShadow: s.boxShadow,
      background: s.backgroundColor,
      borderColor: s.borderTopColor,
    }
  }
  return {
    rect: rect(nav),
    scrollWidth: nav.scrollWidth,
    clientWidth: nav.clientWidth,
    /** Vertical centre of the bar minus the divider line's. */
    centreOffset: Math.round((r.top + r.height / 2 - line.top) * 10) / 10,
    /** Horizontal centre of the bar minus the footer's. */
    horizontalOffset: footer
      ? Math.round(
          (r.left +
            r.width / 2 -
            (footer.getBoundingClientRect().left + footer.getBoundingClientRect().width / 2)) *
            10,
        ) / 10
      : null,
    footer: footer ? rect(footer) : null,
    zoomBar: zoom ? rect(zoom) : null,
    breadcrumbs: part("page-breadcrumbs"),
    previous: part("page-previous"),
    counter: part("page-counter"),
    next: part("page-next"),
    rename: input
      ? {
          rect: rect(input),
          scrollWidth: input.scrollWidth,
          clientWidth: input.clientWidth,
          value: input.value,
        }
      : null,
    dock: dock
      ? {
          state: dock.getAttribute("data-state"),
          rect: rect(dock),
          bar: style(nav),
          panel: style(popover),
          /** Every control of the toolbar, in order. */
          controls: [...nav.querySelectorAll("button, [data-slot=page-name]")].map((e) => {
            const c = getComputedStyle(e)
            return {
              rect: rect(e),
              slot: e.getAttribute("data-slot"),
              size: e.getAttribute("data-size"),
              paddingLeft: c.paddingLeft,
              paddingRight: c.paddingRight,
            }
          }),
          /** The one outline drawn behind the toolbar and the panel (expanded). */
          outline: (() => {
            const svg = dock.querySelector('[data-slot="page-dock-outline"]')
            return svg ? rect(svg) : null
          })(),
          zoom: (() => {
            const zoom = document.querySelector('[data-slot="canvas-toolbar"]')
            return zoom ? rect(zoom) : null
          })(),
          expanded:
            document.querySelector('[data-slot="page-counter"]')?.getAttribute("aria-expanded") ??
            null,
          floating: document.querySelectorAll(
            '[data-slot="popover-content"], [data-radix-popper-content-wrapper]',
          ).length,
          library: (() => {
            const library = document.querySelector('[data-slot="library-dock"]')
            return library ? rect(library) : null
          })(),
        }
      : null,
    navigator: popover
      ? {
          rect: rect(popover),
          scroll: scroll
            ? { scrollHeight: scroll.scrollHeight, clientHeight: scroll.clientHeight }
            : null,
        }
      : null,
  }
}

/**
 * The inspector's top: its scroll region (with the right edge of its
 * content, inside a scrollbar), the deselect button (X), the first
 * section of its content, the divider and the page navigation, and the
 * form controls the X covers.
 */
function inspectorTop() {
  const scroll = document.querySelector<HTMLElement>('[data-slot="inspector-scroll"]')
  if (!scroll) return null
  const divider = document.querySelector('[data-slot="canvas-divider"]')
  const nav = document.querySelector('[data-slot="page-nav"]')
  const button = scroll.querySelector<HTMLElement>('[data-slot="inspector-deselect"] button')
  const content = scroll.querySelector(':scope > :not([data-slot="inspector-deselect"])')
  // The first section drawn (an empty notices row is not).
  const first =
    [...(content?.children ?? [])].find((e) => e.getBoundingClientRect().height > 0) ?? null
  const r = scroll.getBoundingClientRect()
  const x = button ? rect(button) : null
  const covered = x
    ? [
        ...scroll.querySelectorAll(
          "button, input, select, textarea, [role=combobox], [role=switch], [role=slider]",
        ),
      ]
        .filter((e) => e !== button)
        .map(rect)
        .filter(
          (c) =>
            c.right > c.left &&
            c.left < x.right - 0.5 &&
            x.left < c.right - 0.5 &&
            c.top < x.bottom - 0.5 &&
            x.top < c.bottom - 0.5,
        )
    : []
  return {
    scroll: rect(scroll),
    /** The right edge of the region's content area (left of a scrollbar). */
    clientRight: Math.round((r.left + scroll.clientLeft + scroll.clientWidth) * 10) / 10,
    dividerY: divider ? rect(divider).top : null,
    nav: nav ? rect(nav) : null,
    deselect: x ? { rect: x, label: button?.getAttribute("aria-label") ?? null } : null,
    firstSection: first ? { element: describe(first), rect: rect(first) } : null,
    covered,
    regionLabel:
      scroll.querySelector('[data-slot="inspector"]')?.getAttribute("aria-label") ?? null,
  }
}

/**
 * The gesture cards of the inspector (gesture-carousel.tsx): the region
 * and its clipping viewport with their scroll sizes, the cards in view and
 * the form controls in them, the arrows, and the document's width.
 */
function gestureCards() {
  const region = document.querySelector<HTMLElement>('[data-slot="gesture-cards"]')
  if (!region) return null
  const viewport = region.querySelector<HTMLElement>('[data-slot="gesture-cards-viewport"]')
  const items = [...region.querySelectorAll<HTMLElement>("[data-carousel-item]")]
  const shown = items.filter((item) => item.hasAttribute("data-visible"))
  const arrow = (slot: string) => {
    const button = region.querySelector<HTMLButtonElement>(`[data-slot="${slot}"]`)
    return button
      ? { rect: rect(button), disabled: button.disabled, label: button.getAttribute("aria-label") }
      : null
  }
  return {
    rect: rect(region),
    label: region.getAttribute("aria-label"),
    scrollWidth: region.scrollWidth,
    clientWidth: region.clientWidth,
    overflowing: region.hasAttribute("data-overflowing"),
    viewport: viewport
      ? {
          rect: rect(viewport),
          scrollWidth: viewport.scrollWidth,
          clientWidth: viewport.clientWidth,
          overflowX: getComputedStyle(viewport).overflowX,
        }
      : null,
    count: items.length,
    /** Every card's height, in view or not. */
    heights: items.map((item) => {
      const card = item.querySelector('[data-slot="gesture-card"]') ?? item
      return Math.round(card.getBoundingClientRect().height * 10) / 10
    }),
    cards: shown.map((item) => {
      const card = item.querySelector('[data-slot="gesture-card"]') ?? item
      return {
        rect: rect(card),
        borderStyle: getComputedStyle(card).borderTopStyle,
        borderWidth: getComputedStyle(card).borderTopWidth,
        empty:
          card.getAttribute("data-gesture-slot") === "empty" ||
          !!card.querySelector('[data-slot="gesture-card-empty"]'),
        /** The empty state's content centre minus its body's centre. */
        emptyOffset: (() => {
          const body = card.querySelector('[data-slot="gesture-card-empty"]')
          const content = body?.querySelector('[data-slot="gesture-card-empty-content"]')
          if (!body || !content) return null
          const b = body.getBoundingClientRect()
          const c = content.getBoundingClientRect()
          const round = (n: number) => Math.round(n * 10) / 10
          return {
            x: round(c.left + c.width / 2 - (b.left + b.width / 2)),
            y: round(c.top + c.height / 2 - (b.top + b.height / 2)),
          }
        })(),
        /** Scrolls inside itself (it must not). */
        scrolls:
          card.scrollHeight > card.clientHeight + 1 || card.scrollWidth > card.clientWidth + 1,
        label:
          document.getElementById(card.getAttribute("aria-labelledby") ?? "")?.textContent ?? null,
        controls: [
          ...card.querySelectorAll("button, input, select, textarea, [role=combobox]"),
        ].map(rect),
      }
    }),
    previous: arrow("gesture-cards-previous"),
    next: arrow("gesture-cards-next"),
    documentWidth: {
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    },
  }
}

/**
 * The selected key's header in the Inspector and its title appearance
 * popover: where they are, whether the popover is on top of everything at
 * its corners and centre (the Inspector, its curtain), whether anything in
 * it scrolls, and the heights of its controls.
 */
function titleLook() {
  const header = document.querySelector('[data-slot="inspector"] [data-slot="key-appearance"]')
  const trigger = document.querySelector(
    '[data-slot="inspector"] [data-slot="title-format-trigger"]',
  )
  const popover = document.querySelector('[data-slot="title-popover"]')
  const curtain = document.querySelector(".inspector-curtain")
  const scrolls = (element: Element) => {
    const s = getComputedStyle(element)
    return (
      ["auto", "scroll"].includes(s.overflowY) && element.scrollHeight > element.clientHeight + 1
    )
  }
  let onTop: boolean | null = null
  let scrollers: string[] = []
  let controls: number[] = []
  if (popover) {
    const r = popover.getBoundingClientRect()
    const inset = 6
    const points = [
      [r.left + inset, r.top + inset],
      [r.right - inset, r.top + inset],
      [r.left + inset, r.bottom - inset],
      [r.right - inset, r.bottom - inset],
      [(r.left + r.right) / 2, (r.top + r.bottom) / 2],
    ] as const
    onTop = points.every(([x, y]) => {
      const hit = document.elementFromPoint(x, y)
      return hit !== null && popover.contains(hit)
    })
    scrollers = [popover, ...popover.querySelectorAll("*")].filter(scrolls).map(describe)
    controls = [
      ...popover.querySelectorAll(
        '[data-slot="title-row"] button, [data-slot="title-row"] input, [data-slot="title-font"]',
      ),
    ]
      .filter(
        (e) =>
          !(e as HTMLElement).closest('[role="checkbox"]') && e.getAttribute("role") !== "checkbox",
      )
      .map((e) => Math.round(e.getBoundingClientRect().height))
  }
  const wrapper = popover?.parentElement
  return {
    window: { width: window.innerWidth, height: window.innerHeight },
    header: header
      ? {
          rect: rect(header),
          overflowsX: header.scrollWidth > header.clientWidth + 1,
          preview: (() => {
            const p = header.querySelector('[data-slot="key-preview"]')
            return p ? rect(p) : null
          })(),
        }
      : null,
    trigger: trigger ? rect(trigger) : null,
    popover: popover ? rect(popover) : null,
    popoverZ: wrapper ? Number(getComputedStyle(wrapper).zIndex) || 0 : null,
    curtainZ: curtain ? Number(getComputedStyle(curtain).zIndex) || 0 : null,
    inPortal: popover ? !popover.closest('[data-slot="inspector-scroll"]') : null,
    onTop,
    scrollers,
    controls,
  }
}

/**
 * The selected key's menu and its picker popover (icon-menu.tsx): where
 * they are, whether the picker is on top of everything at its corners and
 * centre, what scrolls in it besides the grids' own area, and the header's
 * rows (no Background row, Show title under the title).
 */
function keyMenuLook() {
  const header = document.querySelector('[data-slot="inspector"] [data-slot="key-appearance"]')
  const menu = document.querySelector('[data-slot="key-menu"]')
  const picker = document.querySelector('[data-slot="key-picker-popover"]')
  const curtain = document.querySelector(".inspector-curtain")
  const scrolls = (element: Element) => {
    const s = getComputedStyle(element)
    return (
      ["auto", "scroll"].includes(s.overflowY) && element.scrollHeight > element.clientHeight + 1
    )
  }
  let onTop: boolean | null = null
  let scrollers: string[] = []
  if (picker) {
    const r = picker.getBoundingClientRect()
    const inset = 6
    const points = [
      [r.left + inset, r.top + inset],
      [r.right - inset, r.top + inset],
      [r.left + inset, r.bottom - inset],
      [r.right - inset, r.bottom - inset],
      [(r.left + r.right) / 2, (r.top + r.bottom) / 2],
    ] as const
    onTop = points.every(([x, y]) => {
      const hit = document.elementFromPoint(x, y)
      return hit !== null && picker.contains(hit)
    })
    // Only a library's own grid area may scroll.
    scrollers = [picker, ...picker.querySelectorAll("*")]
      .filter(scrolls)
      .filter((e) => e.getAttribute("data-slot") !== "asset-scroll")
      .map(describe)
  }
  const title = header?.querySelector('[data-slot="key-title"]')
  const show = header?.querySelector('[data-slot="show-title"]')
  const wrapper = picker?.parentElement
  return {
    window: { width: window.innerWidth, height: window.innerHeight },
    header: header ? rect(header) : null,
    backgroundRow: header?.querySelector('[data-slot="key-background"]') !== null && !!header,
    title: title ? rect(title) : null,
    showTitle: show ? rect(show) : null,
    menu: menu ? rect(menu) : null,
    menuItems: menu ? menu.querySelectorAll('[role="menuitem"]').length : 0,
    picker: picker ? rect(picker) : null,
    pickerKind: picker?.getAttribute("data-picker") ?? null,
    tiles: picker ? picker.querySelectorAll('[data-slot="asset-tile"]').length : 0,
    pickerZ: wrapper ? Number(getComputedStyle(wrapper).zIndex) || 0 : null,
    curtainZ: curtain ? Number(getComputedStyle(curtain).zIndex) || 0 : null,
    inPortal: picker ? !picker.closest('[data-slot="inspector-scroll"]') : null,
    onTop,
    scrollers,
    focusOnPreview: document.activeElement?.getAttribute("data-slot") === "key-preview",
  }
}

function measure() {
  const q = (selector: string) => document.querySelector(selector)
  const main = q('[data-slot="device-editor-main"]')
  // Every ancestor of the editor's main area up to <html>: where height comes from.
  const chain: { element: string; box: Box; minHeight: string; flex: string; overflowY: string }[] =
    []
  for (let e: Element | null = main; e; e = e.parentElement) {
    const s = getComputedStyle(e)
    chain.push({
      element: describe(e),
      box: box(e),
      minHeight: s.minHeight,
      flex: `${s.display} ${s.flexDirection} grow=${s.flexGrow} shrink=${s.flexShrink} basis=${s.flexBasis}`,
      overflowY: s.overflowY,
    })
  }
  // The toolbar: where the native window controls end and where its first
  // visible content (selectors, heading, buttons) starts.
  const bar = q('[data-slot="titlebar"]') as HTMLElement | null
  const controlsEdge = bar
    ? Number.parseFloat(bar.style.getPropertyValue("--titlebar-controls-inset")) || 0
    : 0
  const contentStart = bar
    ? Math.min(
        ...[...bar.querySelectorAll("button, a, input, h1, h2, [role=button], [role=heading]")]
          .map((e) => e.getBoundingClientRect())
          .filter((r) => r.width > 0 && r.height > 0)
          .map((r) => Math.round(r.left)),
      )
    : null
  // Every drawn key: rendered size, border width and colour, selection.
  const controls = [
    ...document.querySelectorAll<HTMLElement>('[data-slot="device-keys"] [data-control]'),
  ].map((e) => {
    const r = e.getBoundingClientRect()
    const style = getComputedStyle(e)
    return {
      control: e.dataset.control ?? "",
      width: Math.round(r.width * 100) / 100,
      height: Math.round(r.height * 100) / 100,
      border: style.borderTopWidth,
      borderColor: style.borderTopColor,
      radius: style.borderTopLeftRadius,
      selected: e.getAttribute("aria-pressed") === "true",
    }
  })
  // The editing bar and its buttons.
  const barElement = q('[data-slot="editing-bar"]')
  const editingBar = barElement
    ? {
        box: box(barElement),
        buttons: [...barElement.querySelectorAll("button")].map((b) => {
          const r = b.getBoundingClientRect()
          const icon = b.querySelector("svg")?.getBoundingClientRect()
          return {
            label: b.getAttribute("aria-label") ?? b.textContent.trim(),
            top: Math.round(r.top * 10) / 10,
            bottom: Math.round(r.bottom * 10) / 10,
            height: Math.round(r.height * 10) / 10,
            width: Math.round(r.width * 10) / 10,
            iconOffset: icon
              ? Math.round((icon.top + icon.height / 2 - (r.top + r.height / 2)) * 10) / 10
              : null,
            clipped: b.scrollWidth > b.clientWidth + 1,
          }
        }),
      }
    : null
  // The inspector curtain: where it is, whether it shows, and what a pointer
  // reaches through it (it must take no pointer events).
  const curtainElement = q('[data-slot="inspector-curtain"]')
  const scrollElement = q('[data-slot="inspector-scroll"]')
  const hit = (x: number, y: number) => {
    const e = document.elementFromPoint(x, y)
    return e ? describe(e.closest("[data-slot]") ?? e) : null
  }
  const curtain = curtainElement
    ? (() => {
        const r = curtainElement.getBoundingClientRect()
        const dock = q('[data-slot="canvas-nav"]')?.getBoundingClientRect()
        const deselect = q('[data-slot="inspector-deselect"] button')?.getBoundingClientRect()
        return {
          rect: box(curtainElement),
          opacity: Number(getComputedStyle(curtainElement).opacity),
          pointerEvents: getComputedStyle(curtainElement).pointerEvents,
          scrolled: curtainElement.parentElement?.hasAttribute("data-scrolled") ?? false,
          scrollTop: scrollElement ? Math.round(scrollElement.scrollTop) : null,
          backdrop:
            getComputedStyle(curtainElement.children[0] as Element).backdropFilter ||
            getComputedStyle(curtainElement.children[0] as Element).getPropertyValue(
              "-webkit-backdrop-filter",
            ),
          hitInside: hit(r.left + r.width / 2, r.top + r.height / 2),
          hitDock: dock ? hit(dock.left + dock.width / 2, dock.bottom - 3) : null,
          hitDeselect: deselect
            ? hit(deselect.left + deselect.width / 2, deselect.top + deselect.height / 2)
            : null,
        }
      })()
    : null
  return {
    viewport: { width: window.innerWidth, height: window.innerHeight },
    devicePixelRatio: window.devicePixelRatio,
    curtain,
    editingBar,
    controls,
    titlebar: bar
      ? {
          context: bar.dataset.context ?? null,
          controlsEdge,
          contentStart,
          gap: contentStart === null ? null : contentStart - controlsEdge,
        }
      : null,
    document: {
      scrollHeight: document.documentElement.scrollHeight,
      clientHeight: document.documentElement.clientHeight,
      scrolls: document.documentElement.scrollHeight > document.documentElement.clientHeight + 1,
      bodyScrollHeight: document.body.scrollHeight,
    },
    statusbar: q('[data-slot="statusbar"]') ? box(q('[data-slot="statusbar"]') as Element) : null,
    workspace: scroller(q('[data-slot="sidebar-inset"]:not([data-view="settings"])')),
    editor: q('[data-slot="device-editor"]')
      ? box(q('[data-slot="device-editor"]') as Element)
      : null,
    editorMain: scroller(main),
    inspectorScroll: scroller(q('[data-slot="inspector-scroll"]')),
    canvas: canvas(),
    library: scroller(q('[data-slot="library-scroll"]')),
    inspector: q('[data-slot="inspector"]') ? box(q('[data-slot="inspector"]') as Element) : null,
    inspectorTop: inspectorTop(),
    gestureCards: gestureCards(),
    selected:
      q(
        '[data-slot="device-keys"] [aria-pressed="true"], [data-slot="device-keys"] [aria-selected="true"]',
      )?.getAttribute("data-control") ?? null,
    chain,
    pageNav: pageNav(),
    titleLook: titleLook(),
    keyMenu: keyMenuLook(),
  }
}

type Command = { id: string; command: string; args: Record<string, unknown> }

function run({ command, args }: Command): unknown {
  switch (command) {
    case "measure":
      return measure()
    case "keys":
      return [...document.querySelectorAll("[data-control]")].map((e) =>
        e.getAttribute("data-control"),
      )
    case "select": {
      const id = String(args.control)
      if (!/^[\w:.-]{1,64}$/.test(id)) return { error: "invalid control id" }
      const target = document.querySelector<HTMLElement>(`[data-control="${CSS.escape(id)}"]`)
      if (!target) return { error: `no control ${id}` }
      target.click()
      return { ok: true }
    }
    case "scroll": {
      const slot = String(args.slot)
      const selector =
        slot === "sidebar-inset"
          ? '[data-slot="sidebar-inset"]:not([data-view="settings"])'
          : slot === "device-editor-main" ||
              slot === "inspector-scroll" ||
              slot === "library-scroll"
            ? `[data-slot="${slot}"]`
            : null
      const element = selector ? document.querySelector(selector) : null
      if (!element) return { error: `no ${slot}` }
      element.scrollTop = args.to === "bottom" ? element.scrollHeight : 0
      return { ok: true, scrollTop: element.scrollTop }
    }
    case "canvas": {
      const action = String(args.action)
      const viewport = document.querySelector<HTMLElement>('[data-slot="device-viewport"]')
      if (!viewport) return { error: "no device canvas" }
      if (action === "pan") {
        viewport.dispatchEvent(
          new WheelEvent("wheel", { deltaX: 40, deltaY: 30, bubbles: true, cancelable: true }),
        )
        return { ok: true }
      }
      if (action === "pointer-focus") {
        // A click on the background, as the pointer gives it.
        const stage = document.querySelector('[data-slot="device-stage"]')
        if (!stage) return { error: "no canvas stage" }
        const r = stage.getBoundingClientRect()
        const at = {
          clientX: r.left + 4,
          clientY: r.top + 4,
          bubbles: true,
          button: 0,
          pointerId: 99,
        }
        stage.dispatchEvent(new PointerEvent("pointerdown", at))
        stage.dispatchEvent(new PointerEvent("pointerup", at))
        return { ok: true }
      }
      if (action === "actual") {
        viewport.dispatchEvent(new KeyboardEvent("keydown", { key: "0", bubbles: true }))
        return { ok: true }
      }
      if (action === "state" || action === "drag") {
        const gridElement = document.querySelector('[data-slot="device-grid"]')
        const rects = () => ({
          viewport: box(viewport),
          grid: gridElement ? box(gridElement) : null,
          transform: gridElement instanceof HTMLElement ? gridElement.style.transform : null,
          zoom: viewport.dataset.zoom ?? null,
          panning: viewport.hasAttribute("data-panning"),
        })
        if (action === "state") return rects()
        // A background drag as the pointer does it: down on the empty stage
        // (or anywhere with Space), moves in steps, up.
        const before = rects()
        const stage = document.querySelector('[data-slot="device-stage"]')
        if (!stage) return { error: "no canvas stage" }
        const r = stage.getBoundingClientRect()
        const button = Number(args.button ?? 0)
        const start = { x: r.left + 6, y: r.bottom - 6 }
        const init = (x: number, y: number) => ({
          clientX: x,
          clientY: y,
          bubbles: true,
          cancelable: true,
          button,
          buttons: button === 1 ? 4 : 1,
          pointerId: 98,
          pointerType: "mouse",
        })
        stage.dispatchEvent(new PointerEvent("pointerdown", init(start.x, start.y)))
        const dx = Number(args.dx)
        const dy = Number(args.dy)
        for (let i = 1; i <= 5; i++) {
          viewport.dispatchEvent(
            new PointerEvent("pointermove", init(start.x + (dx * i) / 5, start.y + (dy * i) / 5)),
          )
        }
        viewport.dispatchEvent(new PointerEvent("pointerup", init(start.x + dx, start.y + dy)))
        return new Promise((resolve) => {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              resolve({ before, after: rects() })
            })
          })
        })
      }
      if (!["zoom-in", "zoom-out", "fit"].includes(action)) return { error: "unknown action" }
      const button = document.querySelector<HTMLButtonElement>(
        `[data-slot="canvas-toolbar"] [data-canvas-action="${action}"]`,
      )
      if (!button) return { error: `no ${action} button` }
      if (button.disabled) return { ok: true, disabled: true }
      button.click()
      return { ok: true }
    }
    case "gestures": {
      // The gesture cards' previous/next rail, as a user clicks it.
      if (args.to !== "next" && args.to !== "previous") return { error: "gestures: next|previous" }
      const button = document.querySelector<HTMLButtonElement>(
        `[data-slot="gesture-cards-${args.to}"]`,
      )
      if (!button) return { ok: true, absent: true }
      if (button.disabled) return { ok: true, disabled: true }
      button.click()
      // Measured once the slide has ended.
      return new Promise((resolve) =>
        setTimeout(() => {
          resolve({ ok: true })
        }, 350),
      )
    }
    case "title": {
      // The selected key's T▾ button, as a user clicks it; Escape as a
      // user presses it in the popover.
      const trigger = document.querySelector<HTMLButtonElement>(
        '[data-slot="inspector"] [data-slot="title-format-trigger"]',
      )
      if (!trigger) return { ok: true, absent: true }
      const open = trigger.getAttribute("aria-expanded") === "true"
      if (args.to === "escape") {
        const target = document.querySelector<HTMLElement>('[data-slot="title-popover"]') ?? trigger
        target.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))
      } else if ((args.to === "open") !== open) {
        trigger.click()
      }
      // Measured once the popover has opened (or closed) and settled.
      return new Promise((resolve) =>
        setTimeout(() => {
          resolve({ ok: true })
        }, 300),
      )
    }
    case "keymenu": {
      // The selected key's preview menu, as a user opens it with the
      // keyboard and chooses an item; Escape in the picker; the header's
      // Show title switch. Opening a picker changes nothing; toggling Show
      // title twice leaves the draft as it was.
      const to = String(args.to)
      const preview = document.querySelector<HTMLElement>(
        '[data-slot="inspector"] [data-slot="key-preview"]',
      )
      if (!preview) return { ok: true, absent: true }
      const settle = (ms = 300) =>
        new Promise((resolve) =>
          setTimeout(() => {
            resolve({ ok: true })
          }, ms),
        )
      if (to === "escape") {
        const target =
          document.querySelector<HTMLElement>('[data-slot="key-picker-popover"]') ??
          document.querySelector<HTMLElement>('[data-slot="key-menu"]') ??
          preview
        target.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))
        return settle()
      }
      if (to === "show-title") {
        const toggle = document.querySelector<HTMLElement>(
          '[data-slot="inspector"] [data-slot="show-title"] [role="switch"]',
        )
        if (!toggle) return { error: "no Show title switch" }
        toggle.click()
        return settle(200)
      }
      const items: Record<string, string> = {
        menu: "",
        icons: i18n.t("editor:iconMenu.library"),
        backgrounds: i18n.t("editor:keyMenu.chooseBackground"),
        colour: i18n.t("editor:keyMenu.customColour"),
      }
      const label = items[to]
      if (label === undefined)
        return { error: "keymenu: menu|icons|backgrounds|colour|escape|show-title" }
      preview.focus()
      preview.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }))
      return settle(250).then(() => {
        if (!label) return { ok: true }
        const item = [
          ...document.querySelectorAll<HTMLElement>('[data-slot="key-menu"] [role="menuitem"]'),
        ].find((e) => e.textContent.trim() === label)
        if (!item) return { error: `no menu item ${to}` }
        item.click()
        return settle(400)
      })
    }
    case "live": {
      // The editor's live preview and where it is
      // (layout-editor/use-live-preview.ts, docs/editor-navigation-and-simulation.md).
      const status = document.querySelector('[data-slot="live-preview-status"]')
      const crumbs = [
        ...document.querySelectorAll('[data-slot="page-nav"] [data-slot="page-name"]'),
      ].map((e) => e.textContent.trim())
      return {
        live: status?.getAttribute("data-status") ?? null,
        text: status?.textContent.trim() ?? null,
        unsaved: document.querySelector('[data-slot="editing-state"]')?.textContent.trim() ?? null,
        page: crumbs.at(-1) ?? null,
        layout:
          document
            .querySelector('[data-slot="titlebar"] button[aria-label^="Layout"]')
            ?.getAttribute("aria-label") ?? null,
        selected:
          document
            .querySelector('[data-slot="device-keys"] [aria-pressed="true"]')
            ?.getAttribute("data-control") ?? null,
      }
    }
    case "dblclick": {
      // Editor navigation (a folder, Back, a page or layout key), as a user
      // double-clicks: never runs anything.
      const id = String(args.control)
      if (!/^[\w:.-]{1,64}$/.test(id)) return { error: "invalid control id" }
      const target = document.querySelector<HTMLElement>(`[data-control="${CSS.escape(id)}"]`)
      if (!target) return { error: `no control ${id}` }
      target.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }))
      return { ok: true }
    }
    case "edit": {
      // The editing bar's buttons; `clear` removes the selected key's
      // assignment as the Delete key does (a visible, undoable edit).
      const to = String(args.to)
      if (to === "clear") {
        const selected = document.querySelector<HTMLElement>(
          '[data-slot="device-keys"] [aria-pressed="true"]',
        )
        if (!selected) return { error: "no key selected" }
        selected.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true }))
        return { ok: true }
      }
      const labels: Record<string, string> = {
        undo: i18n.t("editor:bar.undo"),
        redo: i18n.t("editor:bar.redo"),
        revert: i18n.t("editor:bar.revert"),
        save: i18n.t("editor:bar.save"),
      }
      const label = labels[to]
      if (!label) return { error: "edit: clear|undo|redo|revert|save" }
      const button = [
        ...document.querySelectorAll<HTMLButtonElement>('[data-slot="editing-bar"] button'),
      ].find((b) => b.getAttribute("aria-label") === label || b.textContent.trim() === label)
      if (!button) return { error: `no ${to} button` }
      if (button.disabled) return { ok: true, disabled: true }
      button.click()
      return { ok: true }
    }
    case "page": {
      const label = i18n.t(
        args.to === "previous" ? "editor:pageNav.previous" : "editor:pageNav.next",
      )
      const button = document.querySelector<HTMLButtonElement>(
        `[data-slot="page-nav"] button[aria-label=${JSON.stringify(label)}]`,
      )
      if (!button) return { error: "no page navigation" }
      if (button.disabled) return { ok: true, disabled: true }
      button.click()
      return { ok: true }
    }
    case "dock": {
      const counter = document.querySelector<HTMLButtonElement>(
        '[data-slot="page-nav"] [data-slot="page-counter"]',
      )
      if (!counter) return { error: "no page navigation" }
      if (!["open", "close", "escape", "rename", "rename-cancel"].includes(String(args.to)))
        return { error: "dock: open|close|escape|rename|rename-cancel" }
      if (args.to === "rename") {
        // A double click on the page shown: its name edited in place.
        const name = document.querySelector('[data-slot="page-nav"] [data-slot="page-name"]')
        if (!name) return { error: "no page name" }
        name.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }))
        return { ok: true }
      }
      if (args.to === "rename-cancel") {
        const input = document.querySelector('[data-slot="page-nav"] input')
        if (!input) return { error: "not renaming" }
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))
        return { ok: true }
      }
      const open = counter.getAttribute("aria-expanded") === "true"
      if (args.to === "escape") {
        const target =
          document.querySelector<HTMLElement>(
            '[data-slot="page-navigator-panel"] [tabindex="0"]',
          ) ?? counter
        target.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))
        return { ok: true }
      }
      if ((args.to === "open") !== open) counter.click()
      return { ok: true }
    }
    case "navigate": {
      // Through the same buttons a user clicks; nothing else is reachable.
      // The labels are those of the window's language (src/i18n).
      const toolbar = document.querySelector('[data-slot="titlebar"]')
      const view = String(args.view)
      const label = (text: string) => JSON.stringify(text)
      const target =
        view === "designer"
          ? [
              ...document.querySelectorAll<HTMLElement>(
                `[aria-label=${label(i18n.t("navigation:rail.label"))}] button`,
              ),
            ].find((b) => b.textContent.trim() === i18n.t("navigation:pages.designer"))
          : view === "marketplace"
            ? toolbar?.querySelector<HTMLElement>(
                `button[aria-label=${label(i18n.t("navigation:toolbar.marketplace"))}]`,
              )
            : view === "settings"
              ? toolbar?.querySelector<HTMLElement>(
                  `[role="group"][aria-label=${label(i18n.t("navigation:toolbar.app"))}] button:last-child`,
                )
              : toolbar?.querySelector<HTMLElement>(
                  `nav[aria-label=${label(i18n.t("navigation:plugins.label"))}] button`,
                )
      if (!target) return { error: `no way to open ${view} from here` }
      target.click()
      return { ok: true }
    }
    default:
      return { error: "unknown command" }
  }
}

export function startLayoutProbe() {
  const hot = import.meta.hot
  if (!hot) return
  hot.on("deckpilot:probe", (message: Command) => {
    // Let the click's re-render and layout settle before answering.
    setTimeout(
      () => {
        const reply = (result: unknown) => {
          hot.send("deckpilot:probe-result", { id: message.id, result })
        }
        const failed = (e: unknown) => {
          reply({ error: e instanceof Error ? e.message : String(e) })
        }
        try {
          // Some commands (a drag) answer once the frame has been drawn.
          Promise.resolve(run(message)).then(reply, failed)
        } catch (e) {
          failed(e)
        }
      },
      // Key images are drawn again once a zoom rests (device-viewport.tsx).
      message.command === "select" ||
        message.command === "navigate" ||
        message.command === "canvas" ||
        message.command === "page" ||
        message.command === "dock" ||
        message.command === "dblclick" ||
        message.command === "edit"
        ? 500
        : 0,
    )
  })
}
