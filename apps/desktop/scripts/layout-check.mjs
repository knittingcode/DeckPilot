#!/usr/bin/env node
/**
 * Layout regression check of the Device Editor, against the RUNNING app of a
 * development session (`tauri dev`): real WebKit/WebView2/WebKitGTK layout,
 * not jsdom. Uses the layout probe of the Vite dev server
 * (dev/layout-probe-plugin.ts); see docs/development.md, "Layout probe".
 *
 *   node apps/desktop/scripts/layout-check.mjs            # port 1420, .dev
 *   DECKPILOT_VITE_PORT=1441 DECKPILOT_DEV_DIR=… node …    # another session
 *
 * The main window must show the Device Editor. The check selects every key
 * twice, pages, zooms the device canvas, scrolls the scroll containers and
 * asserts after each step:
 * - the document never scrolls and the status bar stays at the bottom;
 * - the workspace does not scroll while the editor fills it, nor does the
 *   editor's main area: only its inspector region and the library scroll;
 * - the device canvas never scrolls (scroll size = client size, at any
 *   zoom), and its height is the same for every key, page and zoom;
 * - no positioned element escapes a scroll container or the canvas (the
 *   cause of the blank scroll area fixed in November 2026);
 * - the inspector's scrollable height ends at its content (no blank area);
 * - selecting the same key again gives the same heights (nothing
 *   accumulates);
 * - scrolling the library does not move the inspector, and the reverse;
 * - with a key selected, the inspector's deselect button sits at its
 *   top-right below the divider, clear of the page navigation and the
 *   form, also when the inspector is scrolled, and the first section
 *   starts right below the page navigation;
 * - with a key selected, its key header fits the inspector, and its title
 *   appearance popover opens inside the window, above the inspector and its
 *   curtain, without scrolling, without changing the inspector;
 * - with a key selected, its header has no Background row and Show title
 *   under the title (toggling it keeps the header's height), and its key
 *   menu and every picker open inside the window, above the inspector and
 *   its curtain, scrolling only in a library's grid; Escape returns focus to
 *   the preview;
 * - with a key selected, the gesture cards never scroll horizontally, show
 *   only whole cards of equal size and rails only when they overflow;
 *   moving them keeps their height and the inspector's scroll position.
 * Exit code 1 with the failures listed, 0 when all pass.
 */
import { readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const port = Number(process.env.DECKPILOT_VITE_PORT ?? 1420)
const devDir = process.env.DECKPILOT_DEV_DIR ?? path.resolve(here, "../../../.dev")
const token = readFileSync(path.join(devDir, "layout-probe.token"), "utf8").trim()

async function probe(command, args = {}) {
  for (let attempt = 0; ; attempt++) {
    const response = await fetch(`http://localhost:${port}/__deckpilot/probe`, {
      method: "POST",
      headers: { "x-deckpilot-probe": token, "content-type": "application/json" },
      body: JSON.stringify({ command, args }),
    })
    if (response.status === 504 && attempt < 3) {
      await new Promise((r) => setTimeout(r, 1000))
      continue
    }
    const body = await response.json()
    if (!response.ok || body?.error)
      throw new Error(`${command}: ${body?.error ?? response.status}`)
    return body
  }
}

const failures = []
let checks = 0
function expect(condition, message) {
  checks++
  if (!condition) failures.push(message)
}

/** Allowed difference between the content's end and the scrollable end (the inspector's padding). */
const INSPECTOR_PADDING = 24
const TOLERANCE = 2

/** Chroma of a computed `oklch(…)`/`oklab(…)` colour (0 for greys). */
function chroma(colour) {
  const n = colour.match(/-?[\d.]+/g)?.map(Number) ?? []
  if (colour.startsWith("oklch(")) return n[1] ?? 0
  if (colour.startsWith("oklab(")) return Math.hypot(n[1] ?? 0, n[2] ?? 0)
  return 0
}

/** Clear space required between the macOS traffic lights and toolbar content. */
const CONTROLS_GAP = 16

/**
 * The page navigation on the divider (page-nav.tsx): no horizontal
 * overflow, centred on the divider line (±1px) and in the canvas footer,
 * clear of the zoom toolbar, its stepper one tight group of 32px controls
 * 4px apart and centred on one line; an open rename field shows its whole
 * value while within its maximum, and an open Page Navigator stays inside
 * the window. Older apps without the measurement are skipped.
 */
function pageNavInvariants(m, step) {
  const nav = m.pageNav
  if (!nav) return
  expect(
    nav.scrollWidth <= nav.clientWidth + 1,
    `${step}: the page navigation overflows (${nav.scrollWidth} > ${nav.clientWidth})`,
  )
  expect(
    Math.abs(nav.centreOffset) <= 1,
    `${step}: the page navigation is ${nav.centreOffset}px off the divider`,
  )
  if (
    nav.horizontalOffset !== null &&
    nav.footer &&
    nav.rect.right - nav.rect.left < nav.footer.right - nav.footer.left - 4
  ) {
    expect(
      Math.abs(nav.horizontalOffset) <= 1,
      `${step}: the page navigation is ${nav.horizontalOffset}px off centre`,
    )
  }
  if (nav.zoomBar) {
    const z = nav.zoomBar
    const overlaps =
      nav.rect.left < z.right &&
      z.left < nav.rect.right &&
      nav.rect.top < z.bottom &&
      z.top < nav.rect.bottom
    expect(!overlaps, `${step}: the page navigation overlaps the zoom toolbar`)
  }
  const { previous, counter, next } = nav
  if (previous && counter && next) {
    for (const [name, r] of [
      ["previous", previous],
      ["next", next],
    ]) {
      expect(
        Math.abs(r.right - r.left - 32) <= 0.5 && Math.abs(r.bottom - r.top - 32) <= 0.5,
        `${step}: the ${name} page button is ${r.right - r.left}×${r.bottom - r.top}, not 32×32`,
      )
    }
    expect(
      Math.abs(counter.bottom - counter.top - 32) <= 0.5,
      `${step}: the page counter is not 32px high`,
    )
    const gapBefore = counter.left - previous.right
    const gapAfter = next.left - counter.right
    expect(
      Math.abs(gapBefore - 4) <= 1 && Math.abs(gapAfter - 4) <= 1,
      `${step}: the page stepper's gaps are ${gapBefore}/${gapAfter}px, not 4px`,
    )
    const centres = [previous, counter, next].map((r) => (r.top + r.bottom) / 2)
    expect(
      Math.max(...centres) - Math.min(...centres) <= 0.5,
      `${step}: the page stepper's controls are not centred on one line (${centres.join(", ")})`,
    )
  }
  if (nav.rename) {
    expect(
      nav.rename.scrollWidth <= nav.rename.clientWidth + 1,
      `${step}: the rename field hides part of its value`,
    )
    const r = nav.rename.rect
    expect(
      Math.abs(r.bottom - r.top - 32) <= 0.5,
      `${step}: the rename field is ${r.bottom - r.top}px high, not 32`,
    )
  }
  // One control size: every control 32px high, on one line; text controls
  // (breadcrumb segments, the counter) with the same padding.
  const controls = nav.dock?.controls ?? []
  for (const c of controls) {
    const height = c.rect.bottom - c.rect.top
    if (height === 0) continue // a hidden segment (the one being renamed)
    expect(
      Math.abs(height - 32) <= 0.5,
      `${step}: the ${c.slot ?? "control"} of the page navigation is ${height}px high, not 32`,
    )
  }
  const drawn = controls.filter((c) => c.rect.bottom > c.rect.top)
  const centres = drawn.map((c) => (c.rect.top + c.rect.bottom) / 2)
  expect(
    centres.length === 0 || Math.max(...centres) - Math.min(...centres) <= 0.5,
    `${step}: the page navigation's controls are not on one line (${centres.join(", ")})`,
  )
  const text = drawn.filter((c) => c.size === "nav" || c.slot === "page-name")
  const paddings = new Set(text.flatMap((c) => [c.paddingLeft, c.paddingRight]))
  expect(
    paddings.size <= 1,
    `${step}: the text controls of the page navigation have different padding (${[...paddings].join(", ")})`,
  )
  expect(
    Math.abs(nav.rect.bottom - nav.rect.top - 38) <= 0.5,
    `${step}: the page navigation is ${nav.rect.bottom - nav.rect.top}px high, not 38`,
  )
  if (nav.navigator) {
    const r = nav.navigator.rect
    expect(
      r.left >= 0 && r.top >= 0 && r.right <= m.viewport.width && r.bottom <= m.viewport.height,
      `${step}: the Page Navigator leaves the window`,
    )
  }
}

function invariants(m, step) {
  pageNavInvariants(m, step)
  if (m.titlebar && m.titlebar.controlsEdge > 0) {
    expect(
      m.titlebar.gap >= CONTROLS_GAP,
      `${step}: toolbar content starts ${m.titlebar.gap}px after the window controls (needs ${CONTROLS_GAP})`,
    )
  }
  expect(
    !m.document.scrolls,
    `${step}: the document scrolls (${m.document.scrollHeight} > ${m.document.clientHeight})`,
  )
  if (m.statusbar) {
    expect(
      Math.abs(m.statusbar.bottom - m.viewport.height) <= 1,
      `${step}: the status bar is not at the bottom (${m.statusbar.bottom} vs ${m.viewport.height})`,
    )
  }
  expect(
    m.workspace && !m.workspace.scrolls,
    `${step}: the workspace scrolls (${m.workspace?.scrollHeight} > ${m.workspace?.clientHeight})`,
  )
  expect(
    m.editor && m.workspace && Math.abs(m.editor.height - m.workspace.clientHeight) <= 1,
    `${step}: the editor does not fill the workspace (${m.editor?.height} vs ${m.workspace?.clientHeight})`,
  )
  expect(
    m.editorMain && !m.editorMain.scrolls && m.editorMain.scrollTop === 0,
    `${step}: the editor's main area scrolls (${m.editorMain?.scrollHeight} > ${m.editorMain?.clientHeight})`,
  )
  expect(m.inspectorScroll, `${step}: no inspector region`)
  const c = m.canvas
  expect(c, `${step}: no device canvas`)
  if (c) {
    expect(
      c.scrollHeight <= c.clientHeight && c.scrollWidth <= c.clientWidth,
      `${step}: the canvas viewport overflows (${c.scrollWidth}×${c.scrollHeight} > ${c.clientWidth}×${c.clientHeight})`,
    )
    expect(
      c.scrollTop === 0 &&
        c.scrollLeft === 0 &&
        (!c.stageScroll || (c.stageScroll.top === 0 && c.stageScroll.left === 0)),
      `${step}: the canvas scrolled (${c.scrollLeft}, ${c.scrollTop})`,
    )
    expect(
      c.regionScroll.scrollHeight <= c.regionScroll.clientHeight &&
        c.regionScroll.scrollWidth <= c.regionScroll.clientWidth,
      `${step}: the canvas region overflows`,
    )
    expect(
      c.escaping.count === 0,
      `${step}: ${c.escaping.count} positioned element(s) escape the canvas: ${JSON.stringify(c.escaping.lowest)}`,
    )
    expect(
      Number.isFinite(c.zoom) && c.zoom >= 10 && c.zoom <= 400,
      `${step}: canvas zoom ${c.zoom}% out of range`,
    )
    expect(
      c.region.height >= 220 && m.inspectorScroll && m.inspectorScroll.box.height >= 120,
      `${step}: canvas ${c.region.height}px / inspector ${m.inspectorScroll?.box.height}px: a region is too small`,
    )
    if (canvasHeight === null) canvasHeight = c.region.height
    expect(
      Math.abs(c.region.height - canvasHeight) <= 1,
      `${step}: the canvas height changed (${canvasHeight} → ${c.region.height})`,
    )
  }
  for (const name of ["workspace", "editorMain", "inspectorScroll", "library"]) {
    const s = m[name]
    expect(
      s && s.escaping.count === 0,
      `${step}: ${s?.escaping.count} positioned element(s) escape ${name}: ${JSON.stringify(s?.escaping.lowest)}`,
    )
  }
  // The editing bar: every button the same height, at least 8px from the
  // bar's top and bottom edges, icons centred, labels not clipped.
  if (m.editingBar) {
    const { box: bar, buttons } = m.editingBar
    for (const b of buttons) {
      expect(
        b.top - bar.top >= 8 - 0.5,
        `${step}: "${b.label}" is ${b.top - bar.top}px from the bar's top`,
      )
      expect(
        bar.bottom - b.bottom >= 8 - 1.5,
        `${step}: "${b.label}" is ${bar.bottom - b.bottom}px from the bar's bottom`,
      )
      expect(
        Math.abs(b.height - buttons[0].height) < 0.5,
        `${step}: "${b.label}" is ${b.height}px high, not ${buttons[0].height}`,
      )
      expect(
        b.iconOffset === null || Math.abs(b.iconOffset) <= 0.5,
        `${step}: "${b.label}" icon is ${b.iconOffset}px off centre`,
      )
      expect(!b.clipped, `${step}: "${b.label}" is clipped`)
    }
  }
  // Keys: one size, one border width and radius in every state; the
  // selected key differs only in its border colour.
  const controls = m.controls ?? []
  if (controls.length > 0) {
    const [first] = controls
    for (const c of controls) {
      expect(
        Math.abs(c.width - first.width) < 0.01 && Math.abs(c.height - first.height) < 0.01,
        `${step}: ${c.control} is ${c.width}×${c.height}, not ${first.width}×${first.height}`,
      )
      expect(
        c.border === first.border,
        `${step}: ${c.control} border ${c.border} ≠ ${first.border}`,
      )
      expect(
        c.radius === first.radius,
        `${step}: ${c.control} radius ${c.radius} ≠ ${first.radius}`,
      )
    }
    expect(Number.parseFloat(first.border) >= 2, `${step}: key border ${first.border} is under 2px`)
    const selected = controls.find((c) => c.selected)
    const plain = controls.find((c) => !c.selected)
    if (selected && plain) {
      expect(
        chroma(selected.borderColor) > 0.08 && chroma(plain.borderColor) < 0.02,
        `${step}: selection is not a coloured border (${selected.borderColor} vs ${plain.borderColor})`,
      )
    }
  }
  const inspector = m.inspectorScroll
  if (inspector?.scrolls && inspector.lowest) {
    const blank = inspector.scrollHeight - inspector.lowest.bottom
    expect(
      blank <= INSPECTOR_PADDING + TOLERANCE,
      `${step}: ${blank}px of blank scroll area below the inspector's content`,
    )
  }
}

/**
 * The gesture cards with a key selected (gesture-carousel.tsx): no
 * horizontal scrolling in their region or in the document; every card in
 * view, and each of its form controls, wholly inside the clipping viewport;
 * cards in view equally wide (±1px); every card, in view or not, equally
 * tall (±1px) and none scrolling inside; empty cards dashed with their
 * hint centred (±2px), assigned ones solid, all 1px; the rails absent when all cards
 * are shown, otherwise both present, outside the cards, labelled, equally
 * sized and as tall as the cards. Older apps without it are skipped.
 */
function gestureCardsInvariants(m, step) {
  const g = m.gestureCards
  if (!g) return
  expect(
    g.scrollWidth <= g.clientWidth + 1,
    `${step}: the gesture cards scroll horizontally (${g.scrollWidth} > ${g.clientWidth})`,
  )
  expect(g.label, `${step}: the gesture cards have no accessible name`)
  expect(
    g.documentWidth.scrollWidth <= g.documentWidth.clientWidth + 1,
    `${step}: the document overflows horizontally (${g.documentWidth.scrollWidth} > ${g.documentWidth.clientWidth})`,
  )
  const v = g.viewport
  expect(v, `${step}: no gesture cards viewport`)
  if (!v) return
  expect(
    v.overflowX === "clip" || v.overflowX === "hidden",
    `${step}: the gesture cards viewport does not clip (overflow-x ${v.overflowX})`,
  )
  expect(g.cards.length >= 1, `${step}: no gesture card in view`)
  const inside = (r) =>
    r.left >= v.rect.left - 0.5 &&
    r.right <= v.rect.right + 0.5 &&
    r.left >= g.rect.left - 0.5 &&
    r.right <= g.rect.right + 0.5
  const widths = g.cards.map((c) => c.rect.right - c.rect.left)
  for (const card of g.cards) {
    expect(
      inside(card.rect),
      `${step}: the card “${card.label}” is clipped (${JSON.stringify(card.rect)} in ${JSON.stringify(v.rect)})`,
    )
    expect(card.label, `${step}: a gesture card has no name`)
    const clipped = card.controls.filter((c) => c.right > c.left && !inside(c))
    expect(
      clipped.length === 0,
      `${step}: ${clipped.length} control(s) of “${card.label}” are clipped: ${JSON.stringify(clipped[0])}`,
    )
  }
  expect(
    Math.max(...widths) - Math.min(...widths) <= 1,
    `${step}: the gesture cards differ in width (${widths.join("/")})`,
  )
  if (g.heights) {
    expect(
      Math.max(...g.heights) - Math.min(...g.heights) <= 1,
      `${step}: the gesture cards differ in height (${g.heights.join("/")})`,
    )
  }
  for (const card of g.cards) {
    expect(!card.scrolls, `${step}: the card “${card.label}” scrolls inside itself`)
    if (card.borderStyle === undefined) continue
    expect(
      card.borderStyle === (card.empty ? "dashed" : "solid"),
      `${step}: the ${card.empty ? "empty" : "assigned"} card “${card.label}” has a ${card.borderStyle} border`,
    )
    expect(
      card.borderWidth === "1px",
      `${step}: the card “${card.label}” border is ${card.borderWidth}`,
    )
    if (card.emptyOffset) {
      expect(
        Math.abs(card.emptyOffset.x) <= 2 && Math.abs(card.emptyOffset.y) <= 2,
        `${step}: the empty state of “${card.label}” is off centre (${JSON.stringify(card.emptyOffset)})`,
      )
    }
  }
  if (!g.overflowing) {
    expect(
      !g.previous && !g.next && g.cards.length === g.count,
      `${step}: all ${g.count} gesture cards fit, yet ${g.cards.length} are shown or arrows are drawn`,
    )
    return
  }
  expect(g.previous && g.next, `${step}: the gesture cards overflow without arrows`)
  for (const arrow of [g.previous, g.next]) {
    if (!arrow) continue
    expect(arrow.label, `${step}: a gesture arrow has no accessible name`)
    const card = g.cards[0]?.rect
    if (card) {
      const h = arrow.rect.bottom - arrow.rect.top
      expect(
        Math.abs(h - (card.bottom - card.top)) <= 1 && Math.abs(arrow.rect.top - card.top) <= 1,
        `${step}: the “${arrow.label}” rail is ${h}px high, not the cards' ${card.bottom - card.top}px`,
      )
    }
    for (const card of g.cards) {
      expect(
        !intersects(arrow.rect, card.rect),
        `${step}: the “${arrow.label}” arrow overlaps the card “${card.label}”`,
      )
    }
  }
  if (g.previous && g.next) {
    const size = (r) => [r.right - r.left, r.bottom - r.top]
    const [pw, ph] = size(g.previous.rect)
    const [nw, nh] = size(g.next.rect)
    expect(
      Math.abs(pw - nw) <= 0.5 && Math.abs(ph - nh) <= 0.5 && pw >= 32 && pw <= 40,
      `${step}: the rails are ${pw}×${ph} and ${nw}×${nh} (equal, 32–40px wide)`,
    )
  }
  expect(g.cards.length < g.count, `${step}: the cards overflow but all ${g.count} are in view`)
}

/** Rectangles overlap (touching edges do not). */
function intersects(a, b) {
  return (
    a.left < b.right - 0.5 &&
    b.left < a.right - 0.5 &&
    a.top < b.bottom - 0.5 &&
    b.top < a.bottom - 0.5
  )
}

/**
 * The page navigation straddles the divider (its centre on the line,
 * centred across the editor), the zoom toolbar sits in the canvas'
 * bottom-right corner 16px in and above the divider, and neither covers a
 * key, the other, or a control of the inspector.
 */
function controlsClear(m, step) {
  const c = m.canvas
  if (!c?.nav || !c.toolbarRect || c.dividerY === null) {
    expect(false, `${step}: no page navigation, zoom toolbar or divider`)
    return
  }
  const { nav, toolbarRect: bar, dividerY, main } = c
  expect(
    Math.abs((nav.top + nav.bottom) / 2 - dividerY) <= 1,
    `${step}: the page navigation's centre ${(nav.top + nav.bottom) / 2} is not on the divider ${dividerY}`,
  )
  expect(
    main && Math.abs((nav.left + nav.right) / 2 - (main.left + main.right) / 2) <= 1,
    `${step}: the page navigation is not centred across the editor`,
  )
  // The zoom bar: 16px from the viewport's right and bottom edges (the
  // bottom edge is the divider), the same both ways, inside the canvas;
  // higher only in a window so narrow that the navigation is under it.
  const v = c.viewportRect
  const insets = c.toolbarInsets
  const shared = bar.left < nav.right && nav.left < bar.right
  expect(Math.abs(v.bottom - dividerY) <= 1, `${step}: the canvas does not end at the divider`)
  expect(
    insets && Math.abs(insets.right - 16) <= 1,
    `${step}: the zoom bar is ${insets?.right}px from the canvas' right edge (needs 16)`,
  )
  expect(
    insets && (shared ? insets.bottom >= 16 - 1 : Math.abs(insets.bottom - 16) <= 1),
    `${step}: the zoom bar is ${insets?.bottom}px from the canvas' bottom edge (needs 16)`,
  )
  expect(
    shared || (insets && Math.abs(insets.right - insets.bottom) <= 1),
    `${step}: the zoom bar's insets differ (right ${insets?.right}, bottom ${insets?.bottom})`,
  )
  expect(
    bar.left >= v.left && bar.top >= v.top && bar.right <= v.right && bar.bottom <= v.bottom,
    `${step}: the zoom bar is not inside the canvas`,
  )
  expect(!intersects(bar, nav), `${step}: the zoom toolbar overlaps the page navigation`)
  for (const [name, r] of [
    ["page navigation", nav],
    ["zoom toolbar", bar],
  ]) {
    const keys = c.keys.filter((k) => intersects(k, r))
    expect(
      keys.length === 0,
      `${step}: the ${name} covers ${keys.length} key(s): ${JSON.stringify(keys[0])}`,
    )
    const controls = c.inspectorControls.filter((k) => intersects(k, r))
    expect(
      controls.length === 0,
      `${step}: the ${name} covers ${controls.length} inspector control(s): ${JSON.stringify(controls[0])}`,
    )
  }
}

/**
 * The inspector's top with a control selected: its deselect button (X) is
 * 4–12px below the divider and 12–16px (±1) from the inspector's right
 * edge (left of a scrollbar), inside the inspector, clear of the page
 * navigation and of every form control; the first section starts at most
 * 16px below the page navigation's lower half (no heading band above it).
 * `scrolled`: the inspector is scrolled down, the X must still be there.
 */
function inspectorTopClear(m, step, { scrolled = false } = {}) {
  const top = m.inspectorTop
  if (!top) return // an older app without the measurement
  const x = top.deselect?.rect
  expect(x, `${step}: no deselect button in the inspector`)
  if (!x || top.dividerY === null) return
  const below = Math.round((x.top - top.dividerY) * 10) / 10
  expect(below >= 4 && below <= 12, `${step}: the X is ${below}px below the divider (needs 4–12)`)
  const right = Math.round((top.clientRight - x.right) * 10) / 10
  expect(
    right >= 12 - 1 && right <= 16 + 1,
    `${step}: the X is ${right}px from the inspector's right edge (needs 12–16)`,
  )
  const s = top.scroll
  expect(
    x.left >= s.left && x.right <= top.clientRight && x.top >= s.top && x.bottom <= s.bottom,
    `${step}: the X is not inside the inspector (${JSON.stringify(x)} in ${JSON.stringify(s)})`,
  )
  expect(!top.nav || !intersects(x, top.nav), `${step}: the X overlaps the page navigation`)
  expect(
    top.covered.length === 0,
    `${step}: the X covers ${top.covered.length} form control(s): ${JSON.stringify(top.covered[0])}`,
  )
  expect(top.deselect.label, `${step}: the X has no accessible name`)
  if (!scrolled && top.firstSection && top.nav) {
    const lowerHalf = (top.nav.bottom - top.nav.top) / 2
    const gap = Math.round((top.firstSection.rect.top - top.dividerY) * 10) / 10
    expect(
      gap <= lowerHalf + 16 + 0.5,
      `${step}: the first section (${top.firstSection.element}) starts ${gap}px below the divider (at most ${lowerHalf + 16})`,
    )
  }
}

/** The canvas height of the first measure: every later one must match. */
let canvasHeight = null

const keys = (await probe("keys")).filter((k) => typeof k === "string")
if (keys.length === 0) {
  console.error("No device controls found: open the Device Editor in the main window.")
  process.exit(2)
}

{
  const initial = await probe("measure")
  invariants(initial, "initial")
}
const heights = new Map()
for (const round of [1, 2]) {
  for (const key of keys) {
    await probe("select", { control: key })
    // The inspector fills in asynchronously (e.g. the text-fit report of the
    // preview); compare heights once two measurements 400ms apart agree, so a
    // late line is not taken for growth. Real accumulation stays different.
    let m = await probe("measure")
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 400))
      const again = await probe("measure")
      const same = again.inspectorScroll?.scrollHeight === m.inspectorScroll?.scrollHeight
      m = again
      if (same) break
    }
    const step = `round ${round}, ${key}`
    invariants(m, step)
    if (m.selected) inspectorTopClear(m, step)
    if (m.selected) gestureCardsInvariants(m, step)
    const height = m.inspectorScroll?.scrollHeight
    if (round === 1) heights.set(key, height)
    else
      expect(
        Math.abs(height - heights.get(key)) <= TOLERANCE,
        `${step}: the editor's height changed on reselection (${heights.get(key)} → ${height})`,
      )
  }
}

