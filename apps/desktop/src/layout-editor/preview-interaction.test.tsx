// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { DialogProvider } from "@/components/dialogs"
import { i18n } from "@/i18n"
import type {
  ActionType,
  AgentConnection,
  ControlFace,
  DeviceInfo,
  DeviceLayout,
  ProfileView,
  SimulatePressResult,
} from "@/lib/types"
import { EditorSelectionContext, type EditorSelection } from "@/shell/editor-selection-context"
import { installDomStubs } from "@/test/dom"
import { DEVICE_MODELS, agentStatus, deviceInfo } from "@/test/fixtures"
import { emitBackend, resetBackend } from "@/test/tauri"
import { crumbNames, currentPage, upTo } from "@/test/pages"

import { LayoutEditor } from "./editor"
import { resetLayoutHistory } from "./preview-navigation"
import { LIVE_PREVIEW_EVENT } from "./use-live-preview"
import type { EditorPlace } from "./use-layout-editing"

const mocks = vi.hoisted(() => ({
  simulatePress: vi.fn(),
  runAction: vi.fn(),
  saveLayout: vi.fn(),
  previewLayout: vi.fn(),
  livePreviewBegin: vi.fn(),
  livePreviewUpdate: vi.fn(),
  livePreviewEnd: vi.fn(),
  types: [] as unknown[],
  modules: [] as unknown[],
  context: mockContext(),
}))

/** The app context the editor reads: the profile on the device and the agent. */
function mockContext(): {
  profiles: { activeProfileId: string | null; profiles: never[] }
  refresh: () => void
  agent: unknown
} {
  return { profiles: { activeProfileId: "p", profiles: [] }, refresh: () => undefined, agent: null }
}

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: (...args: unknown[]) => mocks.previewLayout(...args) as unknown,
    previewMarquee: () => Promise.resolve(null),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve(mocks.types),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => mocks.saveLayout(...args) as unknown,
    simulatePress: (...args: unknown[]) => mocks.simulatePress(...args) as unknown,
    runAction: (...args: unknown[]) => mocks.runAction(...args) as unknown,
    watchKeyStates: () => Promise.resolve(0),
    livePreviewBegin: (...args: unknown[]) => mocks.livePreviewBegin(...args) as unknown,
    livePreviewUpdate: (...args: unknown[]) => mocks.livePreviewUpdate(...args) as unknown,
    livePreviewEnd: (...args: unknown[]) => mocks.livePreviewEnd(...args) as unknown,
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => mocks.modules,
  useAppContext: () => mocks.context,
}))
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() },
}))

const mk2 = DEVICE_MODELS.mk2
const text = { type: "text", multiline: false } as const

const deckTypes: ActionType[] = [
  {
    id: "deck.next-page",
    name: "Next page",
    description: "",
    category: "Deck",
    requiresProject: false,
    parameters: [
      {
        key: "wrapAround",
        label: "Wrap",
        kind: { type: "boolean" },
        required: false,
        default: { type: "boolean", value: true },
      },
    ],
    editorNavigation: { operation: "page.next" },
  },
  {
    id: "deck.previous-page",
    name: "Previous page",
    description: "",
    category: "Deck",
    requiresProject: false,
    parameters: [
      {
        key: "wrapAround",
        label: "Wrap",
        kind: { type: "boolean" },
        required: false,
        default: { type: "boolean", value: true },
      },
    ],
    editorNavigation: { operation: "page.previous" },
  },
  {
    id: "deck.switch-layout",
    name: "Switch layout",
    description: "",
    category: "Deck",
    requiresProject: false,
    parameters: [
      { key: "layout", label: "Layout", kind: text, required: true },
      { key: "page", label: "Page", kind: text, required: false },
    ],
    editorNavigation: {
      operation: "layout.switch",
      targetParameter: "layout",
      pageParameter: "page",
    },
  },
  {
    id: "deck.previous-layout",
    name: "Back to previous layout",
    description: "",
    category: "Deck",
    requiresProject: false,
    parameters: [],
    editorNavigation: { operation: "layout.back" },
  },
  {
    id: "shell.run",
    name: "Run command",
    description: "",
    category: "System",
    requiresProject: false,
    parameters: [{ key: "command", label: "Command", kind: text, required: true }],
  },
]

