// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { ActionInput } from "@/lib/api"
import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionInstance,
  ActionType,
  DeviceLayout,
  DeviceModel,
  ModuleInfo,
  ProfileView,
} from "@/lib/types"

import {
  catalogDragId,
  catalogGroups,
  defaultParameters,
  existingDefault,
  filterCatalog,
  freeActionName,
  needsNoInput,
  newAction,
} from "./catalog"
import { LayoutEditor } from "./editor"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const wrap = {
  key: "wrapAround",
  label: "Wrap around",
  kind: { type: "boolean" as const },
  required: true,
  default: { type: "boolean" as const, value: true },
}

const nextPage: ActionType = {
  id: "deck.next-page",
  name: "Next page",
  description: "Shows the next page of the layout on the device.",
  category: "Navigation",
  requiresProject: false,
  parameters: [wrap],
  node: { icon: "chevron-right" } as ActionType["node"],
}
const indicator: ActionType = {
  id: "deck.page-indicator",
  name: "Page indicator",
  description: "Shows the page the device is on.",
  category: "Navigation",
  requiresProject: false,
  parameters: [],
}
const openUrl: ActionType = {
  id: "system.open-url",
  name: "Open web page",
  description: "Opens a web page.",
  category: "System",
  requiresProject: false,
  parameters: [
    { key: "url", label: "URL", kind: { type: "text", multiline: false }, required: true },
  ],
}
const oldNext: ActionType = {
  ...nextPage,
  id: "projects.next",
  name: "Next project",
  category: "Projects",
  deprecated: true,
}
const wait: ActionType = {
  id: "general.wait",
  name: "Wait",
  description: "Waits a number of seconds.",
  category: "General",
  requiresProject: false,
  parameters: [
    {
      key: "seconds",
      label: "Seconds",
      kind: { type: "integer", min: 1, max: 300 },
      required: true,
      default: { type: "integer", value: 5 },
    },
  ],
}

const modules: ModuleInfo[] = [
  moduleInfo({ id: "deck", name: "Deck", actionTypes: [nextPage, indicator] }),
  moduleInfo({ id: "system", name: "System", actionTypes: [openUrl] }),
  moduleInfo({ id: "projects", name: "Projects", actionTypes: [oldNext] }),
  moduleInfo({ id: "general", name: "General", actionTypes: [wait], enabled: false }),
]

const instance = (over: Partial<ActionInstance>): ActionInstance => ({
  id: "a",
  name: "Next page",
  actionType: nextPage.id,
  parameters: { wrapAround: { type: "boolean", value: true } },
  description: "",
  icon: null,
  label: null,
  ...over,
})

describe("the catalog of action types", () => {
  it("offers types that need no input, of enabled modules, by category", () => {
    expect(needsNoInput(nextPage)).toBe(true)
    expect(needsNoInput(indicator)).toBe(true)
    // A URL has to be typed: that action is made in the Action Library.
    expect(needsNoInput(openUrl)).toBe(false)
    expect(catalogGroups(modules).map((g) => [g.label, g.entries.map((e) => e.type.id)])).toEqual([
      ["Navigation", ["deck.next-page", "deck.page-indicator"]],
    ])
    // An enabled module's types appear; plugins are left to the Action Library.
    const enabled = modules.map((m) => ({ ...m, enabled: true }))
    expect(catalogGroups(enabled).map((g) => g.label)).toEqual(["Navigation", "General"])
    const asPlugin = enabled.map((m) => (m.id === "general" ? { ...m, plugin: true } : m))
    expect(catalogGroups(asPlugin as ModuleInfo[]).map((g) => g.label)).toEqual(["Navigation"])
  })

  it("creates an action with the type's defaults and a name of its own", () => {
    expect(defaultParameters(nextPage)).toEqual({ wrapAround: { type: "boolean", value: true } })
    expect(defaultParameters(indicator)).toEqual({})
    expect(newAction([], nextPage)).toEqual({
      name: "Next page",
      actionType: "deck.next-page",
      parameters: { wrapAround: { type: "boolean", value: true } },
      description: "",
      icon: "chevron-right",
      label: null,
    })
    expect(freeActionName([instance({}), instance({ name: "next page 2" })], "Next page")).toBe(
      "Next page 3",
    )
  })

  it("reuses an action that is exactly a new one, and no other", () => {
    expect(existingDefault([instance({})], nextPage)?.id).toBe("a")
    // Renamed by the user: still the same action.
    expect(existingDefault([instance({ name: "Forward" })], nextPage)?.id).toBe("a")
    // Other settings, a label of its own, another type: not that action.
    const stops = { wrapAround: { type: "boolean" as const, value: false } }
    expect(existingDefault([instance({ parameters: stops })], nextPage)).toBeUndefined()
    expect(existingDefault([instance({ label: ">" })], nextPage)).toBeUndefined()
    expect(existingDefault([instance({ actionType: "deck.previous-page" })], nextPage)).toBe(
      undefined,
    )
  })

  it("searches names, descriptions, categories and modules", () => {
    const groups = catalogGroups(modules)
    const ids = (query: string) =>
      filterCatalog(groups, query).flatMap((g) => g.entries.map((e) => e.type.id))
    expect(ids("")).toHaveLength(2)
    expect(ids("indicator")).toEqual(["deck.page-indicator"])
    expect(ids("NAVIGATION")).toHaveLength(2)
    expect(ids("deck")).toHaveLength(2)
    expect(ids("soundboard")).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// In the editor

const createAction = vi.fn()
const refresh = vi.fn()

const dnd = vi.hoisted(() => ({
  onDragEnd: undefined as ((event: DragEndEvent) => void) | undefined,
  data: new Map<string, unknown>(),
}))

vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  return {
    ...actual,
    DndContext: (props: { onDragEnd?: (event: DragEndEvent) => void; children?: ReactNode }) => {
      dnd.onDragEnd = props.onDragEnd
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
    saveLayout: (_profile: string, saved: DeviceLayout) =>
      Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
    createAction: (...args: unknown[]) => createAction(...args) as unknown,
  },
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => modules,
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh }),
}))
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "unverified",
  verification: [],
}
const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  pages: [{ id: "home", name: "Home", bindings: [] }],
}