// Pages: the canvas keeps its height and its zoom.
const zoomBefore = (await probe("measure")).canvas?.zoom
for (const to of ["next", "previous"]) {
  await probe("page", { to })
  const m = await probe("measure")
  invariants(m, `page ${to}`)
  expect(
    m.canvas?.zoom === zoomBefore,
    `page ${to}: the zoom changed (${zoomBefore} → ${m.canvas?.zoom})`,
  )
}

// The page navigation dock: the counter unfolds the Page Navigator under
// the toolbar, which keeps its size and place; the panel is wider where
// the central area allows (one outline, with shoulders), over the top of
// the inspector, moving nothing; Escape and the counter fold it again.
{
  if ((await probe("measure")).pageNav?.dock?.state === "expanded") {
    // Left open (by hand or an earlier run).
    await probe("dock", { to: "close" })
  }
  const before = await probe("measure")
  expect(before.pageNav?.dock?.state === "collapsed", "dock: not collapsed at first")
  expect(!before.pageNav?.dock?.outline, "dock: an outline while collapsed")
  await probe("dock", { to: "open" })
  const open = await probe("measure")
  invariants(open, "dock open")
  const nav = open.pageNav
  const dock = nav?.dock
  const panel = nav?.navigator?.rect
  const bar = nav?.rect
  const near = (a, b, d) => Math.abs(a - b) <= d
  const same = (a, b) =>
    a && b && ["left", "top", "right", "bottom"].every((k) => near(a[k], b[k], 0.5))
  expect(
    dock?.state === "expanded" && dock.expanded === "true",
    "dock: the counter did not unfold it",
  )
  expect(panel, "dock: no Page Navigator panel")
  expect(dock?.floating === 0, `dock: ${dock?.floating} popover element(s) while expanded`)
  // The toolbar and each of its controls stay exactly where they were.
  expect(
    same(bar, before.pageNav?.rect),
    `dock: the toolbar moved or resized (${JSON.stringify(before.pageNav?.rect)} → ${JSON.stringify(bar)})`,
  )
  const controlsBefore = before.pageNav?.dock?.controls ?? []
  expect(
    dock?.controls?.length === controlsBefore.length &&
      dock.controls.every((c, index) => same(c.rect, controlsBefore[index]?.rect)),
    "dock: a toolbar control moved",
  )
  if (panel && bar) {
    const width = (r) => r.right - r.left
    const centre = (r) => (r.left + r.right) / 2
    expect(
      near(panel.top, bar.bottom, 1),
      `dock: the panel starts at ${panel.top}, not under the toolbar (${bar.bottom})`,
    )
    expect(
      near(centre(panel), centre(bar), 1),
      `dock: the panel is off the toolbar's centre by ${Math.round(centre(panel) - centre(bar))}px`,
    )
    expect(width(panel) >= width(bar) - 0.5, "dock: the panel is narrower than the toolbar")
    const footer = nav.footer
    const zoomBelow = dock.zoom && dock.zoom.bottom > bar.bottom + 0.5
    if (footer && !zoomBelow) {
      const room = 2 * Math.min(centre(bar) - footer.left, footer.right - centre(bar)) - 2 * 48
      if (room >= width(bar) + 32)
        expect(
          width(panel) > width(bar) + 31,
          `dock: the panel is no wider than the toolbar (${width(panel)})`,
        )
    }
    // One outline: drawn behind both; neither part has its own at the joint.
    expect(dock.outline, "dock: no outline")
    if (dock.outline) {
      expect(
        near(dock.outline.top, bar.top, 0.5) &&
          near(dock.outline.left, panel.left, 0.5) &&
          near(dock.outline.right, panel.right, 0.5) &&
          near(dock.outline.bottom, panel.bottom, 0.5),
        `dock: the outline ${JSON.stringify(dock.outline)} does not cover the toolbar and the panel`,
      )
    }
    expect(dock.panel?.borderTop === "0px", "dock: the panel has a top border of its own")
    const clear = (c) => /rgba\(\d+, \d+, \d+, 0\)|transparent/.test(c ?? "")
    expect(
      clear(dock.bar?.borderColor) && clear(dock.bar?.background),
      "dock: the toolbar keeps its own border or surface",
    )
    expect(
      clear(dock.panel?.borderColor) && clear(dock.panel?.background),
      "dock: the panel has its own border or surface",
    )
    expect(
      dock.panel?.boxShadow === "none" && dock.bar?.boxShadow === "none",
      "dock: a separate shadow",
    )
    expect(
      panel.bottom <= open.viewport.height && panel.left >= 0 && panel.right <= open.viewport.width,
      `dock: the panel is cut off (${JSON.stringify(panel)})`,
    )
    const x = open.inspectorTop?.deselect?.rect
    if (x) {
      expect(!intersects(x, panel), "dock: the panel covers the inspector's X")
      expect(x.left - panel.right >= 0, "dock: the panel reaches into the X's column")
    }
    if (dock.library)
      expect(!intersects(dock.library, panel), "dock: the panel covers the Action Library")
    if (dock.zoom) expect(!intersects(dock.zoom, panel), "dock: the panel covers the zoom toolbar")
  }
  // Nothing under it moved.
  expect(
    open.canvas?.transform === before.canvas?.transform,
    `dock: the device moved (${before.canvas?.transform} → ${open.canvas?.transform})`,
  )
  const first = (m) => m.inspectorTop?.firstSection?.rect
  expect(
    JSON.stringify(first(open)) === JSON.stringify(first(before)),
    `dock: the inspector's content moved (${JSON.stringify(first(before))} → ${JSON.stringify(first(open))})`,
  )
  expect(
    open.inspectorScroll?.scrollHeight === before.inspectorScroll?.scrollHeight,
    "dock: the inspector's height changed",
  )
  await probe("dock", { to: "escape" })
  let after = await probe("measure")
  expect(after.pageNav?.dock?.state === "collapsed", "dock: Escape did not fold it")
  expect(!after.pageNav?.navigator, "dock: the panel stays after Escape")
  expect(
    same(after.pageNav?.rect, before.pageNav?.rect),
    "dock: the toolbar is not as it was after folding",
  )
  await probe("dock", { to: "open" })
  await probe("dock", { to: "close" })
  after = await probe("measure")
  expect(after.pageNav?.dock?.state === "collapsed", "dock: the counter did not fold it")
  invariants(after, "dock folded")
  // Renaming in place: the field takes the segment's height; nothing moves.
  await probe("dock", { to: "rename" })
  const renaming = await probe("measure")
  expect(renaming.pageNav?.rename, "rename: no field after a double click on the page name")
  invariants(renaming, "renaming the page")
  expect(
    Math.abs(renaming.pageNav.rect.top - before.pageNav.rect.top) <= 0.5 &&
      Math.abs(renaming.pageNav.rect.bottom - before.pageNav.rect.bottom) <= 0.5,
    "rename: the page navigation moved vertically",
  )
  await probe("dock", { to: "rename-cancel" })
  after = await probe("measure")
  expect(!after.pageNav?.rename, "rename: Escape did not end it")
  expect(same(after.pageNav?.rect, before.pageNav?.rect), "rename: the toolbar is not as it was")
}