/** A plugin's actions: one declares a valid navigation, one an invalid one. */
const pluginModule = {
  id: "acme",
  version: "1.0.0",
  name: "Acme",
  description: "",
  platforms: [],
  pages: [],
  capabilities: [],
  config: [],
  dependencies: [],
  enabledByDefault: true,
  enabled: false,
  supported: true,
  configValues: {},
  actionTypes: [
    {
      id: "acme.jump",
      name: "Jump",
      description: "",
      category: "Acme",
      requiresProject: false,
      parameters: [{ key: "where", label: "Where", kind: text, required: true }],
      editorNavigation: { operation: "folder.open", targetParameter: "where" },
    },
    {
      id: "acme.broken",
      name: "Broken",
      description: "",
      category: "Acme",
      requiresProject: false,
      // `count` is no text parameter: the declaration does not check out.
      parameters: [{ key: "count", label: "Count", kind: { type: "integer" }, required: true }],
      editorNavigation: { operation: "folder.open", targetParameter: "count" },
    },
    {
      id: "acme.teleport",
      name: "Teleport",
      description: "",
      category: "Acme",
      requiresProject: false,
      parameters: [],
      // Not an operation of the allowlist.
      editorNavigation: { operation: "system.shutdown" },
    },
  ],
}

const key = (index: number) => ({ kind: "key" as const, index })
const press = { type: "press" as const }

const main: DeviceLayout = {
  id: "main",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  explicitTree: true,
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        { control: key(0), trigger: press, target: { type: "openPage", page: "tools" } },
        { control: key(1), trigger: press, target: { type: "action", action: "next" } },
        { control: key(2), trigger: press, target: { type: "action", action: "prev" } },
        { control: key(3), trigger: press, target: { type: "action", action: "switch" } },
        { control: key(4), trigger: press, target: { type: "action", action: "run" } },
        { control: key(5), trigger: press, target: { type: "action", action: "jump" } },
        { control: key(6), trigger: press, target: { type: "action", action: "broken" } },
        { control: key(7), trigger: press, target: { type: "action", action: "teleport" } },
        // The press binding decides: the long press would open a folder.
        { control: key(8), trigger: press, target: { type: "action", action: "next" } },
        {
          control: key(8),
          trigger: { type: "longPress" },
          target: { type: "openPage", page: "tools" },
        },
      ],
    },
    {
      id: "tools",
      name: "Tools",
      parent: "home",
      bindings: [
        { control: key(0), trigger: press, target: { type: "back" } },
        { control: key(1), trigger: press, target: { type: "openPage", page: "docker" } },
      ],
    },
    // No Back key of its own: the agent adds one on key 1 (the preview face).
    { id: "docker", name: "Docker", parent: "tools", bindings: [] },
    {
      id: "second",
      name: "Second",
      bindings: [{ control: key(0), trigger: press, target: { type: "action", action: "lback" } }],
    },
    { id: "third", name: "Third", bindings: [] },
  ],
}

const other: DeviceLayout = {
  id: "other",
  name: "Other",
  model: mk2.id,
  homePage: "o1",
  pages: [
    { id: "o1", name: "Other home", bindings: [] },
    {
      id: "o2",
      name: "Other two",
      bindings: [{ control: key(0), trigger: press, target: { type: "action", action: "lback" } }],
    },
  ],
}

const action = (id: string, actionType: string, parameters = {}) => ({
  id,
  name: id,
  actionType,
  parameters,
  icon: null,
})

const view = {
  activeProjectId: null,
  actionUsages: {},
  profile: {
    id: "p",
    name: "Work",
    description: "",
    actions: [
      action("next", "deck.next-page", { wrapAround: { type: "boolean", value: false } }),
      action("prev", "deck.previous-page", { wrapAround: { type: "boolean", value: true } }),
      action("switch", "deck.switch-layout", {
        layout: { type: "text", value: "other" },
        page: { type: "text", value: "o2" },
      }),
      action("lback", "deck.previous-layout"),
      action("run", "shell.run", { command: { type: "text", value: "rm -rf build" } }),
      action("jump", "acme.jump", { where: { type: "text", value: "docker" } }),
      action("broken", "acme.broken", { count: { type: "integer", value: 3 } }),
      action("teleport", "acme.teleport"),
    ],
    workflows: [],
    projects: [],
    layouts: [main, other],
  },
} as unknown as ProfileView

function agent(change: Partial<AgentConnection> = {}): AgentConnection {
  return {
    running: true,
    status: null,
    canStart: true,
    compatibility: { type: "compatible" },
    appVersion: "0.9.0",
    protocol: 14,
    capabilities: [],
    ...change,
  }
}

const onDevice = (change: Partial<DeviceInfo> = {}) =>
  deviceInfo({
    layout: { type: "compatible", layout: "main", name: "Main" },
    page: "Home",
    pageId: "home",
    ...change,
  })

function shellSelection(places: Map<string, EditorPlace>): EditorSelection {
  let dirty = false
  return {
    models: { status: "ready", data: [mk2] },
    modelId: mk2.id,
    device: null,
    layoutId: "main",
    selectDevice: vi.fn(() => true),
    selectProfile: vi.fn(() => true),
    selectLayout: vi.fn(() => !dirty),
    showLayout: vi.fn(),
    adoptModel: vi.fn(),
    setDirty: vi.fn((value: boolean) => {
      dirty = value
    }),
    guard: vi.fn(() => !dirty),
    isDirty: vi.fn(() => dirty),
    place: (k) => places.get(k),
    rememberPlace: (k, place) => {
      places.set(k, place)
    },
  }
}

