// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { DeviceLayout, ProfileView } from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { DEVICE_MODELS } from "@/test/fixtures"
import { closePages, currentPage, goToPage, managePages, pageNames } from "@/test/pages"

import { LayoutEditor } from "./editor"
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
    expect(nav.closest("[data-slot=device-workspace]")).toBe(
      canvas.closest("[data-slot=device-workspace]"),
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
    for (const name of ["Previous page", "Next page", "Add page", "Manage pages"]) {
      expect(within(nav).getByRole("button", { name })).toHaveAttribute("title")
    }
    button("Next page").focus()
    await user.keyboard("{Enter}")
    expect(currentPage()).toBe("Tools")
    await user.tab({ shift: true })
    await user.tab({ shift: true })
    expect(button("Previous page")).toHaveFocus()
    await user.keyboard(" ")
    expect(currentPage()).toBe("Home")

    // The page number opens the menu of every page, folders included.
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

    // Renamed in the pages panel.
    const pages = await managePages(user)
    const name = within(pages).getByRole("textbox", { name: "Page name" })
    await user.clear(name)
    await user.type(name, "Lights")
    await closePages(user)
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
    const location = screen.getByRole("navigation", { name: "Page location" })
    expect(
      within(location)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["Home", "Tools", "Git"])
    // Still the place of its top-level page.
    expect(screen.getByText("Page 2 of 3")).toBeInTheDocument()

    await user.click(within(location).getByRole("button", { name: "Back" }))
    expect(currentPage()).toBe("Tools")
    expect(screen.queryByRole("navigation", { name: "Page location" })).not.toBeInTheDocument()
  })

  it("keeps the page settings and the page tree in the pages panel", async () => {
    const user = renderEditor()
    await goToPage(user, "Git")
    const pages = await managePages(user)
    // The open page's settings: a folder can close after use.
    const settings = within(pages).getByRole("region", { name: "This page" })
    expect(within(settings).getByRole("textbox", { name: "Page name" })).toHaveValue("Git")
    expect(
      within(settings).getByRole("switch", { name: "Close this folder after a key runs" }),
    ).toBeInTheDocument()
    expect(within(settings).getByRole("button", { name: "Delete page" })).toBeInTheDocument()
    // The tree, with reordering and moving or copying.
    const tree = within(pages).getByRole("region", { name: "Page tree" })
    expect(await within(tree).findByRole("button", { name: "Move “Media” up" })).toBeEnabled()
    expect(within(tree).getByRole("button", { name: "Move or copy “Git”" })).toBeEnabled()
    await user.click(within(tree).getByRole("button", { name: "Media" }))
    await closePages(user)
    expect(currentPage()).toBe("Media")
  })
})
