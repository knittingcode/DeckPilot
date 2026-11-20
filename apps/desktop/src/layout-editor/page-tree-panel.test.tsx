// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import type { DragEndEvent, DragMoveEvent } from "@dnd-kit/core"
import userEvent from "@testing-library/user-event"
import type { ComponentProps } from "react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { DeviceLayout, DeviceModel, PageTree, ProfileView } from "@/lib/types"
import { i18n } from "@/i18n"
import { installDomStubs } from "@/test/dom"
import {
  closePages,
  crumbNames,
  currentPage,
  managePages,
  pageCommand,
  pageNames,
  treeRows,
} from "@/test/pages"

import { LayoutEditor } from "./editor"

const api = {
  previewLayout: vi.fn(),
  listIcons: vi.fn(),
  saveLayout: vi.fn(),
  pageTree: vi.fn(),
  reorderPages: vi.fn(),
  movePage: vi.fn(),
  copyPage: vi.fn(),
  listActionTypes: vi.fn(),
  getGestureDefaults: vi.fn(),
}
const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), warning: vi.fn() }))

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => [],
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh: vi.fn() }),
}))
vi.mock("sonner", () => ({ toast }))

/**
 * The Page Navigator's drag and drop as it is wired: its DndContext's
 * handlers and which rows can be dragged. jsdom has no layout, so a drop
 * is made by calling the handlers with what dnd-kit would pass.
 */
interface TreeDndHandlers {
  onDragMove?: (event: DragMoveEvent) => void
  onDragEnd?: (event: DragEndEvent) => void
}
const dnd = vi.hoisted((): { handlers: TreeDndHandlers; disabled: Map<string, boolean> } => ({
  handlers: {},
  /** Row id → whether it cannot be dragged. */
  disabled: new Map(),
}))
vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  return {
    ...actual,
    DndContext: (props: ComponentProps<typeof actual.DndContext>) => {
      if (props.id === "page-tree") dnd.handlers = props
      return <actual.DndContext {...props} />
    },
    useDraggable: (args: Parameters<typeof actual.useDraggable>[0]) => {
      dnd.disabled.set(String(args.id), args.disabled === true)
      return actual.useDraggable(args)
    },
  }
})

const POSITION_Y = { before: 2, inside: 16, after: 30 }
const dragEvent = (page: string, target: string, position: keyof typeof POSITION_Y) =>
  ({
    active: { id: page },
    over: { id: target, rect: { top: 0, height: 32 } },
    activatorEvent: { clientY: POSITION_Y[position] },
    delta: { x: 0, y: 0 },
  }) as unknown as DragEndEvent

/** Drops `page` before, after or inside `target`'s row. */
function dropOn(page: string, target: string, position: keyof typeof POSITION_Y) {
  act(() => {
    dnd.handlers.onDragEnd?.(dragEvent(page, target, position))
  })
}
/** Drags `page` over `target`'s row (not dropped). */
function move(page: string, target: string, position: keyof typeof POSITION_Y) {
  act(() => {
    dnd.handlers.onDragMove?.(dragEvent(page, target, position))
  })
}

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

const main: DeviceLayout = {
  id: "main",
  name: "Main",
  model: plus.id,
  homePage: "home",
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        {
          control: { kind: "key", index: 3 },
          trigger: { type: "press" },
          target: { type: "openPage", page: "tools" },
        },
      ],
    },
    { id: "tools", name: "Tools", bindings: [] },
    { id: "git", name: "Git", parent: "tools", bindings: [] },
    { id: "notes", name: "Notes", parent: "home", bindings: [] },
  ],
}
const travel: DeviceLayout = {
  id: "travel",
  name: "Travel",
  model: plus.id,
  homePage: "t-home",
  pages: [{ id: "t-home", name: "Home", bindings: [] }],
}