function renderEditor({
  device = onDevice(),
  saved = main,
}: { device?: DeviceInfo | null; saved?: DeviceLayout } = {}) {
  const user = userEvent.setup()
  const places = new Map<string, EditorPlace>()
  const shell = shellSelection(places)
  const onDirtyChange = (dirty: boolean) => {
    shell.setDirty(dirty)
  }
  const ui = (layout: DeviceLayout) => (
    <DialogProvider>
      <EditorSelectionContext value={shell}>
        <LayoutEditor
          key={layout.id}
          profileId="p"
          view={view}
          model={mk2}
          saved={layout}
          device={device}
          onDirtyChange={onDirtyChange}
          onSaved={vi.fn()}
        />
      </EditorSelectionContext>
    </DialogProvider>
  )
  const result = render(ui(saved))
  return {
    user,
    shell,
    places,
    rerender: (layout: DeviceLayout) => {
      result.rerender(ui(layout))
    },
  }
}

const control = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => control(name).getAttribute("title")
/** Lets the debounced preview faces arrive (use-preview-faces.ts). */
const faces = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 200))
  })

async function openMenu(name: string) {
  fireEvent.contextMenu(control(name))
  return screen.findByRole("menu")
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetLayoutHistory()
  mocks.types = deckTypes
  mocks.modules = [pluginModule]
  mocks.context.profiles.activeProfileId = "p"
  mocks.context.agent = agent()
  mocks.simulatePress.mockReset()
  mocks.simulatePress.mockResolvedValue({
    status: "ran",
    dispatched: ["press", "release"],
  } satisfies SimulatePressResult)
  mocks.runAction.mockReset()
  resetBackend()
  mocks.livePreviewBegin.mockReset()
  mocks.livePreviewBegin.mockResolvedValue(1)
  mocks.livePreviewUpdate.mockReset()
  mocks.livePreviewUpdate.mockResolvedValue(true)
  mocks.livePreviewEnd.mockReset()
  mocks.livePreviewEnd.mockResolvedValue(null)
  mocks.saveLayout.mockReset()
  mocks.saveLayout.mockImplementation((_p: string, layout: DeviceLayout) =>
    Promise.resolve({ ...layout, agentReload: { state: "applied" } }),
  )
  mocks.previewLayout.mockReset()
  mocks.previewLayout.mockImplementation(
    (_profile: string, _layout: DeviceLayout, page: string): Promise<ControlFace[]> =>
      Promise.resolve(
        page === "docker"
          ? [
              {
                control: key(0),
                face: { style: "action", title: "Back" },
                navigation: { operation: "folder.back", automatic: true },
              },
            ]
          : [],
      ),
  )
  vi.mocked(toast.success).mockClear()
  vi.mocked(toast.error).mockClear()
  vi.mocked(toast.warning).mockClear()
})
afterEach(async () => {
  cleanup()
  await act(async () => {
    await i18n.changeLanguage("en")
  })
})

