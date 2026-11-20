// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { DndContext, type DragEndEvent, type DragMoveEvent } from "@dnd-kit/core"
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState, type ReactNode } from "react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { decodeTarget, encodeTarget } from "@/layout-editor/targets"
import type { ActionType, BindingTarget, ControlId, DeviceLayout } from "@/lib/types"
import { installDomStubs } from "@/test/dom"

import { DRAG_EDGE_DELAY_MS, carouselGeometry } from "./carousel-geometry"
import { GestureEditor, type GestureTargetGroup } from "./gesture-editor"
import { gestureSlotId } from "./model"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

/** The drop listeners and drop targets the cards registered, as dnd-kit keeps them. */
const dnd = vi.hoisted(() => ({
  monitors: new Set<{
    onDragEnd?: (event: DragEndEvent) => void
    onDragMove?: (event: DragMoveEvent) => void
  }>(),
  droppables: new Set<string>(),
  /** A drop target a drag is over, and what is dragged. */
  over: null as { id: string; data: unknown } | null,
}))

vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  const react = await import("react")
  return {
    ...actual,
    useDroppable: (args: Parameters<typeof actual.useDroppable>[0]) => {
      if (args.disabled) dnd.droppables.delete(String(args.id))
      else dnd.droppables.add(String(args.id))
      const result = actual.useDroppable(args)
      if (dnd.over?.id !== String(args.id)) return result
      return {
        ...result,
        isOver: true,
        active: { id: "dragged", data: { current: dnd.over.data } },
      } as unknown as typeof result
    },
    useDndMonitor: (listener: Parameters<typeof actual.useDndMonitor>[0]) => {
      const current = react.useRef(listener)
      current.current = listener
      react.useEffect(() => {
        const entry = {
          onDragEnd: (event: DragEndEvent) => current.current.onDragEnd?.(event),
          onDragMove: (event: DragMoveEvent) => current.current.onDragMove?.(event),
        }
        dnd.monitors.add(entry)
        return () => {
          dnd.monitors.delete(entry)
        }
      }, [])
      actual.useDndMonitor(listener)
    },
  }
})

/** The width the cards' region is measured at (a ResizeObserver stand-in). */
let regionWidth = 1200
class FixedResizeObserver {
  callback: ResizeObserverCallback
  constructor(callback: ResizeObserverCallback) {
    this.callback = callback
  }
  observe() {
    const entry = { contentRect: { width: regionWidth, height: 200 } }
    this.callback([entry as unknown as ResizeObserverEntry], this)
  }
  unobserve() {}
  disconnect() {}
}

const KEY: ControlId = { kind: "key", index: 1 }

const pluginType: ActionType = {
  id: "acme.notify",
  name: "Notify",
  description: "",
  category: "Acme",
  requiresProject: false,
  parameters: [
    {
      key: "level",
      label: "Level",
      kind: { type: "choice", options: ["info", "warn"] },
      required: false,
    },
  ],
}
const profile = {
  actions: [
    { id: "shop", name: "Shop", actionType: pluginType.id, parameters: {}, icon: null },
    { id: "mail", name: "Mail", actionType: "general.log", parameters: {}, icon: null },
  ],
  workflows: [{ id: "deploy", name: "Deploy", steps: [] }],
  projects: [],
} as unknown as Parameters<typeof GestureEditor>[0]["profile"]

const groups: GestureTargetGroup[] = [
  {
    label: "Actions",
    options: [
      { value: "action:shop", label: "Shop", target: { type: "action", action: "shop" } },
      { value: "action:mail", label: "Mail", target: { type: "action", action: "mail" } },
    ],
  },
  {
    label: "Workflows",
    options: [
      {
        value: "workflow:deploy",
        label: "Deploy",
        target: { type: "workflow", workflow: "deploy" },
      },
    ],
  },
]

function layoutWith(bindings: DeviceLayout["pages"][number]["bindings"]): DeviceLayout {
  return {
    id: "layout",
    name: "Main",
    model: "elgato.streamdeck.mk2",
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  }
}

/** Key 2 with a single click on Shop and a double click on Deploy. */
const gestureKey = layoutWith([
  { control: KEY, trigger: { type: "click" }, target: { type: "action", action: "shop" } },
  {
    control: KEY,
    trigger: { type: "doubleClick" },
    target: { type: "workflow", workflow: "deploy" },
  },
])

