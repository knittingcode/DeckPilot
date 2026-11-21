// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { LayoutEditor } from "./editor"
import { i18n } from "@/i18n"
import type { DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"
import { currentPage, goToPage, pageCommand, upTo } from "@/test/pages"

// Radix menus and selects rely on browser APIs that jsdom does not implement.
Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const saveLayout = vi.fn()
/** What would run something: never called by editing. */
const executes = { simulatePress: vi.fn(), runAction: vi.fn(), runWorkflow: vi.fn() }

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => saveLayout(...args) as unknown,
    simulatePress: (...args: unknown[]) => executes.simulatePress(...args) as unknown,
    runAction: (...args: unknown[]) => executes.runAction(...args) as unknown,
    runWorkflow: (...args: unknown[]) => executes.runWorkflow(...args) as unknown,
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => [],
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh: vi.fn(),
  }),
}))
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const plus: DeviceModel = {
  id: "elgato.streamdeck.plus",
  vendor: "Elgato",
  name: "Stream Deck +",
  usb: null,
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: true },
    touch: { zones: 4, image: null, gestures: ["tap"] },
  },
  support: "unverified",
  verification: [],
}

const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: plus.id,
  homePage: "home",
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "shop" },
        },
        {
          control: { kind: "key", index: 3 },
          trigger: { type: "press" },
          target: { type: "openPage", page: "tools" },
        },
      ],
    },
    { id: "tools", name: "Tools", bindings: [] },
  ],
}

const view = {
  activeProjectId: null,
  profile: {
    id: "p",
    name: "Work",
    description: "",
    actions: [
      { id: "shop", name: "Shop", icon: null },
      { id: "mail", name: "Mail", icon: null },
    ],
    workflows: [],
    projects: [],
    layouts: [layout],
  },
} as unknown as ProfileView