/** Stored parent, else the page with a key opening it; home at the top. */
function fakeTree(layout: DeviceLayout): PageTree {
  return {
    pages: layout.pages.map((p) => ({
      id: p.id,
      name: p.name,
      parent:
        p.id === layout.homePage
          ? null
          : (p.parent ??
            layout.pages.find((o) =>
              o.bindings.some((b) => b.target.type === "openPage" && b.target.page === p.id),
            )?.id ??
            null),
      stored: Boolean(p.parent),
      depth: 0,
      automaticBack: null,
    })),
    warnings: layout.pages.some((p) => p.id === "git") ? ["page `Git` has no way back"] : [],
  }
}

function renderEditor(layout: DeviceLayout = main) {
  const view = {
    activeProjectId: null,
    profile: {
      id: "p",
      name: "Work",
      actions: [],
      workflows: [],
      projects: [],
      layouts: [layout, travel],
    },
  } as unknown as ProfileView
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={plus}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return userEvent.setup()
}

/** The rows of the open Page Navigator, by name. */
const row = (name: string) =>
  within(screen.getByRole("tree", { name: "Page tree" })).getByRole("treeitem", { name })
/** The "⋯" menu of a row, opened; returns the menu. */
async function rowMenu(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(screen.getByRole("button", { name: `Actions for “${name}”` }))
  return screen.findByRole("menu")
}

beforeAll(installDomStubs)
beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset()
  for (const fn of Object.values(toast)) fn.mockReset()
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.listActionTypes.mockResolvedValue([])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.pageTree.mockImplementation((layout: DeviceLayout) => Promise.resolve(fakeTree(layout)))
})
afterEach(async () => {
  cleanup()
  await act(async () => {
    await i18n.changeLanguage("en")
  })
})

const counter = () => document.querySelector("[data-slot=page-counter]") as HTMLElement
const dock = () => document.querySelector("[data-slot=page-navigation-dock]") as HTMLElement
const bar = () => document.querySelector("[data-slot=page-nav]") as HTMLElement

/** The compact toolbar's classes, as they were before the dock. */
const COLLAPSED_BAR = "flex max-w-full min-w-0 items-center gap-1 rounded-lg border bg-card p-0.5"