let latest: DeviceLayout | null = null

function Harness({
  initial,
  resolveDrop,
  issues = [],
  incompatible,
}: {
  initial: DeviceLayout
  resolveDrop?: (dragged: unknown) => BindingTarget | null
  issues?: string[]
  incompatible?: (target: BindingTarget) => string | null
}): ReactNode {
  const [draft, setDraft] = useState(initial)
  return (
    <DndContext>
      <GestureEditor
        draft={draft}
        pageId="home"
        control={KEY}
        profile={profile}
        groups={groups}
        encode={encodeTarget}
        decode={decodeTarget}
        types={[pluginType]}
        defaults={{ doubleClickMs: 300, longPressMs: 700 }}
        issues={issues}
        resolveDrop={resolveDrop}
        incompatible={incompatible}
        onChange={(next) => {
          latest = next
          setDraft(next)
        }}
      />
    </DndContext>
  )
}

function renderCards(initial: DeviceLayout, extra: Partial<Parameters<typeof Harness>[0]> = {}) {
  const user = userEvent.setup()
  render(<Harness initial={initial} {...extra} />)
  return user
}

const region = () => screen.getByRole("group", { name: "Gestures" })
const cards = () => [...document.querySelectorAll<HTMLElement>('[data-slot="gesture-card"]')]
const shown = () =>
  [...document.querySelectorAll<HTMLElement>("[data-carousel-item][data-visible]")].map(
    (item) => within(item).getAllByRole("group")[0]?.getAttribute("aria-labelledby") ?? "",
  )
const bindingsOf = (layout: DeviceLayout | null) =>
  layout?.pages[0]?.bindings.map((b) => [b.trigger.type, b.target]) ?? []

