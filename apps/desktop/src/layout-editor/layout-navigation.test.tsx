// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { ActionInput } from "@/lib/api"
import { backUnavailable } from "@/lib/layout-navigation"
import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionInstance,
  ActionType,
  DeviceInfo,
  DeviceLayout,
  DeviceModel,
  ModuleInfo,
  ProfileView,
  Usage,
} from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { deviceInfo } from "@/test/fixtures"

import { presetDragId } from "./catalog"
import { LayoutEditor } from "./editor"

const text = (key: string, label: string, required: boolean) => ({
  key,
  label,
  kind: { type: "text" as const, multiline: false },
  required,
})
const switchLayout: ActionType = {
  id: "deck.switch-layout",
  name: "Switch layout",
  description: "Shows another layout of the active profile on this device.",
  category: "Navigation",
  requiresProject: false,
  parameters: [
    text("layout", "Layout", true),
    text("page", "Page", false),
    {
      key: "history",
      label: "Back",
      kind: { type: "choice", options: ["push", "replace", "clear"] },
      required: true,
      default: { type: "choice", value: "push" },
    },
  ],
  controls: { key: ["press"] },
  node: { icon: "app-window" } as ActionType["node"],
}
const previousLayout: ActionType = {
  id: "deck.previous-layout",
  name: "Back to previous layout",
  description: "Shows the layout this device showed before the current one.",
  category: "Navigation",
  requiresProject: false,
  parameters: [],
  controls: { key: ["press"] },
  node: { icon: "arrow-left" } as ActionType["node"],
}
const modules: ModuleInfo[] = [
  moduleInfo({ id: "deck", name: "Deck", actionTypes: [switchLayout, previousLayout] }),
]

