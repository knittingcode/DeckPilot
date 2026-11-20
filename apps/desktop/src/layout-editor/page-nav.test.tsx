// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { DeviceLayout, ProfileView } from "@/lib/types"
import { i18n } from "@/i18n"
import { installDomStubs } from "@/test/dom"
import { DEVICE_MODELS } from "@/test/fixtures"
import {
  breadcrumbs,
  crumbNames,
  currentPage,
  goToPage,
  managePages,
  pageNames,
  renamePage,
} from "@/test/pages"

import { LayoutEditor } from "./editor"
import { PageNavigationDock } from "./page-nav"
import { newPageName, pageNavigation } from "./page-navigation"

const saveLayout = vi.fn()

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    pageTree: (layout: DeviceLayout) =>
      Promise.resolve({
        pages: layout.pages.map((p) => ({
          id: p.id,
          name: p.name,
          parent: p.id === "git" ? "tools" : p.id === layout.homePage ? null : "home",
          stored: false,
          depth: 0,
          automaticBack: null,
        })),
        warnings: [],
      }),
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => saveLayout(...args) as unknown,
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => [],
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh: vi.fn() }),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

const mk2 = DEVICE_MODELS.mk2

/** Three top-level pages; "Git" is a folder in "Tools", opened by one of its keys. */
const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  explicitTree: true,
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
      ],
    },
    {
      id: "tools",
      name: "Tools",
      bindings: [
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "press" },
          target: { type: "action", action: "mail" },
        },
        {
          control: { kind: "key", index: 4 },
          trigger: { type: "press" },
          target: { type: "openPage", page: "git" },
        },
      ],
    },
    { id: "git", name: "Git", parent: "tools", bindings: [] },
    { id: "media", name: "Media", bindings: [] },
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

