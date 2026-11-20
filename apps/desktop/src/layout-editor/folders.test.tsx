// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"

import { LayoutEditor } from "./editor"
import { NEW_FOLDER_DRAG_ID } from "./library-panel"
import { closePages, currentPage, managePages, pageCommand, pageNames, upTo } from "@/test/pages"

// Radix menus and dialogs rely on browser APIs that jsdom does not implement.
Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const saveLayout = vi.fn()

/**
 * The editor's drag and drop as it is wired: what every draggable of the
 * real components hands to dnd-kit, and the editor's own drop handler.
 * jsdom has no layout, so a pointer drag cannot find what it is over; a
 * drop is made by calling the handler with the real drag data instead.
 */
const dnd = vi.hoisted(() => ({
  onDragEnd: undefined as ((event: DragEndEvent) => void) | undefined,
  data: new Map<string, unknown>(),
}))

vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  return {
    ...actual,
    DndContext: (props: {
      id?: string
      onDragEnd?: (event: DragEndEvent) => void
      children?: ReactNode
    }) => {
      // The editor's own, not the Page Navigator's (page-tree).
      if (props.id !== "page-tree") dnd.onDragEnd = props.onDragEnd
      return <actual.DndContext {...props} />
    },
    useDraggable: (args: Parameters<typeof actual.useDraggable>[0]) => {
      dnd.data.set(String(args.id), args.data)
      return actual.useDraggable(args)
    },
  }
})
vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => saveLayout(...args) as unknown,
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

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: {
    keys: { rows: 3, columns: 5, image: null },
    encoders: null,
    touch: null,
  },
  support: "unverified",
  verification: [],
}

const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mk2.id,
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
          control: { kind: "key", index: 1 },
          trigger: { type: "press" },
          target: { type: "action", action: "mail" },
          label: "Inbox",
        },
      ],
    },
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

const control = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => control(name).getAttribute("title")
const tabs = pageNames
const selectedTab = currentPage

/** Drops the draggable `id` on the control `over` ("key:3"), as dnd-kit does. */
function drop(id: string, over: string | null) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  const onDragEnd = dnd.onDragEnd
  if (!onDragEnd) throw new Error("the editor has no drop handler")
  act(() => {
    onDragEnd({
      active: { id, data: { current: dnd.data.get(id) } },
      over: over === null ? null : { id: over },
    } as unknown as DragEndEvent)
  })
}

async function save(user: ReturnType<typeof userEvent.setup>): Promise<DeviceLayout> {
  await user.click(screen.getByRole("button", { name: /Save/ }))
  const [, saved] = saveLayout.mock.lastCall as [string, DeviceLayout]
  return saved
}

beforeEach(() => {
  dnd.data.clear()
  saveLayout.mockReset()
  saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  vi.mocked(toast.error).mockClear()
})
afterEach(cleanup)