const dnd = vi.hoisted(() => ({
  onDragEnd: undefined as ((event: DragEndEvent) => void) | undefined,
  data: new Map<string, unknown>(),
}))
const calls = vi.hoisted(() => ({
  createAction: vi.fn(),
  updateAction: vi.fn(),
  refresh: vi.fn(),
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
    createAction: (...args: unknown[]) => calls.createAction(...args) as unknown,
    updateAction: (...args: unknown[]) => calls.updateAction(...args) as unknown,
  },
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => modules,
  useAppContext: () => ({
    profiles: { activeProfileId: "p", profiles: [] },
    refresh: calls.refresh,
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
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "unverified",
  verification: [],
}

function layout(id: string, name: string, change: Partial<DeviceLayout> = {}): DeviceLayout {
  return {
    id,
    name,
    model: mk2.id,
    homePage: `${id}-home`,
    pages: [{ id: `${id}-home`, name: "Home", bindings: [] }],
    ...change,
  }
}
const teams = layout("teams", "Teams", {
  pages: [
    { id: "teams-home", name: "Home", bindings: [] },
    { id: "teams-calls", name: "Calls", bindings: [] },
  ],
})
const media = layout("media", "Media")
const big = layout("big", "Big board", { model: "elgato.streamdeck.xl" })

const toTeams: ActionInstance = {
  id: "to-teams",
  name: "Teams",
  actionType: switchLayout.id,
  parameters: {
    layout: { type: "text", value: "teams" },
    history: { type: "choice", value: "push" },
  },
  description: "",
  icon: "app-window",
  label: null,
}
const back: ActionInstance = {
  id: "back",
  name: "Back to previous layout",
  actionType: previousLayout.id,
  parameters: {},
  icon: "arrow-left",
}

/** Main, the open layout, with `action` on Key 1. */
function mainWith(action?: string): DeviceLayout {
  return layout("main", "Main", {
    pages: [
      {
        id: "main-home",
        name: "Home",
        bindings: action
          ? [
              {
                control: { kind: "key", index: 0 },
                trigger: { type: "press" },
                target: { type: "action", action },
              },
            ]
          : [],
      },
    ],
  })
}

function renderEditor({
  saved = mainWith(),
  actions = [],
  usages = {},
  device = null,
}: {
  saved?: DeviceLayout
  actions?: ActionInstance[]
  usages?: Record<string, Usage[]>
  device?: DeviceInfo | null
} = {}) {
  const view = {
    activeProjectId: null,
    actionUsages: usages,
    workflowUsages: {},
    issues: [],
    profile: {
      id: "p",
      name: "Work",
      description: "",
      actions,
      workflows: [],
      projects: [],
      layouts: [saved, teams, media, big],
      activeLayouts: { [mk2.id]: "main" },
    },
  } as ProfileView
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={mk2}
      saved={saved}
      device={device}
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

beforeAll(installDomStubs)
beforeEach(() => {
  dnd.data.clear()
  calls.refresh.mockClear()
  calls.createAction.mockReset()
  calls.updateAction.mockReset()
  let made = 0
  calls.createAction.mockImplementation((_profile: string, input: ActionInput) => {
    made += 1
    return Promise.resolve({ ...input, id: `new-${made}` })
  })
  calls.updateAction.mockImplementation((_profile: string, action: ActionInstance) =>
    Promise.resolve(action),
  )
})
afterEach(cleanup)

describe("Switch layout in the library", { timeout: 20_000 }, () => {
  it("offers the profile's other layouts of the model and Back to previous layout", () => {
    renderEditor()
    const section = screen.getByRole("region", { name: "New: Navigation" })
    expect(within(section).getByText("Back to previous layout")).toBeInTheDocument()
    expect(within(section).getByText("Switch to Teams")).toBeInTheDocument()
    expect(within(section).getByText("Switch to Media")).toBeInTheDocument()
    // Not the open layout, not a layout of another model, not the bare type.
    expect(within(section).queryByText("Switch to Main")).not.toBeInTheDocument()
    expect(within(section).queryByText("Switch to Big board")).not.toBeInTheDocument()
    expect(within(section).queryByText("Switch layout")).not.toBeInTheDocument()
  })

  it("puts a Switch layout action on a key, created once and then reused", async () => {
    renderEditor()
    drop(presetDragId("switch-layout:teams"), "key:0")
    await waitFor(() => {
      expect(describes("Key 1")).toBe("Key 1: Press: Teams")
    })
    expect(calls.createAction).toHaveBeenCalledExactlyOnceWith("p", {
      name: "Teams",
      actionType: "deck.switch-layout",
      parameters: {
        layout: { type: "text", value: "teams" },
        history: { type: "choice", value: "push" },
      },
      description: "",
      icon: "app-window",
      label: null,
    })
    drop(presetDragId("switch-layout:teams"), "key:1")
    await waitFor(() => {
      expect(describes("Key 2")).toBe("Key 2: Press: Teams")
    })
    expect(calls.createAction).toHaveBeenCalledTimes(1)
  })

  it("reuses the profile's identical Switch layout action", async () => {
    renderEditor({ actions: [{ ...toTeams, name: "Calls & chat" }] })
    drop(presetDragId("switch-layout:teams"), "key:3")
    await waitFor(() => {
      expect(describes("Key 4")).toBe("Key 4: Press: Calls & chat")
    })
    expect(calls.createAction).not.toHaveBeenCalled()
  })
})

describe("the Switch layout inspector", { timeout: 20_000 }, () => {
  async function select(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    return screen.findByRole("region", { name: "Switch layout" })
  }

  it("shows the destination and offers only the profile's layouts for the model", async () => {
    const user = renderEditor({ saved: mainWith("to-teams"), actions: [toTeams] })
    const panel = await select(user)
    expect(panel).toHaveTextContent("Shows: Teams")
    expect(panel).toHaveTextContent("Only layouts for Stream Deck MK.2 assigned to this profile.")
    // Without a label, the key shows the layout's name.
    expect(screen.getByRole("textbox", { name: "Key title" })).toHaveAttribute(
      "placeholder",
      "Teams",
    )
    await user.click(within(panel).getByRole("combobox", { name: "Layout" }))
    const options = await screen.findAllByRole("option")
    expect(options.map((o) => o.textContent)).toEqual(["Maindefault", "Teams", "Media"])
    await user.type(screen.getByPlaceholderText("Search layouts"), "med")
    await waitFor(() => {
      expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["Media"])
    })
  })

  it("changes the action of a key no other key uses", async () => {
    const user = renderEditor({ saved: mainWith("to-teams"), actions: [toTeams] })
    const panel = await select(user)
    await user.click(within(panel).getByRole("combobox", { name: "Layout" }))
    await user.click(await screen.findByRole("option", { name: /Media/ }))
    await waitFor(() => {
      expect(calls.updateAction).toHaveBeenCalledWith("p", {
        ...toTeams,
        // Named after its layout, so the name follows.
        name: "Media",
        parameters: {
          layout: { type: "text", value: "media" },
          history: { type: "choice", value: "push" },
        },
      })
    })
    expect(calls.createAction).not.toHaveBeenCalled()
    await waitFor(() => {
      expect(panel).toHaveTextContent("Shows: Media")
    })
  })

  it("gives the key an action of its own when other keys run the same one", async () => {
    const user = renderEditor({
      saved: mainWith("to-teams"),
      actions: [toTeams],
      usages: {
        "to-teams": [
          {
            type: "layoutBinding",
            layout: "media",
            layoutName: "Media",
            page: "media-home",
            pageName: "Home",
            control: { kind: "key", index: 2 },
          },
        ],
      },
    })
    const panel = await select(user)
    expect(panel).toHaveTextContent("Other keys run “Teams” too")
    await user.click(within(panel).getByRole("combobox", { name: "Layout" }))
    await user.click(await screen.findByRole("option", { name: /Media/ }))
    await waitFor(() => {
      expect(describes("Key 1")).toBe("Key 1: Press: Media")
    })
    expect(calls.updateAction).not.toHaveBeenCalled()
    expect(calls.createAction).toHaveBeenCalledExactlyOnceWith("p", {
      name: "Media",
      actionType: "deck.switch-layout",
      parameters: {
        layout: { type: "text", value: "media" },
        history: { type: "choice", value: "push" },
      },
      description: "",
      icon: "app-window",
      label: null,
    })
  })

  it("sets the page and what Back does", async () => {
    const user = renderEditor({ saved: mainWith("to-teams"), actions: [toTeams] })
    const panel = await select(user)
    await user.click(within(panel).getByRole("combobox", { name: "Page" }))
    await user.click(await screen.findByRole("option", { name: "Calls" }))
    await waitFor(() => {
      expect(calls.updateAction).toHaveBeenLastCalledWith("p", {
        ...toTeams,
        parameters: {
          layout: { type: "text", value: "teams" },
          page: { type: "text", value: "teams-calls" },
          history: { type: "choice", value: "push" },
        },
      })
    })
    await waitFor(() => {
      expect(within(panel).getByRole("combobox", { name: "Back" })).toBeEnabled()
    })
    await user.click(within(panel).getByRole("combobox", { name: "Back" }))
    await user.click(await screen.findByRole("option", { name: "Back skips this layout" }))
    await waitFor(() => {
      expect(calls.updateAction).toHaveBeenLastCalledWith(
        "p",
        expect.objectContaining({
          parameters: expect.objectContaining({
            history: { type: "choice", value: "replace" },
          }) as unknown,
        }),
      )
    })
  })
})

describe("Back to previous layout", { timeout: 20_000 }, () => {
  const showsMain = (previousLayout?: string) =>
    deviceInfo({
      layout: { type: "compatible", layout: "main", name: "Main" },
      ...(previousLayout ? { previousLayout } : {}),
    })

  it("explains the key and says when the device cannot go back", async () => {
    const user = userEvent.setup()
    renderEditor({ saved: mainWith("back"), actions: [back], device: showsMain() })
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    const panel = await screen.findByRole("region", { name: "Back to previous layout" })
    expect(panel).toHaveTextContent(
      "Returns this device to the layout it showed before, in this profile. Does nothing when there is none. A folder's Back key returns to a page instead.",
    )
    expect(within(panel).getByRole("status")).toHaveTextContent(
      "this device has no previous layout",
    )
    // The preview shows the key as the device does: unavailable.
    const key = screen.getByRole("button", { name: "Key 1" })
    expect(key.querySelector("[data-unavailable]")).toHaveTextContent("No previous layout")
  })

  it("is available when the device has a layout to go back to", async () => {
    const user = userEvent.setup()
    renderEditor({ saved: mainWith("back"), actions: [back], device: showsMain("teams") })
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    const panel = await screen.findByRole("region", { name: "Back to previous layout" })
    expect(within(panel).getByRole("status")).toHaveTextContent("Now: back to Teams.")
    expect(
      screen.getByRole("button", { name: "Key 1" }).querySelector("[data-unavailable]"),
    ).toBeNull()
  })

  it("is unavailable only while the device shows this layout with no way back", () => {
    const bindings = mainWith("back").pages[0]?.bindings ?? []
    expect([...backUnavailable(bindings, [back], showsMain(), "main")]).toEqual(["key:0"])
    expect(backUnavailable(bindings, [back], showsMain("teams"), "main").size).toBe(0)
    expect(backUnavailable(bindings, [back], showsMain(), "teams").size).toBe(0)
    expect(backUnavailable(bindings, [back], null, "main").size).toBe(0)
    expect(
      backUnavailable(bindings, [back], { ...showsMain(), state: "disconnected" }, "main").size,
    ).toBe(0)
    expect(backUnavailable(bindings, [toTeams], showsMain(), "main").size).toBe(0)
  })
})
