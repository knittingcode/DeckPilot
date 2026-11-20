// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { gestureSlotId, releaseSlotId, slotGesture } from "@/components/gesture-editor/model"
import type { ActionInput } from "@/lib/api"
import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionType,
  ControlId,
  DeviceLayout,
  DeviceModel,
  Gesture,
  ModuleInfo,
  ProfileView,
} from "@/lib/types"

import { catalogDragId } from "./catalog"
import { LayoutEditor } from "./editor"
import { NEW_FOLDER_DRAG_ID } from "./library-panel"
import { pageNames } from "@/test/pages"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const nextPage: ActionType = {
  id: "deck.next-page",
  name: "Next page",
  description: "Shows the next page of the layout on the device.",
  category: "Navigation",
  requiresProject: false,
  parameters: [],
}
const modules: ModuleInfo[] = [moduleInfo({ id: "deck", name: "Deck", actionTypes: [nextPage] })]

const createAction = vi.fn()
const saveLayout = vi.fn()

/**
 * The editor's drag and drop as it is wired (see `folders.test.tsx`): the
 * drag data of the real draggables, the editor's drop handler and every
 * listener the components registered for drops.
 */
const dnd = vi.hoisted(() => ({
  onDragEnd: undefined as ((event: DragEndEvent) => void) | undefined,
  monitors: new Set<{ onDragEnd?: (event: DragEndEvent) => void }>(),
  data: new Map<string, unknown>(),
  droppables: new Set<string>(),
}))

vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  const react = await import("react")
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
    useDroppable: (args: Parameters<typeof actual.useDroppable>[0]) => {
      if (args.disabled) dnd.droppables.delete(String(args.id))
      else dnd.droppables.add(String(args.id))
      return actual.useDroppable(args)
    },
    useDndMonitor: (listener: { onDragEnd?: (event: DragEndEvent) => void }) => {
      // The newest listener of the component, as dnd-kit keeps it.
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
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => saveLayout(...args) as unknown,
    createAction: (...args: unknown[]) => createAction(...args) as unknown,
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => modules,
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh: vi.fn() }),
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
  pages: [
    { id: "home", name: "Home", bindings: [] },
    { id: "tools", name: "Tools", bindings: [] },
  ],
}
const view = {
  activeProjectId: null,
  profile: {
    id: "p",
    name: "Work",
    actions: [
      { id: "shop", name: "Shop", actionType: "general.log", parameters: {}, icon: null },
      { id: "mail", name: "Mail", actionType: "general.log", parameters: {}, icon: null },
    ],
    workflows: [{ id: "build", name: "Build", steps: [] }],
    projects: [],
    layouts: [layout],
  },
} as unknown as ProfileView

function renderEditor() {
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

const KEY: ControlId = { kind: "key", index: 2 }
const slot = (gesture: Gesture) => gestureSlotId(KEY, gesture)
/** A gesture's chooser; an empty gesture's is named "Assign an action to: …". */
const row = (name: string) =>
  screen.getByRole("combobox", {
    name: (accessible) => accessible === name || accessible === `Assign an action to: ${name}`,
  })
const describes = (name: string) => screen.getByRole("button", { name }).getAttribute("title")
/** The gestures' slots (not the raw release's). */
const slots = () =>
  [...document.querySelectorAll("[data-gesture-slot]")]
    .filter((element) => !element.getAttribute("aria-labelledby")?.endsWith("-release-label"))
    .map((element) => element.getAttribute("data-gesture-slot"))

/** Drops the draggable `id` on the drop target `over`, as dnd-kit does. */
function drop(id: string, over: string) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  expect(dnd.droppables.has(over), `a drop target ${over}`).toBe(true)
  const event = {
    active: { id, data: { current: dnd.data.get(id) } },
    over: { id: over },
  } as unknown as DragEndEvent
  act(() => {
    dnd.onDragEnd?.(event)
    for (const monitor of dnd.monitors) monitor.onDragEnd?.(event)
  })
}

beforeEach(() => {
  dnd.data.clear()
  dnd.droppables.clear()
  dnd.monitors.clear()
  vi.mocked(toast.error).mockClear()
  saveLayout.mockReset()
  saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  createAction.mockReset()
  createAction.mockImplementation((_profile: string, input: ActionInput) =>
    Promise.resolve({ ...input, id: "next" }),
  )
})
afterEach(cleanup)

describe("gesture slots", () => {
  it("names one slot per gesture of a control", () => {
    expect(slot("click")).toBe("gesture-slot:key:2:click")
    expect(slotGesture(KEY, slot("longPress"))).toBe("longPress")
    // Another control's slot, a key and nothing are not this control's.
    expect(slotGesture({ kind: "key", index: 3 }, slot("click"))).toBeNull()
    expect(slotGesture(KEY, "key:2")).toBeNull()
    expect(slotGesture(KEY, undefined)).toBeNull()
  })
})