describe("the Page Navigator (the dock the page counter unfolds)", () => {
  it("leaves the collapsed toolbar as it was", () => {
    renderEditor()
    expect(bar().className).toBe(COLLAPSED_BAR)
    expect(dock().getAttribute("style")).toBeNull()
    expect(document.querySelector("[data-slot=page-dock-outline]")).toBeNull()
  })

  for (const language of ["en", "pl"]) {
    it(`keeps the toolbar as it is for many pages, deep folders and long names (${language})`, async () => {
      await act(async () => {
        await i18n.changeLanguage(language)
      })
      const long = "A very long folder name that the breadcrumbs have to cut somewhere"
      const chain = ["d1", "d2", "d3", "d4", "d5"].map((id, index, ids) => ({
        id,
        name: index === 4 ? long : `Level ${index + 1}`,
        parent: index === 0 ? "home" : ids[index - 1],
        bindings: [],
      }))
      const extra = Array.from({ length: 10 }, (_, index) => ({
        id: `top${index}`,
        name: `Top ${index}`,
        bindings: [],
      }))
      const user = renderEditor({ ...main, pages: [...main.pages, ...chain, ...extra] })
      const before = bar().className
      expect(before).toBe(COLLAPSED_BAR)
      const counterText = counter().textContent
      await user.click(counter())
      const navigator = await screen.findByRole("region", {
        name: i18n.t("editor:pageSettings.sheetTitle"),
      })
      expect(bar().className).toBe(before)
      expect(counter().textContent).toBe(counterText)
      expect(counterText).toMatch(/\/\s*1\d/)
      // The deepest page is at its level, its name cut with a tooltip.
      const deepest = within(navigator).getByRole("treeitem", { name: long })
      expect(deepest).toHaveAttribute("aria-level", "6")
      expect(deepest.querySelector("[data-slot=page-tree-name]")).toHaveAttribute("title", long)
    })
  }

  it("draws one outline behind the toolbar and the panel once measured", async () => {
    const sizes = new Map([
      ["page-nav", { width: 180, height: 30 }],
      ["page-navigator-panel", { width: 180, height: 140 }],
    ])
    const size = (element: HTMLElement, key: "width" | "height") =>
      sizes.get(element.dataset.slot ?? "")?.[key] ?? 0
    const width = vi
      .spyOn(HTMLElement.prototype, "offsetWidth", "get")
      .mockImplementation(function (this: HTMLElement) {
        return size(this, "width")
      })
    const height = vi
      .spyOn(HTMLElement.prototype, "offsetHeight", "get")
      .mockImplementation(function (this: HTMLElement) {
        return size(this, "height")
      })
    try {
      const user = renderEditor()
      await managePages(user)
      const outline = await waitFor(() => {
        const svg = document.querySelector("[data-slot=page-dock-outline]")
        expect(svg).not.toBeNull()
        return svg as SVGElement
      })
      expect(outline).toHaveAttribute("aria-hidden")
      expect(outline).toHaveAttribute("height", "170")
      expect(outline.querySelector("path")).toHaveClass("fill-card", "stroke-border")
      // The toolbar keeps its size and place; the outline is its border now.
      expect(bar()).toHaveClass("rounded-lg", "border", "p-0.5", "border-transparent")
      expect(bar()).not.toHaveClass("w-full")
      expect(dock().getAttribute("style")).toBeNull()
    } finally {
      width.mockRestore()
      height.mockRestore()
    }
  })

  it("unfolds under the bar, in the dock, and folds again", async () => {
    const user = renderEditor()
    expect(screen.queryByRole("button", { name: "Manage pages" })).not.toBeInTheDocument()
    // Collapsed: the bar alone.
    expect(dock()).toHaveAttribute("data-state", "collapsed")
    expect(counter()).toHaveAttribute("aria-expanded", "false")
    expect(screen.queryByRole("region", { name: "Pages" })).not.toBeInTheDocument()
    const navigator = await managePages(user)
    // Expanded: the panel in the dock, right under the bar; no popover.
    expect(dock()).toHaveAttribute("data-state", "expanded")
    expect(counter()).toHaveAttribute("aria-expanded", "true")
    expect(counter()).toHaveAttribute("aria-controls", navigator.id)
    expect(navigator.parentElement).toBe(dock())
    expect(navigator.previousElementSibling).toBe(bar())
    expect(document.querySelector("[data-slot=popover-content]")).toBeNull()
    expect(document.querySelector("[data-radix-popper-content-wrapper]")).toBeNull()
    // The toolbar is not changed (jsdom measures nothing: no outline is
    // drawn), the panel hangs under it without a top edge of its own.
    expect(bar().className).toBe(COLLAPSED_BAR)
    expect(navigator).toHaveClass("top-full", "border-t-0")
    expect(navigator.className).not.toMatch(/shadow|rounded-t|rounded-lg(\s|$)/)
    expect(dock().className).not.toMatch(/shadow/)
    // Focus goes into the tree, to the page shown.
    expect(row("Home")).toHaveFocus()
    expect(navigator.querySelector("[data-slot=page-navigator-scroll]")).toHaveClass(
      "relative",
      "overflow-y-auto",
    )
    // Escape folds it and gives focus back to the counter.
    await closePages(user)
    expect(screen.queryByRole("region", { name: "Pages" })).not.toBeInTheDocument()
    expect(dock()).toHaveAttribute("data-state", "collapsed")
    await waitFor(() => {
      expect(counter()).toHaveFocus()
    })
    // The counter toggles it.
    await user.click(counter())
    expect(screen.getByRole("region", { name: "Pages" })).toBeInTheDocument()
    await user.click(counter())
    expect(screen.queryByRole("region", { name: "Pages" })).not.toBeInTheDocument()
  })

  it("folds on a click outside, not in a row's menu, a folder's chevron or a dialog", async () => {
    const user = renderEditor()
    await managePages(user)
    // A row's menu (a portal) keeps it open, and so does choosing in it.
    const menu = await rowMenu(user, "Tools")
    await user.click(within(menu).getByRole("menuitemcheckbox"))
    expect(screen.getByRole("region", { name: "Pages" })).toBeInTheDocument()
    // Opening and closing a folder too.
    await user.click(row("Tools").querySelector("[data-slot=tree-chevron]") as HTMLElement)
    expect(row("Tools")).toHaveAttribute("aria-expanded", "false")
    expect(screen.getByRole("region", { name: "Pages" })).toBeInTheDocument()
    // Outside: folded, nothing else happens.
    await user.click(screen.getByRole("group", { name: "Device canvas" }))
    expect(screen.queryByRole("region", { name: "Pages" })).not.toBeInTheDocument()
    expect(currentPage()).toBe("Home")
  })

  it("shows the layout's hierarchy as a tree and opens pages from it", async () => {
    const user = renderEditor()
    const navigator = await managePages(user)
    // Git is inside Tools, which is inside Home: levels, not a flat list.
    expect(
      treeRows(navigator).map((r) => [r.getAttribute("aria-label"), r.getAttribute("aria-level")]),
    ).toEqual([
      ["Home", "1"],
      ["Tools", "2"],
      ["Git", "3"],
      ["Notes", "2"],
    ])
    expect(row("Home")).toHaveAttribute("aria-current", "page")
    expect(row("Home")).toHaveAttribute("aria-selected", "true")
    expect(row("Tools")).toHaveAttribute("aria-expanded", "true")
    await user.click(row("Git"))
    expect(currentPage()).toBe("Git")
    // The breadcrumbs follow, and the counter.
    expect(crumbNames()).toEqual(["Home", "Tools", "Git"])
    await managePages(user)
    expect(row("Git")).toHaveAttribute("aria-current", "page")
    expect(row("Home")).not.toHaveAttribute("aria-current")
  })

  it("expands and collapses folders apart from navigating", async () => {
    const user = renderEditor()
    await managePages(user)
    const chevron = row("Tools").querySelector("[data-slot=tree-chevron]") as HTMLElement
    await user.click(chevron)
    expect(row("Tools")).toHaveAttribute("aria-expanded", "false")
    expect(screen.queryByRole("treeitem", { name: "Git" })).not.toBeInTheDocument()
    // Still on Home: the chevron does not navigate.
    expect(currentPage()).toBe("Home")
    await user.click(row("Tools").querySelector("[data-slot=tree-chevron]") as HTMLElement)
    expect(row("Git")).toBeInTheDocument()
  })

  it("is a keyboard tree: arrows move, open and close, Enter shows", async () => {
    const user = renderEditor()
    await managePages(user)
    expect(row("Home")).toHaveFocus()
    await user.keyboard("{ArrowDown}")
    expect(row("Tools")).toHaveFocus()
    expect(row("Tools")).toHaveAttribute("tabindex", "0")
    expect(row("Home")).toHaveAttribute("tabindex", "-1")
    await user.keyboard("{ArrowLeft}")
    expect(row("Tools")).toHaveAttribute("aria-expanded", "false")
    await user.keyboard("{ArrowLeft}")
    expect(row("Home")).toHaveFocus()
    await user.keyboard("{ArrowDown}{ArrowRight}")
    expect(row("Tools")).toHaveAttribute("aria-expanded", "true")
    await user.keyboard("{ArrowRight}")
    expect(row("Git")).toHaveFocus()
    await user.keyboard("{Enter}")
    expect(currentPage()).toBe("Git")
    expect(screen.queryByRole("region", { name: "Pages" })).not.toBeInTheDocument()
  })

  /** The labels of a row's menu, in order (every kind of item). */
  const menuLabels = (menu: HTMLElement) =>
    [...menu.querySelectorAll('[role^="menuitem"]')].map((item) => item.textContent)

  it("offers the home page only Rename and Add page", async () => {
    const user = renderEditor()
    await managePages(user)
    const menu = await rowMenu(user, "Home")
    expect(menuLabels(menu)).toEqual(["Rename", "Add page"])
    await user.click(within(menu).getByRole("menuitem", { name: "Add page" }))
    // A new page in the root, shown.
    expect(currentPage()).toBe("Page 2")
    expect(crumbNames()).toEqual(["Home", "Page 2"])
  })

  it("offers a page Rename, Duplicate and Delete, and a folder a new page inside", async () => {
    const user = renderEditor()
    await managePages(user)
    let menu = await rowMenu(user, "Notes")
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent),
    ).toEqual(["Rename", "Duplicate", "Delete page"])
    // Its one setting, a checkbox.
    expect(within(menu).getByRole("menuitemcheckbox")).toHaveTextContent("Close after a key runs")
    // Destructive colour on Delete only; nothing disabled, no notes under the items.
    expect(within(menu).getByRole("menuitem", { name: "Delete page" })).toHaveAttribute(
      "data-variant",
      "destructive",
    )
    for (const item of menu.querySelectorAll('[role^="menuitem"]')) {
      expect(item).not.toHaveAttribute("data-disabled")
      expect(item).toHaveClass("h-8")
      if (!item.textContent.includes("Delete"))
        expect(item).toHaveAttribute("data-variant", "default")
    }
    expect(within(menu).getAllByRole("separator")).toHaveLength(1)
    expect(menu).toHaveClass("w-auto")
    expect(menu.className).not.toMatch(/(^|\s)h-\[/)
    await user.keyboard("{Escape}")

    menu = await rowMenu(user, "Tools")
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent),
    ).toEqual(["Rename", "Duplicate", "New page inside", "Delete page"])
  })

  it("has the same items in a row's context menu", async () => {
    const user = renderEditor()
    await managePages(user)
    fireEvent.contextMenu(row("Notes"))
    const menu = await screen.findByRole("menu", { name: "Actions for “Notes”" })
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent),
    ).toEqual(["Rename", "Duplicate", "Delete page"])
  })

  it("hides what needs a saved draft while there are unsaved changes", async () => {
    const user = renderEditor()
    await user.click(screen.getByRole("button", { name: "Add page" }))
    await managePages(user)
    const menu = await rowMenu(user, "Tools")
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent),
    ).toEqual(["Rename", "New page inside", "Delete page"])
    await user.keyboard("{Escape}")
    // A drop says why it did nothing.
    dropOn("notes", "tools", "before")
    expect(api.reorderPages).not.toHaveBeenCalled()
    expect(toast.error).toHaveBeenCalledWith(
      "Save or revert your changes to reorder, move or copy pages.",
    )
  })

  it("deletes a page after asking, with the deletion notes", async () => {
    const user = renderEditor()
    await pageCommand(user, "Notes", "Delete page")
    const dialog = await screen.findByRole("alertdialog", { name: "Delete “Notes”?" })
    expect(dialog).toHaveTextContent("You can undo this until the layout is saved.")
    await user.click(within(dialog).getByRole("button", { name: "Delete page" }))
    expect(await pageNames(user)).toEqual(["Home", "Tools", "Git"])
  })

  it("duplicates a page next to it and shows the copy", async () => {
    const copy = { id: "notes-2", name: "Notes", parent: "home", bindings: [] }
    api.copyPage.mockResolvedValue({
      page: "notes-2",
      removedKeys: 0,
      layouts: [{ ...main, pages: [...main.pages, copy] }],
    })
    const user = renderEditor()
    await pageCommand(user, "Notes", /Duplicate/)
    expect(api.copyPage).toHaveBeenCalledWith("p", "main", "notes", null, "home")
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("“Notes” duplicated")
    })
    expect(await pageNames(user)).toEqual(["Home", "Tools", "Git", "Notes", "Notes"])
  })

  it("reorders siblings by dragging and shows the stored order", async () => {
    const reordered = {
      ...main,
      pages: [main.pages[0], main.pages[3], main.pages[1], main.pages[2]],
    }
    api.reorderPages.mockResolvedValue(reordered)
    const user = renderEditor()
    await managePages(user)
    dropOn("notes", "tools", "before")
    expect(api.reorderPages).toHaveBeenCalledWith("p", "main", ["home", "notes", "tools", "git"])
    await waitFor(() => {
      expect(api.pageTree).toHaveBeenLastCalledWith(reordered)
    })
    expect(await screen.findByText("“Notes” moved to its new place")).toHaveClass("sr-only")
    await closePages(user)
    expect(await pageNames(user)).toEqual(["Home", "Notes", "Tools", "Git"])
    // Stored by the command: nothing left to save.
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument()
  })

  it("moves a page into a folder by dropping it on the folder", async () => {
    const after = {
      ...main,
      pages: main.pages.map((p) => (p.id === "notes" ? { ...p, parent: "tools" } : p)),
    }
    api.movePage.mockResolvedValue({ page: "notes", removedKeys: 0, layouts: [after] })
    const user = renderEditor()
    await managePages(user)
    dropOn("notes", "tools", "inside")
    expect(api.movePage).toHaveBeenCalledWith("p", "main", "notes", null, "tools")
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("“Notes” moved to “Tools”")
    })
    await waitFor(() => {
      expect(row("Notes")).toHaveAttribute("aria-level", "3")
    })
  })

  it("rejects moving the home page, and a folder into itself or a page inside it", async () => {
    const user = renderEditor()
    await managePages(user)
    for (const [page, target, position] of [
      ["home", "notes", "before"],
      ["home", "tools", "inside"],
      ["tools", "tools", "inside"],
      ["tools", "git", "inside"],
      ["tools", "git", "after"],
      ["notes", "home", "before"],
    ] as const) {
      dropOn(page, target, position)
    }
    expect(api.reorderPages).not.toHaveBeenCalled()
    expect(api.movePage).not.toHaveBeenCalled()
    // The home row cannot even be picked up.
    expect(dnd.disabled.get("home")).toBe(true)
    expect(dnd.disabled.get("notes")).toBe(false)
  })

  it("shows where a page would go while it is dragged", async () => {
    const user = renderEditor()
    await managePages(user)
    move("notes", "tools", "before")
    expect(row("Tools").querySelector("[data-slot=drop-line]")).toHaveClass("-top-px")
    move("notes", "tools", "inside")
    expect(row("Tools")).toHaveAttribute("data-drop", "inside")
    expect(row("Tools").querySelector("[data-slot=drop-line]")).toBeNull()
    // Nowhere to go: no indicator.
    move("tools", "git", "inside")
    expect(row("Git")).not.toHaveAttribute("data-drop")
  })

  it("reorders with Alt+Up and Alt+Down, and says so", async () => {
    const reordered = {
      ...main,
      pages: [main.pages[0], main.pages[3], main.pages[2], main.pages[1]],
    }
    api.reorderPages.mockResolvedValue(reordered)
    const user = renderEditor()
    await managePages(user)
    // Home stays where it is.
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}")
    expect(api.reorderPages).not.toHaveBeenCalled()
    await user.keyboard("{End}")
    expect(row("Notes")).toHaveFocus()
    await user.keyboard("{Alt>}{ArrowUp}{/Alt}")
    expect(api.reorderPages).toHaveBeenCalledWith("p", "main", ["home", "notes", "git", "tools"])
    expect(
      await within(screen.getByRole("region", { name: "Pages" })).findByText("“Notes” moved up"),
    ).toHaveClass("sr-only")
    expect(screen.getByRole("region", { name: "Pages" })).toBeInTheDocument()
  })

  it("renames a page in place, and undo takes the old name back", async () => {
    const user = renderEditor()
    await pageCommand(user, "Notes", "Rename")
    const field = await screen.findByRole("textbox", { name: "Page name" })
    expect(field).toHaveFocus()
    await user.clear(field)
    await user.type(field, "Journal{Enter}")
    expect(row("Journal")).toBeInTheDocument()
    // A draft change: unsaved, and undone in one step.
    expect(screen.getByText("Unsaved")).toBeInTheDocument()
    await closePages(user)
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(await pageNames(user)).toEqual(["Home", "Tools", "Git", "Notes"])
  })

  it("adds a page inside a folder and shows it", async () => {
    const user = renderEditor()
    await pageCommand(user, "Tools", /New page inside/)
    expect(currentPage()).toBe("Page 2")
    expect(crumbNames()).toEqual(["Home", "Tools", "Page 2"])
    // The toolbar's + adds in the folder of the page shown.
    await user.click(screen.getByRole("button", { name: "Add page" }))
    expect(crumbNames()).toEqual(["Home", "Tools", "Page 3"])
  })

  it("is in Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    const user = renderEditor()
    await user.click(screen.getByRole("button", { name: /^Strona 1 z 1: Home/ }))
    const navigator = await screen.findByRole("region", { name: "Strony" })
    expect(within(navigator).getByRole("tree", { name: "Drzewo stron" })).toBeInTheDocument()
    await user.click(within(navigator).getByRole("button", { name: "Czynności: „Notes”" }))
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: /Zmień nazwę/ })).toBeInTheDocument()
  })
})