describe("double-click on the preview: editor navigation only", { timeout: 20_000 }, () => {
  it("selects with a single click and never runs or simulates anything", async () => {
    const { user } = renderEditor()
    await user.click(control("Key 5"))
    expect(control("Key 5")).toHaveAttribute("aria-pressed", "true")
    expect(currentPage()).toBe("Home")
    await user.click(control("Key 1"))
    expect(currentPage()).toBe("Home")
    expect(mocks.simulatePress).not.toHaveBeenCalled()
    expect(mocks.runAction).not.toHaveBeenCalled()
  })

  it("opens a folder, nested folders, and goes back with Back keys, the automatic one too", async () => {
    const { user } = renderEditor()
    await user.dblClick(control("Key 1"))
    expect(currentPage()).toBe("Tools")
    expect(crumbNames()).toEqual(["Home", "Tools"])
    await user.dblClick(control("Key 2"))
    expect(currentPage()).toBe("Docker")
    expect(crumbNames()).toEqual(["Home", "Tools", "Docker"])
    // The automatic Back key is no binding: only its preview face says so.
    await faces()
    await user.dblClick(control("Key 1"))
    expect(currentPage()).toBe("Tools")
    // The explicit Back binding.
    await user.dblClick(control("Key 1"))
    expect(currentPage()).toBe("Home")
    expect(crumbNames()).toEqual(["Home"])
    expect(mocks.simulatePress).not.toHaveBeenCalled()
  })

  it("does nothing for Back at the top and for keys that do not navigate", async () => {
    const { user } = renderEditor()
    // Key 5 runs a command: nothing happens, no message.
    await user.dblClick(control("Key 5"))
    expect(currentPage()).toBe("Home")
    expect(toast.error).not.toHaveBeenCalled()
    expect(toast.warning).not.toHaveBeenCalled()
    // An empty key.
    await user.dblClick(control("Key 15"))
    expect(currentPage()).toBe("Home")
    expect(mocks.simulatePress).not.toHaveBeenCalled()
    expect(mocks.runAction).not.toHaveBeenCalled()
  })

  it("turns pages with the deck's page actions, honouring wrapAround", async () => {
    const { user } = renderEditor()
    // Previous page with wraparound: from the first to the last.
    await user.dblClick(control("Key 3"))
    expect(currentPage()).toBe("Third")
    await user.dblClick(control("Key 3")) // empty on Third: nothing
    expect(currentPage()).toBe("Third")
    await user.click(screen.getByRole("button", { name: "Previous page" }))
    await user.click(screen.getByRole("button", { name: "Previous page" }))
    expect(currentPage()).toBe("Home")
    // Next page without wraparound, from Home: Second (folders are skipped).
    await user.dblClick(control("Key 2"))
    expect(currentPage()).toBe("Second")
    // The press binding of a key decides, not its long press (a folder).
    await user.click(screen.getByRole("button", { name: "Previous page" }))
    await user.dblClick(control("Key 9"))
    expect(currentPage()).toBe("Second")
  })

  it("stops at the last page without wraparound", async () => {
    const layout: DeviceLayout = {
      ...main,
      pages: main.pages.map((p) =>
        p.id === "third"
          ? {
              ...p,
              bindings: [
                { control: key(0), trigger: press, target: { type: "action", action: "next" } },
              ],
            }
          : p,
      ),
    }
    const { user } = renderEditor({ saved: layout })
    await user.click(screen.getByRole("button", { name: "Next page" }))
    await user.click(screen.getByRole("button", { name: "Next page" }))
    expect(currentPage()).toBe("Third")
    await user.dblClick(control("Key 1"))
    expect(currentPage()).toBe("Third")
  })

  it("navigates for a plugin action with a valid declaration only", async () => {
    const { user } = renderEditor()
    await user.dblClick(control("Key 7"))
    expect(currentPage()).toBe("Home")
    await user.dblClick(control("Key 8"))
    expect(currentPage()).toBe("Home")
    await user.dblClick(control("Key 6"))
    expect(currentPage()).toBe("Docker")
    expect(crumbNames()).toEqual(["Home", "Docker"])
  })

  it("switches the layout through the shell, to its page, and back again", async () => {
    const { user, shell, places, rerender } = renderEditor()
    await user.dblClick(control("Key 4"))
    expect(shell.selectLayout).toHaveBeenCalledWith("other")
    // Opens at the page the key names, with no folder trail.
    expect(places.get("p:other")).toEqual({
      pageId: "o2",
      trail: [],
      selection: { controls: [], anchor: null, focus: null },
    })
    rerender(other)
    expect(currentPage()).toBe("Other two")
    expect(crumbNames()).toEqual(["Other two"])
  })

  it("goes back to the layout the editor showed before, else does nothing", async () => {
    const { user, shell, rerender } = renderEditor()
    await user.click(screen.getByRole("button", { name: "Next page" }))
    // Nothing to go back to yet.
    await user.dblClick(control("Key 1"))
    expect(shell.selectLayout).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Previous page" }))
    await user.dblClick(control("Key 4"))
    expect(shell.selectLayout).toHaveBeenLastCalledWith("other")
    rerender(other)
    // Back to previous layout: the layout the editor came from.
    await user.dblClick(control("Key 1"))
    expect(shell.selectLayout).toHaveBeenLastCalledWith("main")
    expect(shell.selectLayout).toHaveBeenCalledTimes(2)
    rerender(main)
    // Taken: nothing more to go back to.
    await user.click(screen.getByRole("button", { name: "Next page" }))
    await user.dblClick(control("Key 1"))
    expect(shell.selectLayout).toHaveBeenCalledTimes(2)
  })

  it("asks before a layout switch discards unsaved changes", async () => {
    const { user, shell } = renderEditor()
    await user.click(control("Key 3"))
    fireEvent.keyDown(control("Key 3"), { key: "Delete" })
    expect(screen.getByText("Unsaved")).toBeInTheDocument()
    await user.dblClick(control("Key 4"))
    const dialog = await screen.findByRole("alertdialog", { name: "Discard unsaved changes?" })
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(shell.selectLayout).not.toHaveBeenCalled()
    await user.dblClick(control("Key 4"))
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: "Discard changes",
      }),
    )
    await waitFor(() => {
      expect(shell.selectLayout).toHaveBeenCalledWith("other")
    })
    expect(shell.setDirty).toHaveBeenLastCalledWith(false)
  })
})