// Zoom: in to the limit and out to it, the canvas never scrolls; keys stay
// one size (checked by the invariants); then back to Fit to screen.
const zooms = []
for (const action of ["actual", "zoom-in", "zoom-in", "zoom-in", "zoom-in", "zoom-in", "zoom-in"]) {
  await probe("canvas", { action })
  const m = await probe("measure")
  invariants(m, `canvas ${action}`)
  zooms.push(m.canvas?.zoom)
}
for (let i = 0; i < 14; i++) await probe("canvas", { action: "zoom-out" })
let m = await probe("measure")
invariants(m, "canvas zoomed out to the limit")
zooms.push(m.canvas?.zoom)
expect(m.canvas?.zoom === 10, `zooming out stops at 10% (got ${m.canvas?.zoom})`)
await probe("canvas", { action: "fit" })
m = await probe("measure")
invariants(m, "canvas fitted")
const device = m.canvas?.device
const viewportBox = m.canvas?.viewport
expect(
  device && viewportBox && device.top >= viewportBox.top && device.bottom <= viewportBox.bottom,
  `fit: the device ${JSON.stringify(device)} is not inside the viewport`,
)
controlsClear(m, "fit")
const fixed = { nav: m.canvas.nav, toolbar: m.canvas.toolbarRect }