// Long scripts of real interactions with the whole editor; see the note in
// `editor.test.tsx` on why they get more than the default time.
describe("folders in the layout editor", { timeout: 20_000 }, () => {
  it("creates a folder by dragging New folder onto an empty key", async () => {
    const user = renderEditor()
    expect(
      within(screen.getByRole("region", { name: "Navigation" })).getByText("New folder"),
    ).toBeInTheDocument()
    drop(NEW_FOLDER_DRAG_ID, "key:4")
    // The key opens a folder that now exists, as a page of its own.
    expect(describes("Key 5")).toBe("Key 5: Press: Open “Folder”")
    expect(await tabs(user)).toEqual(["Home", "Folder"])
    // The editor stays on the page; the new key is selected.
    expect(selectedTab()).toBe("Home")
    expect(control("Key 5")).toHaveAttribute("aria-pressed", "true")

    // Once more: another folder with a name of its own.
    drop(NEW_FOLDER_DRAG_ID, "key:5")
    expect(describes("Key 6")).toBe("Key 6: Press: Open “Folder 2”")

    const saved = await save(user)
    const [home, first, second] = saved.pages
    expect(saved.pages).toHaveLength(3)
    // Stable ids, stored inside the page they were created on.
    expect(first).toMatchObject({ name: "Folder", parent: "home", bindings: [] })
    expect(second).toMatchObject({ name: "Folder 2", parent: "home", bindings: [] })
    expect(first?.id).not.toBe(second?.id)
    expect(home?.bindings.slice(2).map((b) => b.target)).toEqual([
      { type: "openPage", page: first?.id },
      { type: "openPage", page: second?.id },
    ])
    // The keys that were there are untouched.
    expect(home?.bindings.slice(0, 2)).toEqual(layout.pages[0]?.bindings)
  })

  it("does not put a folder on a key that does something", async () => {
    const user = renderEditor()
    drop(NEW_FOLDER_DRAG_ID, "key:0")
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    expect(await tabs(user)).toEqual(["Home"])
    expect(toast.error).toHaveBeenCalledWith("Key 1 already does something", expect.anything())
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument()
    // Dropped beside the keys: nothing happens.
    drop(NEW_FOLDER_DRAG_ID, null)
    expect(await tabs(user)).toEqual(["Home"])
  })

  it("creates a folder from the key menu and undoes it in one step", async () => {
    const user = renderEditor()
    // A key that does something offers no folder.
    fireEvent.contextMenu(control("Key 1"))
    expect(await screen.findByRole("menuitem", { name: /New folder/ })).toHaveAttribute(
      "data-disabled",
    )
    await user.keyboard("{Escape}")

    fireEvent.contextMenu(control("Key 3"))
    await user.click(await screen.findByRole("menuitem", { name: /New folder/ }))
    expect(describes("Key 3")).toBe("Key 3: Press: Open “Folder”")
    expect(await tabs(user)).toEqual(["Home", "Folder"])
    // Now the menu offers to open it instead.
    fireEvent.contextMenu(control("Key 3"))
    expect(await screen.findByRole("menuitem", { name: /Open “Folder”/ })).toBeInTheDocument()
    expect(screen.queryByRole("menuitem", { name: /New folder/ })).not.toBeInTheDocument()
    await user.keyboard("{Escape}")

    // One undo removes the key and the page together; redo brings both back.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(describes("Key 3")).toBe("Key 3")
    expect(await tabs(user)).toEqual(["Home"])
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Redo" }))
    expect(describes("Key 3")).toBe("Key 3: Press: Open “Folder”")
    expect(await tabs(user)).toEqual(["Home", "Folder"])
  })

  it("enters a folder by double-click, takes an action there and returns", async () => {
    const user = renderEditor()
    drop(NEW_FOLDER_DRAG_ID, "key:4")
    // A double-click on a key that opens nothing only selects it.
    await user.dblClick(control("Key 1"))
    expect(selectedTab()).toBe("Home")

    await user.dblClick(control("Key 5"))
    expect(selectedTab()).toBe("Folder")
    expect(screen.getByText("This folder is empty")).toBeInTheDocument()
    expect(
      within(screen.getByRole("navigation", { name: "Breadcrumb" }))
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["Home", "Folder"])

    // An action from the library, dropped on a key of the folder.
    drop("library:action:mail", "key:2")
    expect(describes("Key 3")).toBe("Key 3: Press: Mail")
    // A folder in the folder.
    drop(NEW_FOLDER_DRAG_ID, "key:3")
    expect(describes("Key 4")).toBe("Key 4: Press: Open “Folder 2”")
    await user.dblClick(control("Key 4"))
    expect(selectedTab()).toBe("Folder 2")

    // Back goes up one folder at a time, to the page each was opened from.
    await upTo(user, "Folder")
    expect(selectedTab()).toBe("Folder")
    await upTo(user, "Home")
    expect(selectedTab()).toBe("Home")
    // The home page still has its keys.
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")

    const saved = await save(user)
    const folder = saved.pages.find((p) => p.name === "Folder")
    const inner = saved.pages.find((p) => p.name === "Folder 2")
    expect(inner?.parent).toBe(folder?.id)
    expect(folder?.bindings.map((b) => b.target)).toEqual([
      { type: "action", action: "mail" },
      { type: "openPage", page: inner?.id },
    ])
  })

  it("moves a key onto an empty key and swaps two keys by dragging", async () => {
    const user = renderEditor()
    // Onto an empty key: the binding moves with its label.
    drop("control:key:1", "key:7")
    expect(describes("Key 2")).toBe("Key 2")
    expect(describes("Key 8")).toBe("Key 8: Press: Inbox")
    // Onto a key that does something: the two swap.
    drop("control:key:0", "key:7")
    expect(describes("Key 1")).toBe("Key 1: Press: Inbox")
    expect(describes("Key 8")).toBe("Key 8: Press: Shop")
    // Onto itself: nothing changes.
    drop("control:key:0", "key:0")
    expect(describes("Key 1")).toBe("Key 1: Press: Inbox")

    const saved = await save(user)
    expect(
      saved.pages[0]?.bindings.map((b) => [b.control.index, b.target, b.label ?? null]),
    ).toEqual([
      [7, { type: "action", action: "shop" }, null],
      [0, { type: "action", action: "mail" }, "Inbox"],
    ])
  })

  it("asks before deleting a folder and keeps it when cancelled", async () => {
    const user = renderEditor()
    drop(NEW_FOLDER_DRAG_ID, "key:4")
    await user.dblClick(control("Key 5"))
    drop("library:action:mail", "key:0")
    drop(NEW_FOLDER_DRAG_ID, "key:1")

    await pageCommand(user, "Folder", /Delete page/)
    const dialog = await screen.findByRole("alertdialog", { name: "Delete “Folder”?" })
    expect(dialog).toHaveTextContent("2 controls on it are deleted with it.")
    expect(dialog).toHaveTextContent(
      "1 control on other pages that opens it will do nothing afterwards.",
    )
    expect(dialog).toHaveTextContent("The page inside it (“Folder 2”) is kept and moves to “Home”.")
    expect(dialog).toHaveTextContent("You can undo this until the layout is saved.")

    // Cancelled: everything is as it was.
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    expect(await tabs(user)).toEqual(["Home", "Folder", "Folder 2"])
    expect(selectedTab()).toBe("Folder")
    expect(describes("Key 1")).toBe("Key 1: Press: Mail")

    // Confirmed: the folder goes, the editor shows the page it was on, the
    // key that opened it is empty and the folder inside it is kept.
    await pageCommand(user, "Folder", /Delete page/)
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete page" }),
    )
    expect(await tabs(user)).toEqual(["Home", "Folder 2"])
    expect(selectedTab()).toBe("Home")
    expect(describes("Key 5")).toBe("Key 5")
    const saved = await save(user)
    expect(saved.pages.map((p) => [p.name, p.parent])).toEqual([
      ["Home", undefined],
      ["Folder 2", "home"],
    ])

    // And undone: the folder, its keys and the key that opens it are back.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(await tabs(user)).toEqual(["Home", "Folder", "Folder 2"])
    expect(describes("Key 5")).toBe("Key 5: Press: Open “Folder”")
  })

  it("never offers to delete the home page", async () => {
    const user = renderEditor()
    const pages = await managePages(user)
    // No explanation in the navigator, and no Delete in the home page's menu.
    expect(pages).not.toHaveTextContent("is the home page")
    await user.click(within(pages).getByRole("button", { name: "Actions for “Home”" }))
    const menu = await screen.findByRole("menu")
    expect(within(menu).queryByRole("menuitem", { name: /Delete page/ })).not.toBeInTheDocument()
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent),
    ).toEqual(["Rename", "Add page"])
    await user.keyboard("{Escape}")
    await closePages(user)
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })
})