function renderEditor(saved: DeviceLayout = layout) {
  const user = userEvent.setup()
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={plus}
      saved={saved}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const control = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => control(name).getAttribute("title")

beforeEach(() => {
  saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
})
afterEach(cleanup)

// These tests are long scripts of real interactions with the whole editor,
// and nearly all of their time is React rendering it: a click is 15-30 ms
// of commits (every control is a dnd-kit target in its own context menu),
// with no timers, waits or pending requests in between. They take up to
// 0.7 s on an idle computer, 1.9 s with one busy process per core, 2.7 s
// with two and 4.3 s with one per core beside 28 test workers, so on a
// busy computer they ran past the 5 s default.
describe("LayoutEditor", { timeout: 20_000 }, () => {
  it("draws every control of the model and describes bound ones", () => {
    renderEditor()
    expect(screen.getAllByRole("button", { name: /^Key \d+$/ })).toHaveLength(8)
    expect(screen.getAllByRole("button", { name: /^Dial \d+$/ })).toHaveLength(4)
    expect(screen.getAllByRole("button", { name: /^Touch zone \d+$/ })).toHaveLength(4)
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    expect(describes("Key 2")).toBe("Key 2")
    // "Tools" is a folder (a key of Home opens it): one top-level page.
    expect(screen.getByText("Page 1 of 1")).toBeInTheDocument()
    expect(currentPage()).toBe("Home")
  })

  it("shows only settings in the inspector: none without a selection, no instructions", async () => {
    const user = renderEditor()
    // Nothing selected: the inspector is there, empty.
    expect(screen.getByRole("region", { name: "Inspector" }).textContent).toBe("")
    // An empty key: its slots, no instruction.
    await user.click(control("Key 2"))
    expect(screen.queryByText(/Drag an action from the Library/)).not.toBeInTheDocument()
    expect(screen.getByRole("region", { name: "Key 2 settings" })).toBeInTheDocument()
    await user.click(control("Key 1"))
    expect(screen.queryByText(/Only events this model can produce/)).not.toBeInTheDocument()
  })

  it("draws the device, its pages and the inspector on one dark workspace", () => {
    renderEditor()
    const main = document.querySelector("[data-slot=device-editor-main]") as HTMLElement
    // The dark token scope (index.css), whatever the app's theme.
    expect(main).toHaveClass("dark", "workspace-scope", "relative", "flex-col", "overflow-hidden")
    for (const part of ["device-canvas", "page-nav", "inspector", "editing-bar"])
      expect(main.querySelector(`[data-slot=${part}]`)).toBeInTheDocument()
    // The canvas never scrolls; the inspector region is the one that does.
    const canvas = main.querySelector("[data-slot=device-viewport]") as HTMLElement
    expect(canvas).toHaveClass("overflow-hidden")
    expect(canvas.querySelector("[data-slot=device-canvas]")).toBeInTheDocument()
    const inspector = main.querySelector("[data-slot=inspector-scroll]") as HTMLElement
    expect(inspector).toHaveClass("relative", "overflow-y-auto")
    expect(inspector.querySelector("[data-slot=inspector]")).toBeInTheDocument()
    expect(canvas.contains(inspector)).toBe(false)
    // No cards inside it.
    expect(main.querySelector("[data-slot=card]")).toBeNull()
  })

  it("frosts the top of the inspector only once it is scrolled, without taking pointer events", () => {
    renderEditor()
    const curtain = document.querySelector('[data-slot="inspector-curtain"]')
    const scroll = document.querySelector<HTMLElement>('[data-slot="inspector-scroll"]')
    const region = document.querySelector('[data-slot="inspector-region"]')
    if (!curtain || !scroll || !region) throw new Error("no inspector curtain")
    // Decorative: hidden from assistive technology, outside the scrolling
    // content (it never scrolls, takes no space), styled by index.css
    // (pointer-events: none, z-index under the dock and the deselect button).
    expect(curtain).toHaveAttribute("aria-hidden", "true")
    expect(curtain).toHaveClass("inspector-curtain")
    expect(scroll.contains(curtain)).toBe(false)
    expect(curtain.parentElement).toBe(region)
    // Hidden at the top, shown once scrolled past 2px, hidden again at the top.
    expect(region).not.toHaveAttribute("data-scrolled")
    for (const [top, shown] of [
      [1, false],
      [40, true],
      [0, false],
    ] as const) {
      scroll.scrollTop = top
      fireEvent.scroll(scroll)
      if (shown) expect(region).toHaveAttribute("data-scrolled")
      else expect(region).not.toHaveAttribute("data-scrolled")
    }
  })

  it("selects with click, Cmd/Ctrl-click and Shift-click", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    expect(control("Key 1")).toHaveAttribute("aria-pressed", "true")
    expect(screen.getByRole("region", { name: "Key 1 settings" })).toBeInTheDocument()

    await user.keyboard("{Control>}")
    await user.click(control("Key 3"))
    await user.keyboard("{/Control}")
    expect(control("Key 1")).toHaveAttribute("aria-pressed", "true")
    expect(control("Key 3")).toHaveAttribute("aria-pressed", "true")
    expect(control("Key 2")).toHaveAttribute("aria-pressed", "false")
    expect(screen.getByRole("region", { name: "Settings of 2 controls" })).toBeInTheDocument()

    await user.click(control("Key 2"))
    await user.keyboard("{Shift>}")
    await user.click(control("Key 5"))
    await user.keyboard("{/Shift}")
    expect(screen.getByRole("region", { name: "Settings of 4 controls" })).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Deselect controls" }))
    // The canvas draws its keys again after the selection clears; on a slow
    // machine that is still under way here.
    await vi.waitFor(() => {
      expect(control("Key 2")).toHaveAttribute("aria-pressed", "false")
    })
  })

  it("heads the inspector with no title, only a deselect button that keeps the draft", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    const inspector = screen.getByRole("region", { name: "Key 1 settings" })
    expect(inspector).toHaveAttribute("data-inspector-target", "key:0")
    // No visible heading names the control: the canvas shows the selection.
    expect(within(inspector).queryByRole("heading", { name: "Key 1" })).not.toBeInTheDocument()
    expect(within(inspector).queryByText("Key 1")).not.toBeInTheDocument()
    expect(document.querySelector("[data-slot=inspector-title]")).toBeNull()

    // A dirty draft stays dirty: deselecting neither saves nor discards.
    await user.keyboard("{Delete}")
    expect(screen.getByText("Unsaved")).toBeInTheDocument()
    await user.click(control("Key 2"))
    const deselect = screen.getByRole("button", { name: "Deselect key" })
    expect(deselect.closest("[data-slot=inspector-deselect]")).not.toBeNull()
    // Its tooltip (Radix positions it with the ResizeObserver of the test
    // setup, src/test/setup.ts).
    await user.hover(deselect)
    expect(await screen.findByRole("tooltip", { name: "Deselect key" })).toBeInTheDocument()
    await user.click(deselect)
    expect(control("Key 2")).toHaveAttribute("aria-pressed", "false")
    expect(screen.getByRole("region", { name: "Inspector" }).textContent).toBe("")
    expect(screen.queryByRole("button", { name: /^Deselect/ })).not.toBeInTheDocument()
    expect(screen.getByText("Unsaved")).toBeInTheDocument()
    expect(describes("Key 1")).toBe("Key 1")
    expect(describes("Key 4")).toBe("Key 4: Press: Open “Tools”")
    expect(saveLayout).not.toHaveBeenCalled()
    for (const run of Object.values(executes)) expect(run).not.toHaveBeenCalled()

    await user.click(control("Dial 1"))
    expect(screen.getByRole("button", { name: "Deselect dial" })).toBeInTheDocument()
    await user.click(control("Touch zone 1"))
    expect(screen.getByRole("button", { name: "Deselect touch zone" })).toBeInTheDocument()
  })

  it("names the inspector and its deselect button in Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    try {
      const user = renderEditor()
      // The canvas is drawn in Polish once the language change rendered.
      await user.click(await screen.findByRole("button", { name: "Klawisz 1" }))
      const inspector = screen.getByRole("region", { name: "Klawisz 1 – ustawienia" })
      expect(within(inspector).queryByText("Klawisz 1")).not.toBeInTheDocument()
      await user.click(screen.getByRole("button", { name: "Odznacz klawisz" }))
      expect(control("Klawisz 1")).toHaveAttribute("aria-pressed", "false")
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    }
  })

  it("navigates with arrow keys and clears with Delete, undo and redo", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{ArrowRight}")
    expect(control("Key 2")).toHaveFocus()
    expect(control("Key 2")).toHaveAttribute("aria-pressed", "true")
    await user.keyboard("{ArrowLeft}{Delete}")
    expect(describes("Key 1")).toBe("Key 1")
    expect(screen.getByText("Unsaved")).toBeInTheDocument()

    await user.keyboard("{Control>}z{/Control}")
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    await user.keyboard("{Control>}{Shift>}z{/Shift}{/Control}")
    expect(describes("Key 1")).toBe("Key 1")
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled()
    await user.keyboard("{Control>}y{/Control}")
    expect(describes("Key 1")).toBe("Key 1")
  })

  it("copies, pastes and duplicates with the keyboard, across pages", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{Control>}c{/Control}")
    await user.click(control("Key 6"))
    await user.keyboard("{Control>}v{/Control}")
    expect(describes("Key 6")).toBe("Key 6: Press: Shop")

    await user.keyboard("{Control>}d{/Control}")
    expect(describes("Key 7")).toBe("Key 7: Press: Shop")

    await goToPage(user, "Tools")
    await user.click(control("Key 2"))
    await user.keyboard("{Control>}v{/Control}")
    expect(describes("Key 2")).toBe("Key 2: Press: Shop")

    await goToPage(user, "Home")
    await user.click(control("Key 1"))
    await user.keyboard("{Control>}x{/Control}")
    expect(describes("Key 1")).toBe("Key 1")
    await user.click(control("Key 8"))
    await user.keyboard("{Control>}v{/Control}")
    expect(describes("Key 8")).toBe("Key 8: Press: Shop")
  })

  it("offers assign, clear and folder navigation in the context menu", async () => {
    const user = renderEditor()
    fireEvent.contextMenu(control("Key 2"))
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: /Remove assignment/ })).toHaveAttribute(
      "data-disabled",
    )
    await user.click(within(menu).getByRole("menuitem", { name: /Assign/ }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Mail" }))
    expect(describes("Key 2")).toBe("Key 2: Press: Mail")
    expect(screen.queryByRole("menu")).not.toBeInTheDocument()

    fireEvent.contextMenu(control("Key 2"))
    await user.click(await screen.findByRole("menuitem", { name: /Remove assignment/ }))
    expect(describes("Key 2")).toBe("Key 2")

    fireEvent.contextMenu(control("Key 4"))
    await user.click(await screen.findByRole("menuitem", { name: /Open “Tools”/ }))
    expect(currentPage()).toBe("Tools")
    // A folder is where its top-level page is.
    expect(screen.getByText("Page 1 of 1")).toBeInTheDocument()
    expect(screen.getByText("This folder is empty")).toBeInTheDocument()
    expect(screen.getByRole("note")).toHaveTextContent("no “Back” binding")

    await upTo(user, "Home")
    expect(currentPage()).toBe("Home")
  })

  it("follows the stored page tree for Back, the location and deleting", async () => {
    const tree: DeviceLayout = {
      ...layout,
      explicitTree: true,
      pages: [
        ...layout.pages.map((p) => (p.id === "tools" ? { ...p, parent: "home" } : p)),
        { id: "git", name: "Git", parent: "tools", bindings: [] },
      ],
    }
    const user = renderEditor(tree)
    await goToPage(user, "Git")
    const location = screen.getByRole("navigation", { name: "Breadcrumb" })
    expect(
      within(location)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["Home", "Tools", "Git"])
    expect(screen.getByRole("note")).toHaveTextContent("adds one on its first free key")

    // Opened from the page menu: Back goes to its folder, not straight home.
    await upTo(user, "Tools")
    expect(currentPage()).toBe("Tools")

    // Deleting "Tools" asks first, saying what goes with it, and keeps
    // "Git" in a folder that exists.
    await pageCommand(user, "Tools", /Delete page/)
    const dialog = await screen.findByRole("alertdialog", { name: "Delete “Tools”?" })
    expect(dialog).toHaveTextContent(
      "1 control on other pages that opens it will do nothing afterwards.",
    )
    expect(dialog).toHaveTextContent("The page inside it (“Git”) is kept and moves to “Home”.")
    await user.click(within(dialog).getByRole("button", { name: "Delete page" }))
    await user.click(screen.getByRole("button", { name: /Save/ }))
    const [, saved] = saveLayout.mock.calls[0] as [string, DeviceLayout]
    expect(saved.pages.map((p) => [p.id, p.parent])).toEqual([
      ["home", undefined],
      ["git", "home"],
    ])
  })

  it("leaves undo, copy and paste in text fields to the field", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{Delete}")
    await user.click(control("Key 2"))
    await user.keyboard("{Control>}c{/Control}")

    const name = screen.getByLabelText("Search the library")
    await user.click(name)
    for (const key of ["z", "y", "c", "x", "v", "d"]) {
      const event = new KeyboardEvent("keydown", { key, ctrlKey: true, bubbles: true })
      name.dispatchEvent(event)
      // Not taken: the native Edit menu (macOS) or the field handles it.
      expect(event.defaultPrevented).toBe(false)
    }
    expect(describes("Key 1")).toBe("Key 1")
    expect(screen.getByRole("button", { name: "Undo" })).toBeEnabled()
  })

  it("leaves copying selected page text to the browser", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    const range = document.createRange()
    range.selectNodeContents(screen.getByText("Page 1 of 1"))
    window.getSelection()?.removeAllRanges()
    window.getSelection()?.addRange(range)
    const event = new KeyboardEvent("keydown", { key: "c", ctrlKey: true, bubbles: true })
    control("Key 1").dispatchEvent(event)
    expect(event.defaultPrevented).toBe(false)
    window.getSelection()?.removeAllRanges()

    // Nothing was copied: pasting onto another key does not change it.
    await user.click(control("Key 6"))
    await user.keyboard("{Control>}v{/Control}")
    expect(describes("Key 6")).toBe("Key 6")
  })

  it("does not undo behind an open menu", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{Delete}")
    fireEvent.contextMenu(control("Key 2"))
    await screen.findByRole("menu")
    await user.keyboard("{Control>}z{/Control}")
    const key = screen.getByRole("button", { name: "Key 1", hidden: true })
    expect(key).toHaveAttribute("title", "Key 1")
  })

  it("saves with Cmd/Ctrl+S, also from a text field", async () => {
    const user = renderEditor()
    await user.keyboard("{Control>}s{/Control}")
    expect(saveLayout).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Add page" }))
    await user.type(screen.getByLabelText("Search the library"), "x")
    await user.keyboard("{Control>}s{/Control}")
    const [, saved] = saveLayout.mock.calls[0] as [string, DeviceLayout]
    // The unsaved change (a new page) was saved, though the focus was in a field.
    expect(saved.pages.map((p) => p.name)).toContainEqual(expect.stringMatching(/^Page \d+$/))
  })

  it("saves the edited layout", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{Delete}")
    await user.click(screen.getByRole("button", { name: /Save/ }))
    const [, saved] = saveLayout.mock.calls[0] as [string, DeviceLayout]
    expect(saved.pages[0]?.bindings).toHaveLength(1)
    await vi.waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Layout saved")
    })
  })

  it("says when the agent did not apply the saved layout", async () => {
    saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
      Promise.resolve({ ...saved, agentReload: { state: "timedOut" } }),
    )
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{Delete}")
    await user.click(screen.getByRole("button", { name: /Save/ }))
    await vi.waitFor(() => {
      expect(toast.warning).toHaveBeenCalledWith("Layout saved", {
        description: expect.stringContaining("did not confirm") as unknown,
      })
    })
    expect(toast.success).not.toHaveBeenCalled()
  })
})