// A click on the background focuses the canvas without a focus ring.
await probe("canvas", { action: "pointer-focus" })
m = await probe("measure")
const focus = m.canvas?.focus
expect(focus?.focused, "pointer focus: the canvas did not take focus")
expect(
  focus &&
    (focus.outlineStyle === "none" || focus.outlineWidth === "0px") &&
    focus.boxShadow === "none",
  `pointer focus shows a ring: outline ${focus?.outlineStyle} ${focus?.outlineWidth}, box-shadow ${focus?.boxShadow}`,
)

// The canvas controls do not move with the device or the inspector.
const unmoved = (step) => {
  const c = m.canvas
  for (const [name, before] of Object.entries(fixed)) {
    const now = name === "nav" ? c?.nav : c?.toolbarRect
    expect(
      now &&
        before &&
        ["left", "top", "right", "bottom"].every((k) => Math.abs(now[k] - before[k]) <= 0.5),
      `${step}: the ${name} moved (${JSON.stringify(before)} → ${JSON.stringify(now)})`,
    )
  }
}
await probe("canvas", { action: "pan" })
m = await probe("measure")
invariants(m, "canvas panned")
unmoved("canvas panned")
await probe("canvas", { action: "zoom-in" })
m = await probe("measure")
invariants(m, "canvas zoomed in")
unmoved("canvas zoomed in")
await probe("scroll", { slot: "inspector-scroll", to: "bottom" })
m = await probe("measure")
unmoved("inspector scrolled to the bottom")
await probe("scroll", { slot: "inspector-scroll", to: "top" })
await probe("canvas", { action: "fit" })

