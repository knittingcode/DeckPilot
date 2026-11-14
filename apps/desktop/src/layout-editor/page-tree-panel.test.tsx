// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { DeviceLayout, DeviceModel, PageTree, ProfileView } from "@/lib/types"
import { installDomStubs } from "@/test/dom"

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
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh: vi.fn() }),
}))
vi.mock("sonner", () => ({ toast }))

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

function renderEditor() {
  const view = {
    activeProjectId: null,
    profile: {
      id: "p",
      name: "Work",
      actions: [],
      workflows: [],
      projects: [],
      layouts: [main, travel],
    },
  } as unknown as ProfileView
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={plus}
      saved={main}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return userEvent.setup()
}

const panel = () => within(screen.getByRole("region", { name: "Page tree" }))
const tabs = () => screen.getAllByRole("tab").map((t) => t.textContent)

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
afterEach(cleanup)

describe("page tree panel", () => {
  it("shows the folders of the layout and opens pages from it", async () => {
    const user = renderEditor()
    const git = await panel().findByRole("button", { name: "Git" })
    // Git is inside Tools, which is inside Home.
    const home = panel().getByRole("button", { name: "Home" }).closest("li") as HTMLElement
    const tools = within(home).getByRole("button", { name: "Tools" }).closest("li") as HTMLElement
    expect(within(tools).getByRole("button", { name: "Git" })).toBe(git)
    expect(panel().getByRole("list", { name: "Page tree warnings" })).toHaveTextContent(
      "page `Git` has no way back",
    )
    // Home cannot be moved.
    expect(panel().queryByRole("button", { name: "Move or copy “Home”" })).not.toBeInTheDocument()

    await user.click(git)
    expect(screen.getByRole("tab", { name: "Git" })).toHaveAttribute("aria-selected", "true")
    expect(panel().getByRole("button", { name: "Git" })).toHaveAttribute("aria-current", "page")
  })

  it("reorders sibling pages and shows the stored order", async () => {
    const reordered = {
      ...main,
      pages: [main.pages[0], main.pages[3], main.pages[2], main.pages[1]],
    }
    api.reorderPages.mockResolvedValue(reordered)
    const user = renderEditor()
    await panel().findByRole("button", { name: "Notes" })
    expect(panel().getByRole("button", { name: "Move “Git” up" })).toBeDisabled()
    expect(panel().getByRole("button", { name: "Move “Notes” down" })).toBeDisabled()

    await user.click(panel().getByRole("button", { name: "Move “Notes” up" }))
    expect(api.reorderPages).toHaveBeenCalledWith("p", "main", ["home", "notes", "git", "tools"])
    await waitFor(() => {
      expect(tabs()).toEqual(["Home", "Notes", "Git", "Tools"])
    })
    // Stored by the command: nothing left to save.
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument()
  })

  it("needs unsaved changes to be saved first", async () => {
    const user = renderEditor()
    await panel().findByRole("button", { name: "Notes" })
    await user.type(screen.getByRole("textbox", { name: "Layout name" }), " 2")
    expect(panel().getByText(/Save or revert your changes/)).toBeInTheDocument()
    expect(panel().getByRole("button", { name: "Move “Notes” up" })).toBeDisabled()
    expect(panel().getByRole("button", { name: "Move or copy “Tools”" })).toBeDisabled()
  })

  it("moves a folder with its pages to another layout", async () => {
    const after = {
      ...main,
      pages: [main.pages[0], main.pages[3]].map((p) => ({ ...p, bindings: [] })),
    }
    api.movePage.mockResolvedValue({ page: "tools", removedKeys: 1, layouts: [after, travel] })
    const user = renderEditor()
    await user.click(await panel().findByRole("button", { name: "Move or copy “Tools”" }))
    const dialog = await screen.findByRole("dialog", { name: "Move or copy “Tools”" })
    await user.click(within(dialog).getByRole("combobox", { name: "Destination" }))
    // Tools cannot go into itself or into Git, which is inside it.
    expect(screen.queryByRole("option", { name: "Inside “Git”" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("option", { name: "Home page of “Travel”" }))
    await user.click(within(dialog).getByRole("button", { name: "Move" }))

    expect(api.movePage).toHaveBeenCalledWith("p", "main", "tools", "travel", null)
    await waitFor(() => {
      expect(tabs()).toEqual(["Home", "Notes"])
    })
    expect(toast.success).toHaveBeenCalledWith("“Tools” moved to the home page of “Travel”", {
      description: "1 key that opened it was removed.",
    })
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })

  it("copies within the layout and opens the copy", async () => {
    const copy = { id: "notes-2", name: "Notes", parent: "tools", bindings: [] }
    api.copyPage.mockResolvedValue({
      page: "notes-2",
      removedKeys: 0,
      layouts: [{ ...main, explicitTree: true, pages: [...main.pages, copy] }],
    })
    const user = renderEditor()
    await user.click(await panel().findByRole("button", { name: "Move or copy “Notes”" }))
    const dialog = await screen.findByRole("dialog")
    await user.click(within(dialog).getByRole("combobox", { name: "Destination" }))
    await user.click(screen.getByRole("option", { name: "Inside “Tools”" }))
    await user.click(within(dialog).getByRole("button", { name: "Copy" }))

    expect(api.copyPage).toHaveBeenCalledWith("p", "main", "notes", null, "tools")
    await waitFor(() => {
      expect(screen.getAllByRole("tab", { name: "Notes" })[1]).toHaveAttribute(
        "aria-selected",
        "true",
      )
    })
    expect(toast.success).toHaveBeenCalledWith("“Notes” copied to “Tools”", undefined)
  })

  it("says why a page was not moved and keeps the dialog open", async () => {
    api.movePage.mockRejectedValue("the model of “Travel” has no dials")
    const user = renderEditor()
    await user.click(await panel().findByRole("button", { name: "Move or copy “Notes”" }))
    const dialog = await screen.findByRole("dialog")
    await user.click(within(dialog).getByRole("button", { name: "Move" }))
    expect(api.movePage).toHaveBeenCalledWith("p", "main", "notes", null, null)
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "“Notes” was not moved: the model of “Travel” has no dials",
    )
  })
})