describe("the control's context menu", { timeout: 20_000 }, () => {
  it("offers testing, configuring and removing, with folder navigation", async () => {
    const { user } = renderEditor()
    const menu = await openMenu("Key 5")
    for (const name of [/Test action/, /Configure action/, /Remove assignment/, /Assign/]) {
      expect(within(menu).getByRole("menuitem", { name })).toBeInTheDocument()
    }
    await user.keyboard("{Escape}")

    // A Back key offers Back; a folder key opens the folder.
    await user.dblClick(control("Key 1"))
    const inFolder = await openMenu("Key 1")
    await user.click(within(inFolder).getByRole("menuitem", { name: "Go back" }))
    expect(currentPage()).toBe("Home")
  })

  it("says why an item is disabled", async () => {
    const { user } = renderEditor()
    const menu = await openMenu("Key 15")
    for (const name of [/Test action/, /Configure action/, /Remove assignment/]) {
      const item = within(menu).getByRole("menuitem", { name })
      expect(item).toHaveAttribute("data-disabled")
      expect(item).toHaveTextContent("No action assigned")
    }
    await user.keyboard("{Escape}")
  })

  it.each([
    ["no device", { device: null }, "Device not connected"],
    [
      "another page",
      { device: onDevice({ page: "Second", pageId: "second" }) },
      "The device shows another page: switch it first",
    ],
    [
      "another layout",
      { device: onDevice({ layout: { type: "compatible", layout: "other", name: "Other" } }) },
      "The device shows another layout: switch it first",
    ],
  ] as const)("disables Test action with %s", async (_, options, reason) => {
    renderEditor(options)
    const item = within(await openMenu("Key 5")).getByRole("menuitem", { name: /Test action/ })
    expect(item).toHaveAttribute("data-disabled")
    expect(item).toHaveTextContent(reason)
  })

  it("disables Test action for another profile, an old agent or none", async () => {
    mocks.context.profiles.activeProfileId = "other-profile"
    const { user } = renderEditor()
    let item = within(await openMenu("Key 5")).getByRole("menuitem", { name: /Test action/ })
    expect(item).toHaveTextContent("The device runs another profile: activate this one first")
    await user.keyboard("{Escape}")
    cleanup()

    mocks.context.profiles.activeProfileId = "p"
    mocks.context.agent = agent({ protocol: 13 })
    renderEditor()
    item = within(await openMenu("Key 5")).getByRole("menuitem", { name: /Test action/ })
    expect(item).toHaveTextContent("The agent is too old for simulation: update it")
    cleanup()

    mocks.context.agent = agent({ running: false })
    renderEditor()
    item = within(await openMenu("Key 5")).getByRole("menuitem", { name: /Test action/ })
    expect(item).toHaveTextContent("The DeckPilot agent is not running")
    expect(mocks.simulatePress).not.toHaveBeenCalled()
  })

  it("opens from the keyboard on a focused control", async () => {
    const { user } = renderEditor()
    control("Key 5").focus()
    fireEvent.keyDown(control("Key 5"), { key: "F10", shiftKey: true })
    expect(await screen.findByRole("menu")).toBeInTheDocument()
    await user.keyboard("{Escape}")
    await waitFor(() => {
      expect(screen.queryByRole("menu")).not.toBeInTheDocument()
    })
    fireEvent.keyDown(control("Key 5"), { key: "ContextMenu" })
    expect(await screen.findByRole("menu")).toBeInTheDocument()
  })

  it("configures: selects the control alone and focuses the inspector", async () => {
    const { user } = renderEditor()
    const menu = await openMenu("Key 5")
    await user.click(within(menu).getByRole("menuitem", { name: /Configure action/ }))
    expect(control("Key 5")).toHaveAttribute("aria-pressed", "true")
    await waitFor(() => {
      const inspector = screen.getByRole("region", { name: "Key 5 settings" })
      expect(inspector.contains(document.activeElement)).toBe(true)
    })
  })

  it("asks before removing an assignment", async () => {
    const { user } = renderEditor()
    let menu = await openMenu("Key 5")
    await user.click(within(menu).getByRole("menuitem", { name: /Remove assignment/ }))
    let dialog = await screen.findByRole("alertdialog", { name: "Remove the assignment of Key 5?" })
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(describes("Key 5")).toBe("Key 5: Press: run")
    menu = await openMenu("Key 5")
    await user.click(within(menu).getByRole("menuitem", { name: /Remove assignment/ }))
    dialog = await screen.findByRole("alertdialog")
    await user.click(within(dialog).getByRole("button", { name: "Clear" }))
    await waitFor(() => {
      expect(describes("Key 5")).toBe("Key 5")
    })
  })
})