function renderEditor() {
  const user = userEvent.setup()
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={mk2}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const button = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => button(name).getAttribute("title")

beforeAll(installDomStubs)
beforeEach(() => {
  saveLayout.mockReset()
  saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
})
afterEach(cleanup)

describe("page navigation helpers", () => {
  it("moves along the top-level pages without wrapping around", () => {
    expect(pageNavigation(layout, "home")).toEqual({
      number: 1,
      count: 3,
      previous: undefined,
      next: "tools",
    })
    expect(pageNavigation(layout, "media")).toEqual({
      number: 3,
      count: 3,
      previous: "tools",
      next: undefined,
    })
    // A folder is where its top-level page is.
    expect(pageNavigation(layout, "git")).toMatchObject({ number: 2, previous: "home" })
  })

  it("names a new page after the pages there are, never twice", () => {
    expect(newPageName(layout)).toBe("Page 4")
    const taken = { ...layout, pages: [...layout.pages, { id: "x", name: "Page 4", bindings: [] }] }
    // "Page 4" is now the fourth top-level page and taken: the next free one.
    expect(newPageName(taken)).toBe("Page 5")
  })
})

describe("the page navigation under the device", { timeout: 20_000 }, () => {
  it("sits under the device, instead of tabs and a page tree above it", () => {
    renderEditor()
    expect(screen.queryByRole("tab")).not.toBeInTheDocument()
    expect(screen.queryByRole("textbox", { name: "New page name" })).not.toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Page tree" })).not.toBeInTheDocument()
    const canvas = screen.getByRole("group", { name: "Stream Deck MK.2 controls" })
    const nav = screen.getByRole("toolbar", { name: "Page navigation" })
    // In the document after the device, in the same workspace.
    expect(canvas.compareDocumentPosition(nav) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    // On the divider right under the canvas region, outside the inspector.
    const divider = nav.closest("[data-slot=canvas-divider]")
    expect(divider?.previousElementSibling).toBe(canvas.closest("[data-slot=device-workspace]"))
    expect(nav.closest("[data-slot=inspector-scroll]")).toBeNull()
    // No separate page name: the page number carries it.
    expect(screen.getByRole("button", { name: "Page 1 of 3: Home" })).toHaveAttribute(
      "title",
      "Home, page 1 of 3. Go to another page",
    )
  })

  it("goes to the previous and next top-level page and stops at the ends", async () => {
    const user = renderEditor()
    expect(screen.getByText("Page 1 of 3")).toBeInTheDocument()
    expect(currentPage()).toBe("Home")
    expect(button("Previous page")).toBeDisabled()
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")

    await user.click(button("Next page"))
    expect(screen.getByText("Page 2 of 3")).toBeInTheDocument()
    expect(currentPage()).toBe("Tools")
    // The keys of the page, at once.
    expect(describes("Key 1")).toBe("Key 1")
    expect(describes("Key 2")).toBe("Key 2: Press: Mail")

    // The folder "Git" is skipped: it is not a top-level page.
    await user.click(button("Next page"))
    expect(currentPage()).toBe("Media")
    expect(screen.getByText("Page 3 of 3")).toBeInTheDocument()
    expect(button("Next page")).toBeDisabled()
    expect(button("Next page")).toHaveAttribute("title", "This is the last page")

    await user.click(button("Previous page"))
    await user.click(button("Previous page"))
    expect(currentPage()).toBe("Home")
    expect(button("Previous page")).toBeDisabled()
  })

  it("is used with the keyboard", async () => {
    const user = renderEditor()
    const nav = screen.getByRole("toolbar", { name: "Page navigation" })
    for (const name of ["Previous page", "Next page", "Add page"]) {
      expect(within(nav).getByRole("button", { name })).toHaveAttribute("title")
    }
    // One entry point for the pages: the counter (no tree or manage button).
    expect(within(nav).queryByRole("button", { name: "Manage pages" })).not.toBeInTheDocument()
    button("Next page").focus()
    await user.keyboard("{Enter}")
    expect(currentPage()).toBe("Tools")
    await user.tab({ shift: true })
    await user.tab({ shift: true })
    expect(button("Previous page")).toHaveFocus()
    await user.keyboard(" ")
    expect(currentPage()).toBe("Home")

    // The page number opens the Page Navigator, folders included.
    expect(await pageNames(user)).toEqual(["Home", "Tools", "Git", "Media"])
    await goToPage(user, "Git")
    expect(currentPage()).toBe("Git")
  })

  it("adds a top-level page and shows it", async () => {
    const user = renderEditor()
    await user.click(button("Add page"))
    expect(currentPage()).toBe("Page 4")
    expect(screen.getByText("Page 4 of 4")).toBeInTheDocument()
    expect(screen.getByText(/^This (page|folder) is empty$/)).toBeInTheDocument()
    expect(button("Next page")).toBeDisabled()
    expect(screen.getByText("Unsaved")).toBeInTheDocument()

    // Renamed in the Page Navigator.
    await renamePage(user, "Page 4", "Lights")
    expect(currentPage()).toBe("Lights")

    await user.click(button("Save"))
    const [, saved] = saveLayout.mock.lastCall as [string, DeviceLayout]
    expect(saved.pages.at(-1)).toMatchObject({ name: "Lights", bindings: [] })
    expect(saved.pages.at(-1)?.parent).toBeUndefined()
  })

  it("shows where a folder is and goes back from it", async () => {
    const user = renderEditor()
    await user.click(button("Next page"))
    await user.dblClick(button("Key 5"))
    expect(currentPage()).toBe("Git")
    const location = screen.getByRole("navigation", { name: "Breadcrumb" })
    expect(
      within(location)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["Home", "Tools", "Git"])
    // Still the place of its top-level page.
    expect(screen.getByText("Page 2 of 3")).toBeInTheDocument()

    // No Back button: the breadcrumbs go up, to that level.
    expect(screen.queryByRole("button", { name: "Back" })).not.toBeInTheDocument()
    await user.click(within(location).getByRole("button", { name: "Tools" }))
    expect(currentPage()).toBe("Tools")
    // The level clicked, with the way there kept: Home › Tools.
    expect(crumbNames()).toEqual(["Home", "Tools"])
    expect(within(breadcrumbs()).getByText("Tools")).toHaveAttribute("aria-current", "page")
    await user.click(within(breadcrumbs()).getByRole("button", { name: "Home" }))
    // A top-level page: the breadcrumbs show only itself, not as a button.
    expect(crumbNames()).toEqual(["Home"])
    expect(within(breadcrumbs()).queryByRole("button")).not.toBeInTheDocument()
  })

  it("keeps the page settings and the page tree in the Page Navigator", async () => {
    const user = renderEditor()
    await goToPage(user, "Git")
    const pages = await managePages(user)
    // The tree, with renaming, deleting, reordering and moving or copying.
    const tree = within(pages).getByRole("tree", { name: "Page tree" })
    expect(within(tree).getByRole("treeitem", { name: "Git" })).toHaveAttribute(
      "aria-current",
      "page",
    )
    // The open page's settings, in its row's menu: a folder can close after use.
    await user.click(within(pages).getByRole("button", { name: "Actions for “Git”" }))
    expect(
      within(await screen.findByRole("menu")).getByRole("menuitemcheckbox", {
        name: /Close after a key runs/,
      }),
    ).toHaveAttribute("aria-checked", "false")
    await user.keyboard("{Escape}")
    await user.click(within(pages).getByRole("button", { name: "Actions for “Media”" }))
    const menu = await screen.findByRole("menu")
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent),
    ).toEqual(["Rename", "Duplicate", "Delete page"])
    await user.keyboard("{Escape}")
    await user.click(within(tree).getByRole("treeitem", { name: "Media" }))
    expect(currentPage()).toBe("Media")
  })
})

