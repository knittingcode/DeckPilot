// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { gestureSlotId } from "@/components/gesture-editor/model"
import type { ActionInput } from "@/lib/api"
import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionType,
  ControlId,
  DeviceLayout,
  DeviceModel,
  ModuleInfo,
  ProfileView,
  Workflow,
} from "@/lib/types"

import { LayoutEditor } from "./editor"
import { NEW_MULTI_ACTION_DRAG_ID } from "./library-panel"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const delayType: ActionType = {
  id: "general.delay",
  name: "Delay",
  description: "Waits a number of milliseconds between steps.",
  category: "General",
  requiresProject: false,
  parameters: [
    {
      key: "milliseconds",
      label: "Milliseconds",
      kind: { type: "integer", min: 10, max: 300000 },
      required: true,
      default: { type: "integer", value: 500 },
    },
  ],
}
const logType: ActionType = {
  id: "general.log",
  name: "Log message",
  description: "Writes a message to the execution log.",
  category: "General",
  requiresProject: false,
  parameters: [],
}
const modules: ModuleInfo[] = [
  moduleInfo({ id: "general", name: "General", actionTypes: [logType, delayType] }),
]

const createAction = vi.fn()
const saveLayout = vi.fn()
const saveWorkflow = vi.fn()
const startWorkflow = vi.fn()

/** The editor's drag and drop as it is wired (see `key-logic.test.tsx`). */
const dnd = vi.hoisted(() => ({
  contexts: new Map<string, ((event: DragEndEvent) => void) | undefined>(),
  monitors: new Set<{ onDragEnd?: (event: DragEndEvent) => void }>(),
  data: new Map<string, unknown>(),
}))

vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  const react = await import("react")
  return {
    ...actual,
    DndContext: (props: { onDragEnd?: (event: DragEndEvent) => void; children?: ReactNode }) => {
      // The editor and the step list each have one; a drop goes to every
      // context, as each ignores what is not its own.
      const id = react.useId()
      dnd.contexts.set(id, props.onDragEnd)
      react.useEffect(
        () => () => {
          dnd.contexts.delete(id)
        },
        [id],
      )
      return <actual.DndContext {...props} />
    },
    useDraggable: (args: Parameters<typeof actual.useDraggable>[0]) => {
      dnd.data.set(String(args.id), args.data)
      return actual.useDraggable(args)
    },
    useDndMonitor: (listener: { onDragEnd?: (event: DragEndEvent) => void }) => {
      const current = react.useRef(listener)
      current.current = listener
      react.useEffect(() => {
        const entry = { onDragEnd: (event: DragEndEvent) => current.current.onDragEnd?.(event) }
        dnd.monitors.add(entry)
        return () => {
          dnd.monitors.delete(entry)
        }
      }, [])
      actual.useDndMonitor(listener)
    },
  }
})
vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([logType, delayType]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => saveLayout(...args) as unknown,
    createAction: (...args: unknown[]) => createAction(...args) as unknown,
    saveWorkflow: (...args: unknown[]) => saveWorkflow(...args) as unknown,
    startWorkflow: (...args: unknown[]) => startWorkflow(...args) as unknown,
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => modules,
  useAppContext: () => ({
    profiles: { activeProfileId: "p", profiles: [] },
    agent: { running: true },
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
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "unverified",
  verification: [],
}
const KEY: ControlId = { kind: "key", index: 2 }

function layoutWith(bindings: DeviceLayout["pages"][number]["bindings"]): DeviceLayout {
  return {
    id: "layout",
    name: "Main",
    model: mk2.id,
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  }
}

/** A Multi Action as 0.6.0-alpha.3 stores it: a step list without a graph. */
const stored: Workflow = {
  id: "morning",
  name: "Morning",
  steps: [
    {
      action: "hello",
      onFailure: "continue",
      condition: { type: "always" },
      overrides: {},
    },
  ],
  projectScope: "execution",
}

function viewWith(workflows: Workflow[], layout: DeviceLayout): ProfileView {
  return {
    activeProjectId: null,
    issues: [],
    actionUsages: {},
    workflowUsages: {},
    profile: {
      id: "p",
      name: "Work",
      actions: [
        { id: "hello", name: "Hello", actionType: "general.log", parameters: {}, icon: null },
      ],
      workflows,
      projects: [],
      layouts: [layout],
    },
  } as unknown as ProfileView
}

function renderEditor(workflows: Workflow[] = [], layout = layoutWith([])) {
  render(
    <LayoutEditor
      profileId="p"
      view={viewWith(workflows, layout)}
      model={mk2}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return userEvent.setup()
}

// The canvas is hidden from the accessibility tree while the steps are open.
const describes = (name: string) =>
  screen.getByRole("button", { name, hidden: true }).getAttribute("title")

function drop(id: string, over: string) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  const event = {
    active: { id, data: { current: dnd.data.get(id) } },
    over: { id: over },
  } as unknown as DragEndEvent
  act(() => {
    for (const handler of dnd.contexts.values()) handler?.(event)
    for (const monitor of dnd.monitors) monitor.onDragEnd?.(event)
  })
}

beforeEach(() => {
  dnd.contexts.clear()
  dnd.data.clear()
  dnd.monitors.clear()
  vi.mocked(toast.error).mockClear()
  saveLayout.mockReset()
  saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  createAction.mockReset()
  createAction.mockImplementation((_profile: string, input: ActionInput) =>
    Promise.resolve({ ...input, id: "delay" }),
  )
  saveWorkflow.mockReset()
  saveWorkflow.mockImplementation((_profile: string, workflow: Workflow) =>
    Promise.resolve({ ...workflow, agentReload: { state: "applied" } }),
  )
  startWorkflow.mockReset()
  startWorkflow.mockResolvedValue({ id: "run" })
})
afterEach(cleanup)

describe("Multi Action in the layout editor", { timeout: 20_000 }, () => {
  it("creates a step workflow on a key and edits its steps and delays", async () => {
    const user = renderEditor()
    drop(NEW_MULTI_ACTION_DRAG_ID, "key:2")

    // The workflow is saved at once, bound, and its steps open.
    const dialog = await screen.findByRole("dialog", { name: "Multi Action" })
    expect(saveWorkflow).toHaveBeenCalledTimes(1)
    const [, created] = saveWorkflow.mock.lastCall as [string, Workflow]
    expect(created).toMatchObject({ name: "Multi Action", steps: [], projectScope: "execution" })
    expect(created.graph).toBeUndefined()
    expect(describes("Key 3")).toBe("Key 3: Press: Multi Action")

    // A step, then a delay row: the delay action is created once.
    await user.click(within(dialog).getByRole("combobox", { name: "Action to add" }))
    await user.click(await screen.findByRole("option", { name: "Hello" }))
    await user.click(within(dialog).getByRole("button", { name: "Add step" }))
    await user.click(within(dialog).getByRole("button", { name: "Add delay" }))
    const ms = await within(dialog).findByRole("spinbutton", {
      name: "Delay of step 2 in milliseconds",
    })
    expect(createAction).toHaveBeenCalledTimes(1)
    expect(createAction.mock.lastCall?.[1]).toMatchObject({
      actionType: "general.delay",
      parameters: { milliseconds: { type: "integer", value: 500 } },
    })
    expect(ms).toHaveValue(500)
    fireEvent.change(ms, { target: { value: "1500" } })

    await user.click(within(dialog).getByRole("button", { name: /Save/ }))
    await waitFor(() => {
      expect(saveWorkflow).toHaveBeenCalledTimes(2)
    })
    const [, saved] = saveWorkflow.mock.lastCall as [string, Workflow]
    expect(saved.id).toBe(created.id)
    expect(saved.steps).toEqual([
      { action: "hello", onFailure: "stop", condition: { type: "always" }, overrides: {} },
      {
        action: "delay",
        onFailure: "stop",
        condition: { type: "always" },
        overrides: { milliseconds: { type: "integer", value: 1500 } },
      },
    ])
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })

    // The layout binds the key to the workflow, as any workflow key.
    await user.click(screen.getByRole("button", { name: /Save/ }))
    const [, layout] = saveLayout.mock.lastCall as [string, DeviceLayout]
    expect(layout.pages[0]?.bindings).toEqual([
      expect.objectContaining({
        control: KEY,
        trigger: { type: "press" },
        target: { type: "workflow", workflow: created.id },
      }),
    ])
  })

  it("opens the steps of an existing step workflow from its key and keeps it as stored", async () => {
    const layout = layoutWith([
      {
        control: KEY,
        trigger: { type: "press" },
        target: { type: "workflow", workflow: "morning" },
        label: null,
      },
    ])
    const user = renderEditor([stored], layout)
    await user.click(screen.getByRole("button", { name: /^Key 3/ }))
    await user.click(screen.getByRole("button", { name: "Edit steps (1)" }))
    const dialog = await screen.findByRole("dialog", { name: "Multi Action" })
    // Its own Run button; nothing of the Workflows page's.
    expect(within(dialog).getByRole("button", { name: "Run" })).toBeEnabled()
    expect(within(dialog).queryByRole("button", { name: /Delete/ })).not.toBeInTheDocument()
    // Nothing changed: Save stays off, so the stored workflow is not rewritten.
    expect(within(dialog).getByRole("button", { name: /Save/ })).toBeDisabled()
    await user.click(within(dialog).getByRole("button", { name: "Run" }))
    expect(startWorkflow).toHaveBeenCalledWith("p", "morning")
    expect(saveWorkflow).not.toHaveBeenCalled()
  })

  it("puts a Multi Action on a gesture slot", async () => {
    const user = renderEditor()
    await user.click(screen.getByRole("button", { name: "Key 3" }))
    drop(NEW_MULTI_ACTION_DRAG_ID, gestureSlotId(KEY, "click"))
    await screen.findByRole("dialog", { name: "Multi Action" })
    expect(saveWorkflow).toHaveBeenCalledTimes(1)
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Press: Multi Action")
    })
  })
})