describe("a folder that closes after use", () => {
  it("is offered for pages inside a folder only, and saved with the layout", async () => {
    api.saveLayout.mockResolvedValue({ ...main, agentReload: { state: "applied" } })
    const user = renderEditor()
    const name = /Close after a key runs/
    const toggle = async (page: string) => {
      const menu = await rowMenu(user, page)
      return within(menu).queryByRole("menuitemcheckbox", { name })
    }
    // The home page has nowhere to return to.
    await managePages(user)
    expect(await toggle("Home")).not.toBeInTheDocument()
    await user.keyboard("{Escape}")

    const off = await toggle("Tools")
    expect(off).toHaveAttribute("aria-checked", "false")
    await user.click(off as HTMLElement)
    expect(await toggle("Tools")).toHaveAttribute("aria-checked", "true")
    await user.keyboard("{Escape}")
    await closePages(user)

    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.saveLayout).toHaveBeenCalled()
    })
    const saved = api.saveLayout.mock.lastCall?.[1] as DeviceLayout
    expect(saved.pages.find((p) => p.id === "tools")).toMatchObject({ autoReturn: true })
    // No other page got the field.
    expect(saved.pages.filter((p) => "autoReturn" in p).map((p) => p.id)).toEqual(["tools"])

    // Undo takes it back, and the page is as it was.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    await managePages(user)
    expect(await toggle("Tools")).toHaveAttribute("aria-checked", "false")
  })
})