describe("renaming the page shown in its breadcrumb", { timeout: 20_000 }, () => {
  const name = () => screen.getByText(currentPage() ?? "", { selector: "[aria-current=page]" })
  const field = () => screen.getByRole("textbox", { name: "Page name" })

  async function startEditing(user: ReturnType<typeof userEvent.setup>) {
    await user.dblClick(name())
    return field()
  }

  it("edits the current segment in place, focused and selected", async () => {
    const user = renderEditor()
    const input = await startEditing(user)
    expect(input).toHaveFocus()
    expect(input).toHaveValue("Home")
    expect((input as HTMLInputElement).selectionStart).toBe(0)
    expect((input as HTMLInputElement).selectionEnd).toBe(4)
    // In place of the name, as wide as the text (AutoSizeInput), whole.
    expect(name()).toHaveClass("hidden")
    expect(input.closest("[data-slot=auto-size-input]")).not.toBeNull()
    expect(Number.parseFloat(input.style.width)).toBeGreaterThanOrEqual(64)
    expect(input.closest("[data-slot=page-breadcrumbs]")).not.toBeNull()
    // No navigation, no page change from that double-click.
    expect(currentPage()).toBe("Home")
  })

  it("saves with Enter, and the navigator, counter and undo follow", async () => {
    const user = renderEditor()
    const input = await startEditing(user)
    await user.clear(input)
    await user.type(input, "Start{Enter}")
    expect(screen.queryByRole("textbox", { name: "Page name" })).not.toBeInTheDocument()
    expect(currentPage()).toBe("Start")
    expect(crumbNames()).toEqual(["Start"])
    expect(screen.getByRole("button", { name: /^Page 1 of 3: Start/ })).toHaveAttribute(
      "title",
      "Start, page 1 of 3. Go to another page",
    )
    expect(await pageNames(user)).toEqual(["Start", "Tools", "Git", "Media"])
    expect(screen.getByText("Unsaved")).toBeInTheDocument()
    await user.click(button("Undo"))
    expect(currentPage()).toBe("Home")
  })

  it("cancels with Escape and saves when it loses focus", async () => {
    const user = renderEditor()
    let input = await startEditing(user)
    await user.clear(input)
    await user.type(input, "Nope{Escape}")
    expect(currentPage()).toBe("Home")
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument()
    input = await startEditing(user)
    await user.clear(input)
    await user.type(input, "Elsewhere")
    await user.click(button("Key 1"))
    expect(currentPage()).toBe("Elsewhere")
  })

  it("refuses an empty name and writes nothing for the same name, once", async () => {
    const user = renderEditor()
    const input = await startEditing(user)
    await user.clear(input)
    await user.type(input, "   {Enter}")
    expect(field()).toHaveAttribute("aria-invalid", "true")
    expect(await screen.findByText("A page needs a name")).toBeInTheDocument()
    expect(field()).toHaveFocus()
    await user.clear(field())
    await user.type(field(), "Home{Enter}")
    expect(screen.queryByRole("textbox", { name: "Page name" })).not.toBeInTheDocument()
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument()
    // Enter then blur commits once: one undo step.
    const again = await startEditing(user)
    await user.clear(again)
    await user.type(again, "Once{Enter}")
    again.blur()
    await user.click(button("Undo"))
    expect(currentPage()).toBe("Home")
    expect(button("Undo")).toBeDisabled()
  })

  it("keeps the editor's shortcuts out of the field", async () => {
    const user = renderEditor()
    await user.click(button("Key 1"))
    const input = await startEditing(user)
    await user.type(input, "{Backspace}{Delete}{ArrowLeft}+-0")
    // Key 1 still has its action: Delete went to the field.
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    await user.keyboard("{Escape}")
  })

  it("renames a folder's page by its id, in Polish too", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    try {
      const user = renderEditor()
      await user.click(button("Następna strona"))
      await user.dblClick(button("Klawisz 5"))
      await user.dblClick(screen.getByText("Git", { selector: "[aria-current=page]" }))
      const input = screen.getByRole("textbox", { name: "Nazwa strony" })
      await user.clear(input)
      await user.type(input, "Repo{Enter}")
      const crumbs = screen.getByRole("navigation", { name: "Ścieżka nawigacji" })
      expect(
        within(crumbs)
          .getAllByRole("listitem")
          .map((li) => li.textContent),
      ).toEqual(["Home", "Tools", "Repo"])
      // The folder above keeps its name.
      expect(within(crumbs).getByRole("button", { name: "Tools" })).toBeInTheDocument()
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    }
  })
})