// Dragging the canvas: the device follows the pointer 1:1 at every zoom
// (left button on the background, middle button), selects nothing, and Fit
// always brings it back, also from far outside the view.
const shift = (t) => (t.match(/-?[\d.]+/g) ?? []).map(Number)
const selectedBefore = (await probe("measure")).selected
for (const action of ["zoom-out", "zoom-in", "zoom-in", "zoom-in"]) {
  await probe("canvas", { action })
  for (const [dx, dy, button] of [
    [90, 40, 0],
    [-70, -30, 1],
  ]) {
    const { before, after } = await probe("canvas", { action: "drag", dx, dy, button })
    const [bx, by] = shift(before.transform)
    const [ax, ay] = shift(after.transform)
    expect(
      Math.abs(ax - bx - dx) <= 1 && Math.abs(ay - by - dy) <= 1,
      `drag ${dx},${dy} (button ${button}) at ${after.zoom}% moved the device ${ax - bx},${ay - by}`,
    )
    expect(!after.panning, `a pan at ${after.zoom}% did not end`)
  }
}
expect(
  (await probe("measure")).selected === selectedBefore,
  "dragging the canvas changed the selection",
)
await probe("canvas", { action: "drag", dx: -2000, dy: -2000 })
const lost = await probe("canvas", { action: "state" })
expect(lost.grid.bottom > lost.viewport.top, "the device was dragged completely out of reach")
await probe("canvas", { action: "fit" })
const fitted = await probe("canvas", { action: "state" })
expect(
  fitted.grid.top >= fitted.viewport.top - 1 &&
    fitted.grid.bottom <= fitted.viewport.bottom + 1 &&
    fitted.grid.height > 0,
  `Fit did not bring the device back (${JSON.stringify(fitted.grid)} in ${JSON.stringify(fitted.viewport)})`,
)