describe("Test action: simulated presses", { timeout: 20_000 }, () => {
  async function test(user: ReturnType<typeof userEvent.setup>, name: string, kind: string) {
    const menu = await openMenu(name)
    await user.click(within(menu).getByRole("menuitem", { name: /Test action/ }))
    // Radix submenus: a pointer moving there would close them in jsdom.
    fireEvent.click(await screen.findByRole("menuitem", { name: kind }))
  }

  it.each([
    ["Single press", "single", "Simulated single press on Key 5"],
    ["Double press", "double", "Simulated double press on Key 5"],
    ["Long press", "long", "Simulated long press on Key 5"],
  ] as const)("sends a %s with the editor's identity", async (label, kind, message) => {
    const { user } = renderEditor()
    await test(user, "Key 5", label)
    await waitFor(() => {
      expect(mocks.simulatePress).toHaveBeenCalledWith(
        { model: "elgato.streamdeck.mk2", serial: "SIM-MK2-01" },
        "p",
        "main",
        "home",
        { kind: "key", index: 4 },
        kind,
        false,
      )
    })
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith(message, {
        description: "A simulation: the agent pressed the key on the device, not the editor.",
      })
    })
    // A simulation is no editor navigation.
    expect(currentPage()).toBe("Home")
    expect(mocks.runAction).not.toHaveBeenCalled()
  })

  it("never simulates on a double-click", async () => {
    const { user } = renderEditor()
    await user.dblClick(control("Key 5"))
    await user.dblClick(control("Key 2"))
    expect(mocks.simulatePress).not.toHaveBeenCalled()
  })

  it("asks when the press may change something and sends it again confirmed", async () => {
    mocks.simulatePress.mockResolvedValueOnce({
      status: "confirmationRequired",
      actionNames: ["Run command", "Deploy"],
      reason: "These actions change things outside DeckPilot.",
    })
    const { user } = renderEditor()
    await test(user, "Key 5", "Single press")
    const dialog = await screen.findByRole("alertdialog", {
      name: "Simulate a press on Key 5?",
    })
    expect(dialog).toHaveTextContent("These actions change things outside DeckPilot.")
    expect(
      within(dialog)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["Run command", "Deploy"])
    await user.click(within(dialog).getByRole("button", { name: "Run simulation" }))
    await waitFor(() => {
      expect(mocks.simulatePress).toHaveBeenCalledTimes(2)
    })
    expect(mocks.simulatePress.mock.lastCall?.[6]).toBe(true)
  })

  it("sends nothing more when the confirmation is cancelled", async () => {
    mocks.simulatePress.mockResolvedValueOnce({
      status: "confirmationRequired",
      actionNames: ["Run command"],
      reason: "Changes things.",
    })
    const { user } = renderEditor()
    await test(user, "Key 5", "Double press")
    const dialog = await screen.findByRole("alertdialog")
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(mocks.simulatePress).toHaveBeenCalledTimes(1)
  })

  it.each([
    ["locked", "DeckPilot is locked. Unlock it and try again."],
    ["wrongProfile", "The device runs another profile. Activate this profile first."],
    ["wrongLayout", "The device shows another layout. Switch it to this layout first."],
    ["wrongPage", "The device shows another page. Switch it to this page first."],
    ["deviceNotConnected", "The device is not connected."],
    ["unknownControl", "This control cannot be pressed on the device."],
    ["unsupported", "The agent is too old for simulated presses. Update it."],
    ["agentNotRunning", "The DeckPilot agent is not running."],
    ["failed", "The agent could not simulate the press."],
  ] as const)("explains the refusal %s", async (code, message) => {
    mocks.simulatePress.mockResolvedValueOnce({ status: "refused", code, message: "raw" })
    const { user } = renderEditor()
    await test(user, "Key 5", "Single press")
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("Key 5 was not pressed", { description: message })
    })
  })

  it("names a settings gate", async () => {
    mocks.simulatePress.mockResolvedValueOnce({
      status: "refused",
      code: "failed",
      message: "commands-disabled",
    })
    const { user } = renderEditor()
    await test(user, "Key 5", "Single press")
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("Key 5 was not pressed", {
        description: expect.stringContaining("does not allow it") as unknown,
      })
    })
  })

  it("warns that the device runs the saved layout, then saves or tests the saved one", async () => {
    const { user } = renderEditor()
    await user.click(control("Key 3"))
    fireEvent.keyDown(control("Key 3"), { key: "Delete" })
    await test(user, "Key 5", "Single press")
    let dialog = await screen.findByRole("dialog", { name: "The layout has unsaved changes" })
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(mocks.simulatePress).not.toHaveBeenCalled()

    await test(user, "Key 5", "Single press")
    dialog = await screen.findByRole("dialog", { name: "The layout has unsaved changes" })
    await user.click(within(dialog).getByRole("radio", { name: /Test the saved version/ }))
    await user.click(within(dialog).getByRole("button", { name: "Continue" }))
    await waitFor(() => {
      expect(mocks.simulatePress).toHaveBeenCalledTimes(1)
    })
    expect(mocks.saveLayout).not.toHaveBeenCalled()

    await test(user, "Key 5", "Single press")
    dialog = await screen.findByRole("dialog", { name: "The layout has unsaved changes" })
    await user.click(within(dialog).getByRole("button", { name: "Continue" }))
    await waitFor(() => {
      expect(mocks.simulatePress).toHaveBeenCalledTimes(2)
    })
    expect(mocks.saveLayout).toHaveBeenCalledTimes(1)
  })

  it("uses the editor's profile, never the one on the device", async () => {
    mocks.context.profiles.activeProfileId = "other-profile"
    const { user } = renderEditor()
    const menu = await openMenu("Key 5")
    const item = within(menu).getByRole("menuitem", { name: /Test action/ })
    expect(item).toHaveAttribute("data-disabled")
    await user.click(item)
    expect(screen.queryByRole("menuitem", { name: "Single press" })).not.toBeInTheDocument()
    expect(mocks.simulatePress).not.toHaveBeenCalled()
  })

  it("is in Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    const { user } = renderEditor()
    const menu = await openMenu("Klawisz 5")
    for (const name of [/Testuj akcję/, /Konfiguruj akcję/, /Usuń przypisanie/]) {
      expect(within(menu).getByRole("menuitem", { name })).toBeInTheDocument()
    }
    await user.click(within(menu).getByRole("menuitem", { name: /Testuj akcję/ }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Podwójne naciśnięcie" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith(
        "Zasymulowano podwójne naciśnięcie: Klawisz 5",
        expect.anything(),
      )
    })
    await user.keyboard("{Escape}")
    const empty = within(await openMenu("Klawisz 15")).getByRole("menuitem", {
      name: /Testuj akcję/,
    })
    expect(empty).toHaveTextContent("Brak przypisanej akcji")
  })
})

