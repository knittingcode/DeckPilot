// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// Folder keys in the layout editor (`lib/folder-control.ts`): selected, the
// inspector shows the folder's settings only (no gestures, actions or
// placeholders, also for a key that still has more stored on it), Library
// drops onto them are refused with the drop feedback, their menu offers no
// configuration, and their recovered gestures can be moved to a free key or
// discarded after the shared confirmation, in English and Polish.

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { DialogProvider } from "@/components/dialogs/dialog-service"
import { i18n } from "@/i18n"
import type { ControlBinding, DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"
import { installDomStubs } from "@/test/dom"

import { LayoutEditor } from "./editor"

const saveLayout = vi.fn()

/** The editor's drag and drop as wired (see folders.test.tsx). */
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
    keys: { rows: 3, columns: 5, image: { width: 72, height: 72 } as never },
    encoders: null,
    touch: null,
  },
  support: "verified",
  verification: [],
}

const key = (index: number) => ({ kind: "key" as const, index })
const run = (index: number, action: string, type = "press"): ControlBinding => ({
  control: key(index),
  trigger: { type } as ControlBinding["trigger"],
  target: { type: "action", action },
  label: null,
})
const opens = (index: number, page: string): ControlBinding => ({
  ...run(index, "x"),
  target: { type: "openPage", page },
})

/**
 * Home: Key 2 opens the folder Tools, with two gestures recovered by
 * migration 12; Key 3 runs Shop; Key 4 opens the top-level page Other (no
 * folder key).
 */
function makeLayout(home: ControlBinding[] = []): DeviceLayout {
  return {
    id: "layout",
    name: "Main",
    model: mk2.id,
    homePage: "home",
    explicitTree: true,
    pages: [
      {
        id: "home",
        name: "Home",
        bindings: [opens(1, "tools"), run(2, "shop"), opens(3, "other"), ...home],
      },
      {
        id: "tools",
        name: "Tools",
        parent: "home",
        bindings: [],
        back: { position: "reserved" },
        autoExit: { enabled: false, seconds: 10 },
      },
      { id: "other", name: "Other", bindings: [] },
    ],
    recovered: [
      {
        page: "home",
        control: key(1),
        folder: "tools",
        bindings: [run(1, "shop", "doubleClick"), run(1, "shop", "longPress")],
        gestures: { control: key(1), longPressMs: 900 },
        reason: "folderControl",
      },
    ],
  }
}

function renderEditor(layout = makeLayout()) {
  const view = {
    activeProjectId: null,
    profile: {
      id: "p",
      name: "Work",
      description: "",
      actions: [{ id: "shop", name: "Shop", icon: null }],
      workflows: [],
      projects: [],
      layouts: [layout],
    },
  } as unknown as ProfileView
  const user = userEvent.setup()
  render(
    <DialogProvider>
      <LayoutEditor
        profileId="p"
        view={view}
        model={mk2}
        saved={layout}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />
    </DialogProvider>,
  )
  return user
}

const control = (name: string) => screen.getByRole("button", { name })
function inspector(): HTMLElement {
  const found = document.querySelector<HTMLElement>(
    "[data-slot=device-editor] [data-slot=inspector]",
  )
  if (!found) throw new Error("no inspector")
  return found
}

function drop(id: string, over: string) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  act(() => {
    dnd.onDragEnd?.({
      active: { id, data: { current: dnd.data.get(id) } },
      over: { id: over },
    } as unknown as DragEndEvent)
  })
}

async function save(user: ReturnType<typeof userEvent.setup>): Promise<DeviceLayout> {
  await user.click(screen.getByRole("button", { name: /Save/ }))
  const [, saved] = saveLayout.mock.lastCall as [string, DeviceLayout]
  return saved
}

/** No gesture section, chooser, slot or placeholder. */
function expectNoGestures(region: HTMLElement) {
  expect(region.querySelector("[data-gesture-slot]")).toBeNull()
  expect(within(region).queryAllByRole("combobox")).toEqual([])
  // The key's look: the shared key header (preview, visible title, title
  // appearance), the same as every other key's.
  expect(region.querySelector('[data-slot="key-appearance"]')).not.toBeNull()
  expect(within(region).getByRole("textbox", { name: "Folder name" })).toBeInTheDocument()
  expect(within(region).getByRole("button", { name: "Title appearance" })).toBeInTheDocument()
  expect(within(region).queryByText(/Single press|Double click|Long press/)).toBeNull()
  expect(within(region).queryByRole("button", { name: /Remove/ })).toBeNull()
}

beforeAll(installDomStubs)
beforeEach(() => {
  dnd.data.clear()
  saveLayout.mockReset()
  saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  vi.mocked(toast.error).mockClear()
})
afterEach(cleanup)