describe("the compact Page Navigator", () => {
  for (const language of ["en", "pl"]) {
    it(`opens straight to the tree, with no headings or notes (${language})`, async () => {
      await act(async () => {
        await i18n.changeLanguage(language)
      })
      const user = renderEditor()
      await user.click(screen.getByRole("button", { name: /^(Page|Strona) 1 (of|z) 1: Home/ }))
      const navigator = await screen.findByRole("region", {
        name: i18n.t("editor:pageSettings.sheetTitle"),
      })
      await waitFor(() => {
        expect(api.pageTree).toHaveBeenCalled()
      })
      expect(within(navigator).queryByRole("heading")).not.toBeInTheDocument()
      expect(within(navigator).queryByRole("region")).not.toBeInTheDocument()
      expect(within(navigator).queryByRole("separator")).not.toBeInTheDocument()
      for (const text of [
        "Manage pages",
        "Go to page",
        "Zarządzaj stronami",
        "Przejdź do strony",
        "home page",
        "strona główna",
        "no way back",
        "Save or revert",
      ]) {
        expect(navigator).not.toHaveTextContent(text)
      }
      // Only the page names: Home, Tools, Git, Notes.
      expect(navigator).toHaveTextContent(/^HomeToolsGitNotes$/)
      expect(navigator).not.toHaveAttribute("aria-description")
    })
  }

  it("keeps the ⋯ button out of sight until hover or focus, but in the accessibility tree", async () => {
    const user = renderEditor()
    await managePages(user)
    const actions = within(row("Notes")).getByRole("button", { name: "Actions for “Notes”" })
    expect(actions).toHaveClass(
      "opacity-0",
      "group-hover:opacity-100",
      "group-focus-within:opacity-100",
    )
    expect(row("Notes")).toHaveClass("group")
    expect(actions).not.toHaveAttribute("aria-hidden")
    expect(actions).toBeVisible()
  })

  it("is compact: short rows, no connector lines, a subtle current page", async () => {
    const user = renderEditor()
    await managePages(user)
    expect(row("Git")).toHaveClass("h-8", "rounded-sm", "hover:bg-accent/60")
    expect(row("Git").querySelector(".w-px")).toBeNull()
    // Levels by indentation only.
    const indent = (name: string) => Number.parseFloat(row(name).style.paddingLeft)
    expect(indent("Tools") - indent("Home")).toBeCloseTo(indent("Git") - indent("Tools"))
    expect((indent("Tools") - indent("Home")) * 16).toBeGreaterThanOrEqual(12)
    expect((indent("Tools") - indent("Home")) * 16).toBeLessThanOrEqual(16)
    expect(row("Home")).toHaveClass("font-medium", "bg-accent/40")
    expect(row("Home")).not.toHaveClass("bg-accent")
  })

  it("is as tall as a short tree, scrolling inside only when needed", async () => {
    const small: DeviceLayout = {
      ...main,
      pages: [
        main.pages[0] as DeviceLayout["pages"][number],
        { id: "notes", name: "Notes", parent: "home", bindings: [] },
      ],
    }
    const user = renderEditor(small)
    const navigator = await managePages(user)
    expect(treeRows(navigator)).toHaveLength(2)
    const sizing = [navigator, ...navigator.querySelectorAll<HTMLElement>("*")].flatMap((e) =>
      [...e.classList].filter(
        (c) => /^(h|min-h)-\[/.test(c) || (/^h-\d/.test(c) && !e.matches("[role=treeitem]")),
      ),
    )
    expect(sizing).toEqual([])
    // As tall as the room below the bar at most (measured), as wide as the dock.
    expect(navigator.style.height).toBe("")
    expect(navigator).toHaveClass("absolute", "top-full")
    expect(navigator.querySelector("[data-slot=page-navigator-scroll]")).toHaveClass(
      "relative",
      "overflow-y-auto",
    )
  })

  it("cuts long names, with the full name as the tooltip", async () => {
    const long = "A very long page name that does not fit in the navigator at all"
    const user = renderEditor({
      ...main,
      pages: main.pages.map((p) => (p.id === "notes" ? { ...p, name: long } : p)),
    })
    await managePages(user)
    const name = row(long).querySelector("[data-slot=page-tree-name]")
    expect(name).toHaveClass("truncate", "min-w-0")
    expect(name).toHaveAttribute("title", long)
    // The user's name, verbatim, in either language.
    expect(name).toHaveTextContent(long)
  })
})