beforeAll(() => {
  installDomStubs()
})
beforeEach(() => {
  latest = null
  regionWidth = 1200
  dnd.monitors.clear()
  dnd.droppables.clear()
  dnd.over = null
  vi.stubGlobal("ResizeObserver", FixedResizeObserver)
})
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe("gesture cards", () => {
  it("draws every gesture with one shared card, labelled by its name", () => {
    renderCards(gestureKey)
    expect(region()).toBeInTheDocument()
    expect(cards()).toHaveLength(3)
    for (const name of ["Single press", "Double click", "Long press"]) {
      expect(within(region()).getByRole("group", { name })).toHaveAttribute(
        "data-slot",
        "gesture-card",
      )
    }
    // Independent assignments, each in its own card.
    expect(screen.getByRole("combobox", { name: "Single press" })).toHaveTextContent("Shop")
    expect(screen.getByRole("combobox", { name: "Double click" })).toHaveTextContent("Deploy")
  })

  it("keeps the raw release as a card of the same kind while the key has no gestures", () => {
    renderCards(layoutWith([]))
    expect(cards()).toHaveLength(4)
    expect(screen.getByRole("group", { name: "Release" })).toHaveAttribute(
      "data-slot",
      "gesture-card",
    )
  })

  it("names the release plainly and explains it in a help tooltip", async () => {
    const user = renderCards(layoutWith([]), { resolveDrop: () => null })
    const release = screen.getByRole("group", { name: "Release" })
    // Empty: dashed, the same drop presentation as the gestures.
    expect(release).toHaveClass("border-dashed")
    expect(release).toHaveTextContent(/^ReleaseDrag an action here$/)
    expect(release).toHaveAttribute("data-gesture-slot", "empty")
    const explanation = /Runs without delay when the key is released/
    expect(within(release).queryByText(explanation)).toBeNull()
    // The help button sits right after the name.
    const help = within(release).getByRole("button", { name: "Help: Release" })
    expect(within(release).getByText("Release").nextElementSibling).toBe(help)

    const tip = () => screen.queryByRole("tooltip")
    await user.hover(help)
    expect(await screen.findByRole("tooltip")).toHaveTextContent(explanation)
    await user.unhover(help)
    await user.keyboard("{Escape}")
    await waitFor(() => {
      expect(tip()).toBeNull()
    })
    // Keyboard focus opens it.
    act(() => {
      help.focus()
    })
    expect(await screen.findByRole("tooltip")).toHaveTextContent(explanation)
    // In a portal, outside the cards' clipping.
    const content = document.querySelector('[data-slot="tooltip-content"]')
    expect(region().contains(content)).toBe(false)
    act(() => {
      help.blur()
    })
    await waitFor(() => {
      expect(tip()).toBeNull()
    })
    // A click (a tap) toggles it.
    fireEvent.click(help)
    expect(await screen.findByRole("tooltip")).toHaveTextContent(explanation)
    fireEvent.click(help)
    await waitFor(() => {
      expect(tip()).toBeNull()
    })
  })

  it("shows an empty gesture as a dashed drop card without an add button", async () => {
    const user = renderCards(gestureKey, { resolveDrop: () => null })
    const long = screen.getByRole("group", { name: "Long press" })
    // Its name, an icon and one line, nothing else; no visible button.
    expect(long).toHaveTextContent(/^Long pressDrag an action here$/)
    expect(long).toHaveClass("border", "border-dashed")
    expect(within(long).queryByText(/Add action/)).toBeNull()
    expect(within(long).queryByRole("button")).toBeNull()
    const body = within(long).getByRole("combobox", { name: "Assign an action to: Long press" })
    expect(body).toHaveAttribute("data-slot", "gesture-card-empty")
    expect(body).toHaveClass("flex-1", "justify-center")
    expect(body).toHaveAttribute("aria-description", expect.stringMatching(/press Enter/))
    expect(body.querySelector("svg")).toHaveClass("size-5")
    // An assigned card has a solid border.
    expect(screen.getByRole("group", { name: "Single press" })).not.toHaveClass("border-dashed")

    // Enter opens the same chooser.
    body.focus()
    await user.keyboard("{Enter}")
    await user.click(await screen.findByRole("option", { name: "Mail" }))
    expect(bindingsOf(latest)).toEqual([
      ["click", { type: "action", action: "shop" }],
      ["doubleClick", { type: "workflow", workflow: "deploy" }],
      ["longPress", { type: "action", action: "mail" }],
    ])
    expect(screen.getByRole("combobox", { name: "Long press" })).toHaveTextContent("Mail")
    expect(screen.getByRole("group", { name: "Long press" })).not.toHaveClass("border-dashed")
  })

  it("removes one gesture's assignment and leaves the others", async () => {
    const user = renderCards(gestureKey)
    await user.click(screen.getByRole("button", { name: "Remove Double click" }))
    expect(bindingsOf(latest)).toEqual([["click", { type: "action", action: "shop" }]])
    expect(
      screen.getByRole("combobox", { name: "Assign an action to: Double click" }),
    ).toBeInTheDocument()
  })

  it("highlights a compatible drag and not an incompatible one", () => {
    const slot = gestureSlotId(KEY, "longPress")
    const card = () => screen.getByRole("group", { name: "Long press" })
    const incompatible = (target: BindingTarget) =>
      target.type === "workflow" ? "Workflows cannot run from a long press" : null
    dnd.over = { id: slot, data: { type: "target", target: { type: "action", action: "mail" } } }
    const view = render(
      <Harness initial={gestureKey} resolveDrop={() => null} incompatible={incompatible} />,
    )
    expect(card()).toHaveAttribute("data-drop", "accepted")
    expect(card()).toHaveClass("border-dashed", "border-workspace-selection")
    view.unmount()

    dnd.over = {
      id: slot,
      data: { type: "target", target: { type: "workflow", workflow: "deploy" } },
    }
    render(<Harness initial={gestureKey} resolveDrop={() => null} incompatible={incompatible} />)
    expect(card()).toHaveAttribute("data-drop", "rejected")
    expect(card()).not.toHaveClass("border-workspace-selection")
    expect(card()).toHaveClass("border-destructive")
    dnd.over = null
  })

  it("assigns a drop to the gesture of the card it lands on", async () => {
    renderCards(gestureKey, { resolveDrop: () => ({ type: "action", action: "mail" }) })
    const slot = gestureSlotId(KEY, "longPress")
    expect(dnd.droppables.has(slot)).toBe(true)
    const event = {
      active: { id: "library:action:mail", data: { current: {} } },
      over: { id: slot },
    } as unknown as DragEndEvent
    await act(async () => {
      for (const monitor of dnd.monitors) monitor.onDragEnd?.(event)
      await Promise.resolve()
    })
    expect(bindingsOf(latest)).toEqual([
      ["click", { type: "action", action: "shop" }],
      ["doubleClick", { type: "workflow", workflow: "deploy" }],
      ["longPress", { type: "action", action: "mail" }],
    ])
  })

  it("draws a plugin action's parameters with the shared parameter fields", async () => {
    const user = renderCards(gestureKey)
    const click = screen.getByRole("group", { name: "Single press" })
    await user.click(
      within(click).getByRole("button", { name: "Override parameters for this gesture" }),
    )
    await user.click(within(click).getByRole("checkbox", { name: "Override “Level”" }))
    await user.click(within(click).getByRole("combobox", { name: "Level" }))
    await user.click(await screen.findByRole("option", { name: "warn" }))
    expect(bindingsOf(latest)[0]).toEqual([
      "click",
      { type: "action", action: "shop", overrides: { level: { type: "choice", value: "warn" } } },
    ])
  })

  it("explains invalid timings and the backend's messages", async () => {
    const user = renderCards(gestureKey, { issues: ["Key 2: something is wrong"] })
    const double = within(screen.getByRole("group", { name: "Double click" })).getByLabelText(
      "Double-click window (ms)",
    )
    await user.type(double, "90")
    expect(double).toHaveAttribute("aria-invalid", "true")
    expect(screen.getAllByRole("alert").map((a) => a.textContent)).toEqual([
      "Double-click window must be between 150 and 1000 ms.",
      "Key 2: something is wrong",
    ])
  })
})