describe("folder keys in the editor", { timeout: 20_000 }, () => {
  it("shows only the folder's settings for a folder key", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    const region = inspector()
    expect(region.querySelector("[data-slot=folder-control]")).not.toBeNull()
    expectNoGestures(region)
    // The folder's own settings; no button to open it (a double click on
    // the key does) and no separator above them.
    expect(within(region).queryByRole("button", { name: /open folder/i })).toBeNull()
    const settings = within(region).getByRole("region", { name: "Folder settings" })
    expect(settings).not.toHaveClass("border-t")
    // One name: the key header's title is the folder's name, and the
    // settings have no field of their own for it.
    const fields = within(region).getAllByRole("textbox")
    expect(fields).toHaveLength(1)
    expect(fields[0]).toBe(within(region).getByRole("textbox", { name: "Folder name" }))
    expect(fields[0]).toHaveValue("Tools")
    expect(within(settings).queryByRole("textbox")).toBeNull()
    expect(within(settings).getByRole("switch", { name: "Auto Exit" })).toBeInTheDocument()
    // No explanations about what is missing.
    expect(within(region).queryByText(/gesture.*not|cannot/i)).toBeNull()
    // An action key and a key that opens a top-level page keep their gestures.
    await user.click(control("Key 3"))
    expect(inspector().querySelector("[data-slot=folder-control]")).toBeNull()
    expect(within(inspector()).getAllByRole("combobox").length).toBeGreaterThan(0)
    await user.click(control("Key 4"))
    expect(inspector().querySelector("[data-slot=folder-control]")).toBeNull()
    // Opening the folder with a double click on its key.
    await user.dblClick(control("Key 2"))
    expect(control("Key 1")).toHaveAttribute("data-system")
  })

  it("shows the folder settings only for a key that still has gestures stored", async () => {
    // A key that opens a folder of its page next to other gestures (as a
    // layout of a model this build does not know could keep them; stored
    // ones are recovered by migration 12): a folder key all the same.
    const user = renderEditor(
      makeLayout([run(1, "shop", "doubleClick"), run(1, "shop", "longPress")]),
    )
    await user.click(control("Key 2"))
    expect(inspector().querySelector("[data-slot=folder-control]")).not.toBeNull()
    expectNoGestures(inspector())
  })

  it("refuses Library drops on a folder key with the drop feedback", async () => {
    const user = renderEditor()
    drop("library:action:shop", "key:1")
    expect(toast.error).toHaveBeenCalledTimes(1)
    expect(vi.mocked(toast.error).mock.lastCall?.[1]).toMatchObject({
      description: "This key opens a folder and runs nothing else.",
    })
    // A folder never goes next to what a key runs either.
    drop("library:page:tools", "key:2")
    expect(toast.error).toHaveBeenCalledTimes(2)
    expect(vi.mocked(toast.error).mock.lastCall?.[1]).toMatchObject({
      description: "A folder opens from a key of its own: drop it on a free key.",
    })
    // Nothing changed: the save sends the layout as it was.
    await user.click(control("Key 2"))
    fireEvent.change(within(inspector()).getByRole("textbox", { name: "Folder name" }), {
      target: { value: "Tools 2" },
    })
    const saved = await save(user)
    expect(saved.pages[0]?.bindings).toEqual(makeLayout().pages[0]?.bindings)
  })

  it("moves with another key: dropping a key on it swaps them", async () => {
    const user = renderEditor()
    drop("control:key:2", "key:1")
    const saved = await save(user)
    const home = saved.pages[0]?.bindings ?? []
    expect(home.find((b) => b.control.index === 2)?.target).toEqual({
      type: "openPage",
      page: "tools",
    })
    expect(home.find((b) => b.control.index === 1)?.target).toEqual({
      type: "action",
      action: "shop",
    })
  })

  it("offers no configuration and no assignment in its menu", async () => {
    renderEditor()
    fireEvent.contextMenu(control("Key 2"))
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: /Open “Tools”/ })).toBeInTheDocument()
    expect(within(menu).queryByRole("menuitem", { name: /Configure action/ })).toBeNull()
    expect(within(menu).queryByText("Assign")).toBeNull()
    expect(within(menu).getByRole("menuitem", { name: /Paste/ })).toHaveAttribute(
      "aria-disabled",
      "true",
    )
    // Removing the key stays possible.
    expect(within(menu).getByRole("menuitem", { name: /Remove/ })).toBeInTheDocument()
  })

  it("moves recovered gestures to the first free key", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    const item = within(inspector()).getByRole("region", { name: "Recovered gestures (2)" })
    await user.click(within(item).getByRole("button", { name: "Move to a free key" }))
    expect(within(inspector()).queryByRole("region", { name: /Recovered gestures/ })).toBeNull()
    const saved = await save(user)
    expect(saved.recovered).toBeUndefined()
    // Key 1 (index 0) is free on Home (no folder, no reserved Back).
    expect(
      saved.pages[0]?.bindings.filter((b) => b.control.index === 0).map((b) => b.trigger.type),
    ).toEqual(["doubleClick", "longPress"])
    expect(saved.pages[0]?.gestures).toEqual([{ control: key(0), longPressMs: 900 }])
  })

  it("says so when no key is free for them", async () => {
    const full = Array.from({ length: 15 }, (_, i) => i)
      .filter((i) => i > 3)
      .map((i) => run(i, "shop"))
    const user = renderEditor(makeLayout([run(0, "shop"), ...full]))
    await user.click(control("Key 2"))
    await user.click(within(inspector()).getByRole("button", { name: "Move to a free key" }))
    expect(within(inspector()).getByRole("alert")).toHaveTextContent(
      "Every key of this page runs something.",
    )
  })

  it("discards recovered gestures after the destructive confirmation", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    await user.click(within(inspector()).getByRole("button", { name: "Discard" }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Discard 2 recovered gestures?")
    // Cancelled: kept.
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(
      within(inspector()).getByRole("region", { name: "Recovered gestures (2)" }),
    ).toBeInTheDocument()
    await user.click(within(inspector()).getByRole("button", { name: "Discard" }))
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Discard" }),
    )
    expect(within(inspector()).queryByRole("region", { name: /Recovered gestures/ })).toBeNull()
    const saved = await save(user)
    expect(saved.recovered).toBeUndefined()
    expect(saved.pages[0]?.bindings).toEqual(makeLayout().pages[0]?.bindings)
  })

  it("speaks Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    try {
      const user = renderEditor()
      await user.click(control("Klawisz 2"))
      const item = within(inspector()).getByRole("region", { name: "Odzyskane gesty (2)" })
      expect(
        within(item).getByRole("button", { name: "Przenieś na wolny klawisz" }),
      ).toBeInTheDocument()
      expect(within(item).getByRole("button", { name: "Odrzuć" })).toBeInTheDocument()
      expect(within(inspector()).getByRole("textbox", { name: "Nazwa folderu" })).toHaveValue(
        "Tools",
      )
      expect(within(inspector()).queryByRole("button", { name: /folderu/ })).toBeNull()
      drop("library:action:shop", "key:1")
      expect(vi.mocked(toast.error).mock.lastCall?.[1]).toMatchObject({
        description: "Ten klawisz otwiera folder i nie uruchamia niczego innego.",
      })
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    }
  })
})