describe("breadcrumbs of the page navigation", { timeout: 20_000 }, () => {
  it("goes to an ancestor with the trail kept, without a Back button", async () => {
    const { user } = renderEditor()
    await user.dblClick(control("Key 1"))
    await user.dblClick(control("Key 2"))
    expect(crumbNames()).toEqual(["Home", "Tools", "Docker"])
    expect(screen.queryByRole("button", { name: "Back" })).not.toBeInTheDocument()
    expect(screen.getByText("Docker", { selector: "[aria-current=page]" })).toBeInTheDocument()
    await upTo(user, "Tools")
    expect(currentPage()).toBe("Tools")
    expect(crumbNames()).toEqual(["Home", "Tools"])
    // Back from there follows the trail kept: to Home.
    await user.dblClick(control("Key 1"))
    expect(currentPage()).toBe("Home")
    // The new place is announced.
    expect(document.querySelector("[data-slot=page-announcement]")).toHaveTextContent(
      "Now showing Home",
    )
  })

  it("collapses the middle levels into a menu when they do not fit", async () => {
    // Every crumb reports it is cut (a narrow window): the middle collapses.
    const width = vi
      .spyOn(HTMLElement.prototype, "scrollWidth", "get")
      .mockImplementation(function (this: HTMLElement) {
        return this.hasAttribute("data-crumb") ? 200 : 0
      })
    try {
      const { user } = renderEditor()
      await user.dblClick(control("Key 1"))
      await user.dblClick(control("Key 2"))
      expect(crumbNames()).toEqual(["Home", "Docker"])
      const more = screen.getByRole("button", { name: "Hidden levels: 1" })
      await user.click(more)
      await user.click(await screen.findByRole("menuitem", { name: "Tools" }))
      expect(currentPage()).toBe("Tools")
    } finally {
      width.mockRestore()
    }
  })

  it("is labelled in Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    const { user } = renderEditor()
    await user.dblClick(control("Klawisz 1"))
    const nav = screen.getByRole("navigation", { name: "Ścieżka nawigacji" })
    expect(within(nav).getByRole("button", { name: "Home" })).toBeInTheDocument()
  })
})