function renderEditor(actions: ActionInstance[] = []) {
  const view = {
    activeProjectId: null,
    profile: { id: "p", name: "Work", actions, workflows: [], projects: [], layouts: [layout] },
  } as unknown as ProfileView
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
  return userEvent.setup()
}

const describes = (name: string) => screen.getByRole("button", { name }).getAttribute("title")

function drop(id: string, over: string) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  const onDragEnd = dnd.onDragEnd
  if (!onDragEnd) throw new Error("the editor has no drop handler")
  act(() => {
    onDragEnd({
      active: { id, data: { current: dnd.data.get(id) } },
      over: { id: over },
    } as unknown as DragEndEvent)
  })
}

beforeEach(() => {
  dnd.data.clear()
  refresh.mockClear()
  vi.mocked(toast.error).mockClear()
  createAction.mockReset()
  let made = 0
  createAction.mockImplementation((_profile: string, input: ActionInput) => {
    made += 1
    return Promise.resolve({ ...input, id: `new-${made}` })
  })
})
afterEach(cleanup)

describe("the catalog in the layout editor", { timeout: 20_000 }, () => {
  it("puts a type on a key by creating its action once", async () => {
    renderEditor()
    const section = screen.getByRole("region", { name: "New: Navigation" })
    expect(within(section).getByText("Next page")).toBeInTheDocument()
    expect(within(section).getByText("Page indicator")).toBeInTheDocument()
    // Types that need settings, deprecated types and disabled modules' are
    // not offered here.
    expect(screen.queryByText("Open web page")).not.toBeInTheDocument()
    expect(screen.queryByText("Next project")).not.toBeInTheDocument()
    expect(screen.queryByText("Wait")).not.toBeInTheDocument()

    drop(catalogDragId("deck.next-page"), "key:0")
    await waitFor(() => {
      expect(describes("Key 1")).toBe("Key 1: Press: Next page")
    })
    expect(createAction).toHaveBeenCalledExactlyOnceWith("p", {
      name: "Next page",
      actionType: "deck.next-page",
      parameters: { wrapAround: { type: "boolean", value: true } },
      description: "",
      icon: "chevron-right",
      label: null,
    })
    // The profile is reloaded, so the Action Library shows the new action.
    expect(refresh).toHaveBeenCalledTimes(1)
    // It is in the library now, as an action of the profile.
    expect(
      within(screen.getByRole("region", { name: "Actions" })).getByText("Next page"),
    ).toBeInTheDocument()

    // The same type on another key: the same action, nothing is created.
    drop(catalogDragId("deck.next-page"), "key:1")
    await waitFor(() => {
      expect(describes("Key 2")).toBe("Key 2: Press: Next page")
    })
    expect(createAction).toHaveBeenCalledTimes(1)
    // Another type: its own action.
    drop(catalogDragId("deck.page-indicator"), "key:2")
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Press: Page indicator")
    })
    expect(createAction).toHaveBeenCalledTimes(2)
  })

  it("reuses the profile's action and says when one cannot be created", async () => {
    renderEditor([instance({ id: "mine", name: "Forward" })])
    drop(catalogDragId("deck.next-page"), "key:4")
    await waitFor(() => {
      expect(describes("Key 5")).toBe("Key 5: Press: Forward")
    })
    expect(createAction).not.toHaveBeenCalled()

    createAction.mockRejectedValueOnce(new Error("the configuration is locked"))
    drop(catalogDragId("deck.page-indicator"), "key:5")
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("“Page indicator” could not be added", {
        description: "the configuration is locked",
      })
    })
    // Nothing was put on the key.
    expect(describes("Key 6")).toBe("Key 6")
  })

  it("searches the whole library and folds its groups", async () => {
    const user = renderEditor([instance({ id: "mine", name: "Forward" })])
    const search = screen.getByRole("searchbox", { name: "Search the library" })
    await user.type(search, "indicator")
    expect(screen.getByText("Page indicator")).toBeInTheDocument()
    expect(screen.queryByText("Forward")).not.toBeInTheDocument()
    expect(screen.queryByText("New folder")).not.toBeInTheDocument()
    await user.clear(search)
    await user.type(search, "zzz")
    expect(screen.getByText("Nothing matches.")).toBeInTheDocument()
    await user.clear(search)
    expect(screen.getByText("Forward")).toBeInTheDocument()
    expect(screen.getByText("New folder")).toBeInTheDocument()

    const fold = screen.getByRole("button", { name: /New: Navigation/ })
    expect(fold).toHaveAttribute("aria-expanded", "true")
    await user.click(fold)
    expect(fold).toHaveAttribute("aria-expanded", "false")
    expect(screen.queryByText("Page indicator")).not.toBeInTheDocument()
    // A search shows what it finds, also in a folded group.
    await user.type(search, "indicator")
    expect(screen.getByText("Page indicator")).toBeInTheDocument()
  })
})