// The inspector curtain: hidden at the top, shown once scrolled, anchored
// under the divider, under the dock and the deselect button, and clicks pass
// through it.
await probe("select", { control: keys[Math.min(8, keys.length - 1)] })
await probe("scroll", { slot: "inspector-scroll", to: "top" })
await new Promise((r) => setTimeout(r, 300))
let c = (await probe("measure")).curtain
if (c) {
  expect(c.opacity === 0 && !c.scrolled, `curtain shows at the top (opacity ${c.opacity})`)
  const top = await probe("measure")
  await probe("scroll", { slot: "inspector-scroll", to: "bottom" })
  await new Promise((r) => setTimeout(r, 300))
  const after = await probe("measure")
  c = after.curtain
  if (after.inspectorScroll?.scrolls) {
    expect(c.scrolled && c.opacity > 0.95, `curtain hidden after scrolling (opacity ${c.opacity})`)
  }
  expect(c.pointerEvents === "none", `curtain takes pointer events (${c.pointerEvents})`)
  expect(
    !String(c.hitInside).includes("inspector-curtain"),
    `a pointer inside the curtain reaches ${c.hitInside}`,
  )
  expect(
    c.hitDock === null || !String(c.hitDock).includes("inspector-curtain"),
    `the curtain covers the page dock (${c.hitDock})`,
  )
  expect(
    c.hitDeselect === null || !String(c.hitDeselect).includes("inspector-curtain"),
    `the curtain covers the deselect button (${c.hitDeselect})`,
  )
  const divider = after.canvas?.dividerY
  if (divider !== undefined) {
    expect(
      Math.abs(c.rect.top - divider) <= 1.5,
      `curtain top ${c.rect.top} is not at the divider ${divider}`,
    )
  }
  expect(
    Math.abs((top.inspectorScroll?.box.height ?? 0) - (after.inspectorScroll?.box.height ?? 0)) <=
      0.5,
    "the curtain changed the inspector's height",
  )
  expect(!after.document.scrolls, "the document scrolls with the curtain")
  await probe("scroll", { slot: "inspector-scroll", to: "top" })
}

// The gesture cards' rails: moving the cards changes neither the cards'
// height nor the inspector's scroll position, for every key that overflows.
for (const key of keys) {
  await probe("select", { control: key })
  const before = await probe("measure")
  if (!before.gestureCards?.overflowing) continue
  await probe("scroll", { slot: "inspector-scroll", to: "bottom" })
  const scrolled = (await probe("measure")).inspectorScroll.scrollTop
  for (const to of ["next", "next", "previous", "previous"]) {
    const moved = await probe("gestures", { to })
    const m = await probe("measure")
    const step = `gestures ${to} on ${key}`
    gestureCardsInvariants(m, step)
    if (moved.disabled) continue
    const a = before.gestureCards.rect
    const b = m.gestureCards?.rect
    expect(
      b && Math.abs(b.bottom - b.top - (a.bottom - a.top)) <= 0.5,
      `${step}: the gesture cards' height changed (${a.bottom - a.top} → ${b ? b.bottom - b.top : "none"})`,
    )
    expect(
      m.inspectorScroll.scrollTop === scrolled,
      `${step}: the inspector scrolled (${scrolled} → ${m.inspectorScroll.scrollTop})`,
    )
  }
  await probe("scroll", { slot: "inspector-scroll", to: "top" })
}

// Independent scrolling: the library and the inspector move only themselves.
await probe("select", { control: keys[Math.min(8, keys.length - 1)] })
await probe("scroll", { slot: "inspector-scroll", to: "bottom" })
m = await probe("measure")
invariants(m, "inspector scrolled to the bottom")
inspectorTopClear(m, "inspector scrolled to the bottom", { scrolled: true })
expect(
  m.inspectorScroll.scrollTop > 0 || !m.inspectorScroll.scrolls,
  "the inspector did not scroll to the bottom",
)
expect(m.workspace.scrollTop === 0, "scrolling the inspector moved the workspace")
const inspectorTop = m.inspectorScroll.scrollTop
await probe("scroll", { slot: "library-scroll", to: "bottom" })
m = await probe("measure")
invariants(m, "library scrolled to the bottom")
expect(m.inspectorScroll.scrollTop === inspectorTop, "scrolling the library moved the inspector")
expect(m.workspace.scrollTop === 0, "scrolling the library moved the workspace")
await probe("scroll", { slot: "library-scroll", to: "top" })
await probe("scroll", { slot: "inspector-scroll", to: "top" })