describe("the page stepper", () => {
  it("is one tight group of 32px controls, also at 10 / 100", () => {
    const many: DeviceLayout = {
      id: "many",
      name: "Many",
      model: mk2.id,
      homePage: "p1",
      pages: Array.from({ length: 100 }, (_, i) => ({
        id: `p${String(i + 1)}`,
        name: `Page ${String(i + 1)}`,
        bindings: [],
      })),
    }
    render(
      <PageNavigationDock
        draft={many}
        pageId="p10"
        trail={[]}
        onOpen={vi.fn()}
        onNavigate={vi.fn()}
        onAdd={vi.fn()}
        onRename={vi.fn()}
        navigator={() => null}
      />,
    )
    const group = document.querySelector("[data-slot=page-stepper]") as HTMLElement
    expect(group).toHaveClass("flex", "items-center", "gap-1", "px-1")
    expect(group.className).not.toMatch(/justify-between|w-\d/)
    const [previous, counter, next] = [...group.children] as [HTMLElement, HTMLElement, HTMLElement]
    expect(previous).toHaveAccessibleName("Previous page")
    expect(next).toHaveAccessibleName("Next page")
    for (const arrow of [previous, next]) {
      expect(arrow).toHaveAttribute("data-size", "icon-nav")
      expect(arrow).toHaveClass("size-8")
    }
    expect(counter).toHaveAttribute("data-size", "nav")
    expect(counter).toHaveClass("h-8", "px-2.5", "tabular-nums")
    expect(counter).toHaveAccessibleName("Page 10 of 100: Page 10")
    expect(counter.querySelector("[aria-hidden]")?.textContent).toBe("10/100")
    expect(previous).toBeEnabled()
    expect(next).toBeEnabled()
  })
})

describe("one size for every control of the page navigation", () => {
  const nested: DeviceLayout = {
    id: "l",
    name: "Main",
    model: "m",
    homePage: "home",
    pages: [
      { id: "home", name: "Home", bindings: [] },
      {
        id: "a",
        name: "A rather long folder name for the breadcrumbs",
        parent: "home",
        bindings: [],
      },
      { id: "b", name: "Tools", parent: "a", bindings: [] },
      { id: "c", name: "Git", parent: "b", bindings: [] },
    ],
  }
  const renderNav = (draft: DeviceLayout, pageId: string) =>
    render(
      <PageNavigationDock
        draft={draft}
        pageId={pageId}
        trail={[]}
        onOpen={vi.fn()}
        onNavigate={vi.fn()}
        onAdd={vi.fn()}
        onRename={vi.fn()}
        navigator={() => null}
      />,
    )
  const NO_ONE_OFF = /(^|\s)(px|py|pl|pr|p|h|size)-[\w.[\]]+/g
  /** Sizing classes beyond the shared size's own. */
  const extra = (element: Element, shared: string[]) =>
    (element.className.match(NO_ONE_OFF) ?? [])
      .map((c) => c.trim())
      .filter((c) => !shared.includes(c))

  for (const language of ["en", "pl"]) {
    it(`sizes breadcrumbs, arrows, counter and + alike (${language})`, async () => {
      await act(async () => {
        await i18n.changeLanguage(language)
      })
      renderNav(nested, "c")
      const nav = document.querySelector("[data-slot=page-nav]") as HTMLElement
      const buttons = [...nav.querySelectorAll("button")]
      expect(buttons.length).toBeGreaterThanOrEqual(6)
      for (const button of buttons) {
        const size = button.getAttribute("data-size")
        expect(["nav", "icon-nav"]).toContain(size)
        expect(extra(button, size === "nav" ? ["h-8", "px-2.5"] : ["size-8"])).toEqual([])
      }
      // The page shown: the same size, not a control, set apart by weight.
      const current = nav.querySelector("[data-slot=page-name]") as HTMLElement
      expect(current.tagName).toBe("SPAN")
      expect(current).toHaveAttribute("aria-current", "page")
      expect(current).toHaveClass("h-8", "px-2.5", "font-medium")
      expect(extra(current, ["h-8", "px-2.5"])).toEqual([])
      for (const link of nav.querySelectorAll("[data-slot=breadcrumb-link]")) {
        expect(link).toHaveClass("font-normal")
        expect(link.className).not.toMatch(/(^|\s)bg-/)
      }
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    })
  }

  it("counts 1 / 1 at the same size", () => {
    renderNav({ ...nested, pages: [nested.pages[0] as DeviceLayout["pages"][number]] }, "home")
    const counter = document.querySelector("[data-slot=page-counter]") as HTMLElement
    expect(counter).toHaveAttribute("data-size", "nav")
    expect(counter.querySelector("[aria-hidden]")?.textContent).toBe("1/1")
  })
})