describe("key logic in the layout editor", { timeout: 20_000 }, () => {
  it("takes an action, a workflow and an action type in the three slots", async () => {
    const user = renderEditor()
    await user.click(screen.getByRole("button", { name: "Key 3" }))
    expect(slots()).toEqual(["empty", "empty", "empty"])
    // Each empty gesture, and the raw release, invites a drop.
    expect(screen.getAllByText("Drag an action here")).toHaveLength(4)

    // Single press: a saved action from the library. With nothing else on
    // the key it acts as soon as the key is pressed.
    drop("library:action:shop", slot("click"))
    await waitFor(() => {
      expect(row("Single press")).toHaveTextContent("Shop")
    })
    expect(describes("Key 3")).toBe("Key 3: Press: Shop")
    expect(slots()).toEqual(["filled", "empty", "empty"])

    // Double press: a workflow. The key now has to tell the two apart, so
    // the editor asks before the single press starts to wait.
    drop("library:workflow:build", slot("doubleClick"))
    const dialog = await screen.findByRole("alertdialog", { name: "Switch Key 3 to gestures?" })
    expect(dialog).toHaveTextContent("after the double-click window (300 ms)")
    await user.click(within(dialog).getByRole("button", { name: "Switch to gestures" }))
    expect(describes("Key 3")).toBe("Key 3: Click: Shop · Double click: Build")
    expect(screen.getByRole("note")).toHaveTextContent(
      "A single click fires after the double-click window (300 ms) has passed without a second press.",
    )

    // Hold: an action type from the catalog; its action is created.
    drop(catalogDragId("deck.next-page"), slot("longPress"))
    await waitFor(() => {
      expect(describes("Key 3")).toBe(
        "Key 3: Click: Shop · Double click: Build · Long press: Next page",
      )
    })
    expect(createAction).toHaveBeenCalledTimes(1)
    expect(slots()).toEqual(["filled", "filled", "filled"])

    await user.click(screen.getByRole("button", { name: /Save/ }))
    const [, saved] = saveLayout.mock.lastCall as [string, DeviceLayout]
    expect(
      saved.pages[0]?.bindings.map((b) => [b.control.index, b.trigger.type, b.target]),
    ).toEqual([
      [2, "click", { type: "action", action: "shop" }],
      [2, "doubleClick", { type: "workflow", workflow: "build" }],
      [2, "longPress", { type: "action", action: "next" }],
    ])
  })

  it("replaces what a slot holds and removes it", async () => {
    const user = renderEditor()
    await user.click(screen.getByRole("button", { name: "Key 3" }))
    drop("library:action:shop", slot("click"))
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Press: Shop")
    })
    // Dropping on a slot that holds something replaces it.
    drop("library:action:mail", slot("click"))
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Press: Mail")
    })
    // One step back is the action that was there.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(describes("Key 3")).toBe("Key 3: Press: Shop")
    await user.click(screen.getByRole("button", { name: "Redo" }))

    await user.click(screen.getByRole("button", { name: "Remove Single press" }))
    expect(describes("Key 3")).toBe("Key 3")
    expect(slots()).toEqual(["empty", "empty", "empty"])
    expect(screen.queryByRole("button", { name: /^Remove Single press/ })).not.toBeInTheDocument()

    // A page put on the key alone makes it the page's folder key: it opens
    // it on the press and its inspector holds the folder's settings only.
    drop("library:page:tools", slot("click"))
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Press: Open “Tools”")
    })
    expect(document.querySelector("[data-slot=folder-control]")).not.toBeNull()
    expect(document.querySelector("[data-gesture-slot]")).toBeNull()
  })

  it("refuses what is not an action, a workflow or another page", async () => {
    const user = renderEditor()
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    drop("library:action:shop", gestureSlotId({ kind: "key", index: 0 }, "click"))
    await waitFor(() => {
      expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    })
    await user.click(screen.getByRole("button", { name: "Key 3" }))

    // "New folder" and a key belong on keys.
    drop(NEW_FOLDER_DRAG_ID, slot("click"))
    drop("control:key:0", slot("doubleClick"))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledTimes(2)
    })
    expect(toast.error).toHaveBeenLastCalledWith(
      "A gesture takes an action, a workflow or another page",
      expect.anything(),
    )
    // Nothing changed: not the key with the slots, not the dragged key,
    // and no folder was made.
    expect(describes("Key 3")).toBe("Key 3")
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    expect(await pageNames(user)).toEqual(["Home", "Tools"])
    expect(slots()).toEqual(["empty", "empty", "empty"])
  })

  it("takes a drop on the raw release through the release's assignment", async () => {
    const user = renderEditor()
    await user.click(screen.getByRole("button", { name: "Key 3" }))
    drop("library:action:shop", slot("click"))
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Press: Shop")
    })
    drop("library:action:mail", releaseSlotId(KEY))
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Press: Shop · Release: Mail")
    })
    expect(row("Release")).toHaveTextContent("Mail")
    // One step back removes it again.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(describes("Key 3")).toBe("Key 3: Press: Shop")
  })

  it("has no slots where nothing is selected", () => {
    renderEditor()
    expect(slots()).toEqual([])
    expect([...dnd.droppables].filter((id) => id.startsWith("gesture-slot:"))).toEqual([])
  })
})