describe("live preview: the device shows the draft", { timeout: 20_000 }, () => {
  /** An agent of protocol 16 that runs the edited profile. */
  function liveAgent(): AgentConnection {
    return agent({
      protocol: 16,
      capabilities: ["livePreview"],
      status: agentStatus({ protocol: 16, activeProfile: { id: "p", name: "Work" } }),
    })
  }

  const shown = (stack: string[], seq: number, change: Record<string, unknown> = {}) => ({
    profile: "p",
    layout: "main",
    stack,
    connected: true,
    inScope: true,
    seq,
    ...change,
  })

  function emit(update: Record<string, unknown>) {
    act(() => {
      emitBackend(LIVE_PREVIEW_EVENT, { session: 1, ...update })
    })
  }

  async function started() {
    await waitFor(() => {
      expect(mocks.livePreviewBegin).toHaveBeenCalledWith(onDevice().id, "p", "main")
    })
    emit({ kind: "started", navigation: shown(["home"], 1), resumed: false })
  }

  const navigations = () =>
    mocks.livePreviewUpdate.mock.calls.flatMap(([, update]) => {
      const { navigation } = update as { navigation?: unknown }
      return navigation ? [navigation] : []
    })

  beforeEach(() => {
    mocks.context.agent = liveAgent()
  })

  it("sends the draft and the editor's navigation, and follows the device", async () => {
    const { user } = renderEditor()
    await started()
    expect(screen.getByText("Live on the device")).toBeInTheDocument()
    // The whole draft once.
    expect(mocks.livePreviewUpdate).toHaveBeenCalledWith(1, { revision: 1, layout: main })
    // A double-click opens the folder in the editor and on the device.
    await user.dblClick(control("Key 1"))
    expect(currentPage()).toBe("Tools")
    expect(navigations()).toEqual([{ type: "show", layout: "main", stack: ["home", "tools"] }])
    emit({ kind: "navigated", navigation: shown(["home", "tools"], 2), own: true })
    // The device's own navigation (its Back key): the editor follows.
    emit({ kind: "navigated", navigation: shown(["home"], 3), own: false })
    expect(currentPage()).toBe("Home")
    // ...without sending it back.
    expect(navigations()).toHaveLength(1)
    // Selecting never sends anything to run.
    await user.click(control("Key 5"))
    expect(navigations()).toHaveLength(1)
    expect(mocks.simulatePress).not.toHaveBeenCalled()
    expect(mocks.runAction).not.toHaveBeenCalled()
  })

  it("follows a layout switch on the device unless something is unsaved", async () => {
    const { shell } = renderEditor()
    await started()
    emit({
      kind: "navigated",
      navigation: shown(["o2"], 2, { layout: "other", inScope: false }),
      own: false,
    })
    expect(shell.selectLayout).toHaveBeenCalledWith("other")
    expect(shell.place("p:other")?.pageId).toBe("o2")
  })

  it("stays with unsaved changes when the device switches layouts", async () => {
    const { shell } = renderEditor()
    await started()
    await userEvent.setup().click(control("Key 4"))
    fireEvent.keyDown(control("Key 4"), { key: "Delete" })
    expect(screen.getByText("Unsaved")).toBeInTheDocument()
    emit({
      kind: "navigated",
      navigation: shown(["o2"], 2, { layout: "other", inScope: false }),
      own: false,
    })
    expect(shell.selectLayout).not.toHaveBeenCalled()
    expect(screen.getByText("Device shows another layout")).toBeInTheDocument()
  })

  it("tests the draft on the device without asking about unsaved changes", async () => {
    const user = userEvent.setup()
    renderEditor()
    await started()
    await user.click(control("Key 4"))
    fireEvent.keyDown(control("Key 4"), { key: "Delete" })
    expect(screen.getByText("Unsaved")).toBeInTheDocument()
    const menu = await openMenu("Key 5")
    await user.click(within(menu).getByRole("menuitem", { name: /Test action/ }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Single press" }))
    await waitFor(() => {
      expect(mocks.simulatePress).toHaveBeenCalled()
    })
    expect(screen.queryByRole("dialog", { name: /unsaved/i })).not.toBeInTheDocument()
  })

  it("sends the latest draft before saving it", async () => {
    const user = userEvent.setup()
    renderEditor()
    await started()
    await user.click(control("Key 4"))
    fireEvent.keyDown(control("Key 4"), { key: "Delete" })
    await user.click(screen.getByRole("button", { name: /^Save/ }))
    await waitFor(() => {
      expect(mocks.saveLayout).toHaveBeenCalled()
    })
    const lastDraft = mocks.livePreviewUpdate.mock.calls
      .map(([, update]) => (update as { layout?: DeviceLayout }).layout)
      .filter(Boolean)
      .at(-1)
    expect(lastDraft).toEqual(mocks.saveLayout.mock.lastCall?.[1])
    const draftOrder = mocks.livePreviewUpdate.mock.invocationCallOrder.at(-1) ?? 0
    expect(draftOrder).toBeLessThan(mocks.saveLayout.mock.invocationCallOrder[0] ?? 0)
  })

  it("ends the session when the editor goes", async () => {
    renderEditor()
    await started()
    cleanup()
    expect(mocks.livePreviewEnd).toHaveBeenCalledWith(1)
  })
})