describe("gesture carousel", () => {
  it("fits cards of at least 17rem, else whole cards between the rails", () => {
    expect(carouselGeometry(900, 3)).toEqual({ visible: 3, cardWidth: 292, overflowing: false })
    expect(carouselGeometry(700, 3)).toMatchObject({ visible: 2, overflowing: true })
    expect(carouselGeometry(300, 3)).toMatchObject({ visible: 1, overflowing: true })
    expect(carouselGeometry(300, 3).cardWidth).toBeCloseTo(220)
  })

  it("draws no arrows when every card fits", () => {
    regionWidth = 900
    renderCards(gestureKey)
    expect(shown()).toHaveLength(3)
    expect(screen.queryByRole("button", { name: "Previous gesture" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Next gesture" })).toBeNull()
  })

  it("moves one card per arrow click, disabled at the ends", async () => {
    regionWidth = 700
    const user = renderCards(gestureKey)
    const previous = screen.getByRole("button", { name: "Previous gesture" })
    const next = screen.getByRole("button", { name: "Next gesture" })
    expect(shown()).toEqual(["gesture-key:1-click-label", "gesture-key:1-doubleClick-label"])
    expect(previous).toBeDisabled()
    expect(next).toBeEnabled()
    // Out of view a card takes no drops.
    expect(dnd.droppables.has(gestureSlotId(KEY, "longPress"))).toBe(false)

    await user.click(next)
    expect(shown()).toEqual(["gesture-key:1-doubleClick-label", "gesture-key:1-longPress-label"])
    expect(next).toBeDisabled()
    expect(previous).toBeEnabled()
    await user.click(previous)
    expect(shown()[0]).toBe("gesture-key:1-click-label")
  })

  it("follows Shift+wheel and a horizontal swipe, not the plain wheel", () => {
    regionWidth = 400
    renderCards(gestureKey)
    const now = vi.spyOn(performance, "now")
    now.mockReturnValue(1000)
    fireEvent.wheel(region(), { deltaY: 80 })
    expect(shown()[0]).toBe("gesture-key:1-click-label")
    fireEvent.wheel(region(), { deltaY: 80, shiftKey: true })
    expect(shown()[0]).toBe("gesture-key:1-doubleClick-label")
    // The rest of the same swipe does not move it further.
    fireEvent.wheel(region(), { deltaX: 80 })
    expect(shown()[0]).toBe("gesture-key:1-doubleClick-label")
    now.mockReturnValue(2000)
    fireEvent.wheel(region(), { deltaX: 80 })
    expect(shown()[0]).toBe("gesture-key:1-longPress-label")
    now.mockReturnValue(3000)
    fireEvent.wheel(region(), { deltaX: -80 })
    expect(shown()[0]).toBe("gesture-key:1-doubleClick-label")
    now.mockRestore()
  })

  it("keeps every card mounted and as tall as the row, with full-height rails", async () => {
    regionWidth = 400
    const user = renderCards(gestureKey)
    const items = () => [...document.querySelectorAll<HTMLElement>("[data-carousel-item]")]
    // One card in view, all three mounted; none collapsed or hidden, so the
    // row keeps the tallest card's height whichever is shown.
    expect(shown()).toHaveLength(1)
    expect(items()).toHaveLength(3)
    const track = document.querySelector('[data-slot="gesture-cards-track"]')
    expect(track).toHaveClass("items-stretch")
    for (const item of items()) {
      expect(item.className).not.toMatch(/\bh-0\b|hidden|invisible/)
    }
    const rails = ["Previous gesture", "Next gesture"].map((name) =>
      screen.getByRole("button", { name }),
    )
    for (const rail of rails) expect(rail).toHaveClass("self-stretch", "w-9", "h-auto")
    // Form state of a card out of view survives moving the cards.
    await user.type(screen.getByRole("textbox", { name: "Label for Double click" }), "Ship")
    await user.click(rails[1] as HTMLElement)
    await user.click(rails[0] as HTMLElement)
    expect(screen.getByRole("textbox", { name: "Label for Double click" })).toHaveValue("Ship")
  })

  it("brings a card into view when focus enters it", () => {
    regionWidth = 400
    renderCards(gestureKey)
    expect(shown()).toEqual(["gesture-key:1-click-label"])
    act(() => {
      screen.getByRole("combobox", { name: "Assign an action to: Long press" }).focus()
    })
    expect(shown()).toEqual(["gesture-key:1-longPress-label"])
  })

  it("advances while a drag rests at an edge", () => {
    regionWidth = 400
    vi.useFakeTimers()
    renderCards(gestureKey, { resolveDrop: () => null })
    const box = { left: 0, right: 400, top: 0, bottom: 200, width: 400, height: 200 }
    region().getBoundingClientRect = () => box as DOMRect
    const viewport = region().querySelector<HTMLElement>('[data-slot="gesture-cards-viewport"]')
    if (viewport)
      viewport.getBoundingClientRect = () => ({ ...box, left: 36, right: 364 }) as DOMRect
    const move = (x: number) => {
      const event = {
        activatorEvent: new MouseEvent("pointerdown", { clientX: 200, clientY: 100 }),
        delta: { x: x - 200, y: 0 },
      } as unknown as DragMoveEvent
      for (const monitor of dnd.monitors) monitor.onDragMove?.(event)
    }
    act(() => {
      move(200)
      vi.advanceTimersByTime(DRAG_EDGE_DELAY_MS * 2)
    })
    expect(shown()).toEqual(["gesture-key:1-click-label"])
    act(() => {
      move(380)
      vi.advanceTimersByTime(DRAG_EDGE_DELAY_MS)
    })
    expect(shown()).toEqual(["gesture-key:1-doubleClick-label"])
    act(() => {
      vi.advanceTimersByTime(DRAG_EDGE_DELAY_MS)
    })
    expect(shown()).toEqual(["gesture-key:1-longPress-label"])
  })

  it("speaks Polish and keeps long names within the card", async () => {
    regionWidth = 700
    await i18n.changeLanguage("pl")
    try {
      renderCards(layoutWith([]))
      expect(screen.getByRole("group", { name: "Gesty" })).toBeInTheDocument()
      expect(screen.getByRole("group", { name: "Zwolnienie" })).toBeInTheDocument()
      expect(screen.getByRole("button", { name: "Pomoc: Zwolnienie" })).toBeInTheDocument()
      expect(screen.queryByText(/natychmiastowe/)).toBeNull()
      expect(screen.getByRole("button", { name: "Poprzedni gest" })).toBeDisabled()
      expect(screen.getByRole("button", { name: "Następny gest" })).toBeEnabled()
      const title = screen.getByText("Pojedyncze kliknięcie")
      expect(title).toHaveClass("break-words", "min-w-0")
      // Empty gestures invite a drop; the raw release (no drop target) a choice.
      expect(screen.getAllByText("Wybierz akcję")).toHaveLength(4)
      expect(
        screen.getByRole("combobox", { name: "Przypisz akcję do: Dwukrotne kliknięcie" }),
      ).toBeInTheDocument()
    } finally {
      await i18n.changeLanguage("en")
    }
  })
})