describe("a folder's one name", { timeout: 20_000 }, () => {
  it("is edited in the key header and nowhere else, with one undo step per burst", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    const name = within(inspector()).getByRole("textbox", { name: "Folder name" })
    await user.clear(name)
    await user.type(name, "Build tools")
    // The burst of typing is one undo step.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(within(inspector()).getByRole("textbox", { name: "Folder name" })).toHaveValue("Tools")
    await user.click(screen.getByRole("button", { name: "Redo" }))
    const saved = await save(user)
    expect(saved.pages.find((p) => p.id === "tools")?.name).toBe("Build tools")
    // No second copy on the key's binding.
    expect(saved.pages[0]?.bindings.find((b) => b.control.index === 1)?.label).toBeFalsy()
    // The breadcrumbs of the folder follow the same name.
    await user.dblClick(control("Key 2"))
    expect(screen.getAllByText("Build tools").length).toBeGreaterThan(0)
  })

  it("offers a title the key kept from before as the folder's name, or discards it", async () => {
    const kept = makeLayout()
    kept.recovered = [
      ...(kept.recovered ?? []),
      {
        page: "home",
        control: key(1),
        folder: "tools",
        bindings: [{ ...opens(1, "tools"), label: "My tools" }],
        reason: "folderLabel",
      },
    ]
    const user = renderEditor(kept)
    await user.click(control("Key 2"))
    const region = within(within(inspector()).getByRole("region", { name: "Earlier key title" }))
    expect(region.getByText(/“My tools”/)).toBeInTheDocument()
    // Not counted as a recovered gesture.
    expect(
      within(inspector()).getByRole("region", { name: "Recovered gestures (2)" }),
    ).toBeInTheDocument()
    await user.click(region.getByRole("button", { name: "Use as folder name" }))
    expect(within(inspector()).getByRole("textbox", { name: "Folder name" })).toHaveValue(
      "My tools",
    )
    const saved = await save(user)
    expect(saved.pages.find((p) => p.id === "tools")?.name).toBe("My tools")
    expect(saved.recovered?.every((entry) => entry.reason === "folderControl")).toBe(true)
  })
})