// The key header and its title appearance popover (docs/key-text.md, "Title
// appearance"): for every key that has one, the header fits the inspector
// without scrolling sideways; the popover opens inside the window, on top of
// the inspector and its curtain (a portal), with nothing in it scrolling,
// compact 32px controls, and the inspector exactly as it was; Escape closes
// it and returns everything as it was.
let titled = 0
for (const key of keys) {
  if (titled >= 4) break
  await probe("select", { control: key })
  const before = await probe("measure")
  const look = before.titleLook
  if (!look?.trigger) continue
  titled++
  const step = `title popover of ${key}`
  expect(!look.header.overflowsX, `${step}: the key header scrolls sideways`)
  expect(
    look.header.rect.right - look.header.rect.left <= before.inspector.width + 1,
    `${step}: the key header is wider than the inspector`,
  )
  expect(
    look.header.preview && look.header.preview.right - look.header.preview.left >= 48,
    `${step}: the key preview is missing or too small`,
  )
  await probe("title", { to: "open" })
  const open = await probe("measure")
  invariants(open, `${step} open`)
  const p = open.titleLook
  expect(p.popover !== null, `${step}: the popover did not open`)
  if (p.popover) {
    const { left, top, right, bottom } = p.popover
    expect(
      left >= 0 && top >= 0 && right <= p.window.width && bottom <= p.window.height,
      `${step}: the popover leaves the window (${JSON.stringify(p.popover)} in ${p.window.width}×${p.window.height})`,
    )
    expect(p.inPortal, `${step}: the popover is inside the inspector's scroll region`)
    expect(p.onTop, `${step}: something covers the popover`)
    expect(
      p.curtainZ === null || p.popoverZ > p.curtainZ,
      `${step}: the popover (z ${p.popoverZ}) is not above the curtain (z ${p.curtainZ})`,
    )
    expect(p.scrollers.length === 0, `${step}: scrolls inside: ${p.scrollers.join(", ")}`)
    expect(
      p.controls.length > 0 && p.controls.every((h) => Math.abs(h - 32) <= 1),
      `${step}: controls are not 32px high (${p.controls.join(", ")})`,
    )
  }
  expect(
    JSON.stringify(open.inspector) === JSON.stringify(before.inspector),
    `${step}: the inspector changed size (${JSON.stringify(before.inspector)} → ${JSON.stringify(open.inspector)})`,
  )
  expect(
    open.inspectorScroll.scrollTop === before.inspectorScroll.scrollTop,
    `${step}: the inspector scrolled`,
  )
  await probe("title", { to: "escape" })
  const closed = await probe("measure")
  expect(closed.titleLook.popover === null, `${step}: Escape did not close the popover`)
  expect(
    JSON.stringify(closed.inspector) === JSON.stringify(before.inspector),
    `${step}: the inspector changed after closing`,
  )
}
expect(titled > 0, "no key with a title appearance button was found")

// The key menu and its pickers (docs/key-text.md, "Icons and backgrounds"):
// for a few keys with a header, there is no Background row and Show title
// sits under the title; toggling it twice keeps the header's height; the
// preview's menu and each picker (Icon Library, Background Library, custom
// colour) open inside the window, on top of the inspector and its curtain
// (a portal), with nothing scrolling but a library's own grid, the
// inspector exactly as it was; Escape closes them, focus back on the
// preview.
let menus = 0
for (const key of keys) {
  if (menus >= 2) break
  await probe("select", { control: key })
  const before = await probe("measure")
  const look = before.keyMenu
  if (!look?.header) continue
  menus++
  const step = `key menu of ${key}`
  expect(!look.backgroundRow, `${step}: the header still has a Background row`)
  expect(
    look.title && look.showTitle && look.showTitle.top >= look.title.bottom - 1,
    `${step}: Show title is not under the title (${JSON.stringify(look.title)} / ${JSON.stringify(look.showTitle)})`,
  )
  await probe("keymenu", { to: "show-title" })
  const hidden = await probe("measure")
  expect(
    hidden.keyMenu.header &&
      Math.abs(
        hidden.keyMenu.header.bottom -
          hidden.keyMenu.header.top -
          (look.header.bottom - look.header.top),
      ) <= 0.5,
    `${step}: toggling Show title changed the header's height`,
  )
  await probe("keymenu", { to: "show-title" })
  await probe("keymenu", { to: "menu" })
  const opened = await probe("measure")
  const menu = opened.keyMenu.menu
  expect(
    menu !== null && opened.keyMenu.menuItems >= 5,
    `${step}: the menu did not open with its items`,
  )
  if (menu) {
    expect(
      menu.left >= 0 &&
        menu.top >= 0 &&
        menu.right <= opened.keyMenu.window.width &&
        menu.bottom <= opened.keyMenu.window.height,
      `${step}: the menu leaves the window (${JSON.stringify(menu)})`,
    )
  }
  await probe("keymenu", { to: "escape" })
  for (const picker of ["icons", "backgrounds", "colour"]) {
    const at = `${step}, ${picker}`
    await probe("keymenu", { to: picker })
    const open = await probe("measure")
    invariants(open, `${at} open`)
    const p = open.keyMenu
    expect(p.picker !== null && p.pickerKind === picker, `${at}: the picker did not open`)
    if (p.picker) {
      const { left, top, right, bottom } = p.picker
      expect(
        left >= 0 && top >= 0 && right <= p.window.width && bottom <= p.window.height,
        `${at}: the picker leaves the window (${JSON.stringify(p.picker)} in ${p.window.width}×${p.window.height})`,
      )
      expect(p.inPortal, `${at}: the picker is inside the inspector's scroll region`)
      expect(p.onTop, `${at}: something covers the picker`)
      expect(
        p.curtainZ === null || p.pickerZ > p.curtainZ,
        `${at}: the picker (z ${p.pickerZ}) is not above the curtain (z ${p.curtainZ})`,
      )
      expect(p.scrollers.length === 0, `${at}: scrolls besides its grid: ${p.scrollers.join(", ")}`)
      if (picker !== "colour") expect(p.tiles > 3, `${at}: only ${p.tiles} tiles`)
      expect(p.menu === null, `${at}: the menu is still open under the picker`)
    }
    expect(
      JSON.stringify(open.inspector) === JSON.stringify(before.inspector),
      `${at}: the inspector changed size`,
    )
    await probe("keymenu", { to: "escape" })
    const closed = await probe("measure")
    expect(closed.keyMenu.picker === null, `${at}: Escape did not close the picker`)
    expect(closed.keyMenu.focusOnPreview, `${at}: the focus did not go back to the preview`)
  }
  const after = await probe("measure")
  expect(
    JSON.stringify(after.inspector) === JSON.stringify(before.inspector),
    `${step}: the inspector changed after the menu`,
  )
}
expect(menus > 0, "no key with a key menu was found")

const { viewport } = m
if (failures.length > 0) {
  console.error(
    `Layout check FAILED at ${viewport.width}×${viewport.height} (${failures.length} of ${checks} checks):`,
  )
  for (const failure of failures) console.error(`  - ${failure}`)
  process.exit(1)
}
console.log(
  `Layout check passed at ${viewport.width}×${viewport.height}: ${checks} checks, ${keys.length} controls selected twice, ${titled} title popovers, ${menus} key menus, canvas ${canvasHeight}px high, zooms ${zooms.join("/")}%.`,
)
