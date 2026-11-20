// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// Compatibility in the device editor (docs/device-controls.md): the library
// for the selected control, drops, the inspector and saving.

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionType,
  ControlFace,
  DeviceLayout,
  DeviceModel,
  ModuleInfo,
  ProfileView,
} from "@/lib/types"

import { catalogDragId } from "./catalog"
import { LayoutEditor } from "./editor"
import { LIBRARY_COMPATIBLE_ONLY_KEY } from "./library-sidebar"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const KEY_INPUTS = ["press", "release", "click", "double-click", "long-press"] as const

const type = (id: string, name: string, controls?: ActionType["controls"]): ActionType => ({
  id,
  name,
  description: `${name}.`,
  category: "Navigation",
  requiresProject: false,
  parameters: [],
  ...(controls ? { controls } : {}),
})

/** Encoder only: a volume knob. */
const volume = type("audio.volume", "Volume", {
  encoder: ["rotate-clockwise", "rotate-counter-clockwise"],
})
/** Keys and, once per turn or press, dials. */
const nextPage = type("deck.next-page", "Next page", {
  key: [...KEY_INPUTS],
  encoder: ["rotate-clockwise", "rotate-counter-clockwise", "press", "click"],
})
/** Keys only, declared. */
const mute = type("audio.mute", "Mute", { key: [...KEY_INPUTS] })
/** No declaration: keys only. */
const log = type("general.log", "Log")

const modules: ModuleInfo[] = [
  moduleInfo({ id: "audio", actionTypes: [volume, mute] }),
  moduleInfo({ id: "deck", actionTypes: [nextPage, log] }),
]

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

const api = vi.hoisted(() => ({
  previewLayout: vi.fn(),
  saveLayout: vi.fn(),
  createAction: vi.fn(),
}))

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: (...args: unknown[]) => api.previewLayout(...args) as unknown,
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([volume, nextPage, mute, log]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => api.saveLayout(...args) as unknown,
    createAction: (...args: unknown[]) => api.createAction(...args) as unknown,
  },
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
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

const actions = [
  { id: "vol", name: "Room volume", actionType: volume.id, parameters: {}, icon: null },
  { id: "mute", name: "Mute mic", actionType: mute.id, parameters: {}, icon: null },
  { id: "log", name: "Write log", actionType: log.id, parameters: {}, icon: null },
  { id: "next", name: "Forward", actionType: nextPage.id, parameters: {}, icon: null },
]

function layoutFor(model: DeviceModel, bindings: DeviceLayout["pages"][number]["bindings"] = []) {
  return {
    id: "layout",
    name: "Main",
    model: model.id,
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  } satisfies DeviceLayout
}

function renderEditor(model: DeviceModel, saved: DeviceLayout = layoutFor(model)) {
  const view = {
    activeProjectId: null,
    profile: {
      id: "p",
      name: "Work",
      actions,
      workflows: [],
      projects: [],
      layouts: [saved],
    },
  } as unknown as ProfileView
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={model}
      saved={saved}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return userEvent.setup()
}

const control = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => control(name).getAttribute("title")
const library = () => screen.getByRole("region", { name: "Actions" }).parentElement as HTMLElement

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
  localStorage.clear()
  dnd.data.clear()
  vi.mocked(toast.error).mockClear()
  vi.mocked(toast.warning).mockClear()
  api.previewLayout.mockReset()
  api.previewLayout.mockResolvedValue([])
  api.saveLayout.mockReset()
  api.saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  api.createAction.mockReset()
})
afterEach(cleanup)

describe("the library for the selected control", { timeout: 20_000 }, () => {
  it("disables what cannot go on a Stream Deck MK.2 key, with the reason", async () => {
    const user = renderEditor(mk2)
    // Nothing is selected: everything can be dragged.
    expect(screen.getByText("Select a control to see what fits it.")).toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: "Drag Room volume onto a control" }),
    ).toBeInTheDocument()

    await user.click(control("Key 1"))
    expect(screen.getByText("Showing what fits Key 1.")).toBeInTheDocument()
    const reason = "Requires a dial. The selected Stream Deck MK.2 key supports press actions only."
    // A profile action of an encoder-only type, and the type in the catalog.
    const action = screen.getByLabelText(`Room volume: not for the selected control. ${reason}`)
    expect(action).toHaveAttribute("aria-disabled", "true")
    expect(within(action).getByText(reason)).toBeInTheDocument()
    expect(
      screen.getByLabelText(`Volume: not for the selected control. ${reason}`),
    ).toHaveAttribute("aria-disabled", "true")
    // Key actions stay draggable, declared or not.
    expect(screen.getByRole("button", { name: "Drag Mute mic onto a control" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "Drag Write log onto a control" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "Drag Forward onto a control" })).toBeEnabled()
  })

  it("disables key actions for a dial and explains a legacy action", async () => {
    const user = renderEditor(plus)
    await user.click(control("Dial 1"))
    const reason =
      "Requires a key. The selected Stream Deck + dial supports turn and press actions."
    expect(
      screen.getByLabelText(`Mute mic: not for the selected control. ${reason}`),
    ).toHaveAttribute("aria-disabled", "true")
    // An action without a declaration is keys only.
    expect(
      screen.getByLabelText(`Write log: not for the selected control. ${reason}`),
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Drag Room volume onto a control" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "Drag Forward onto a control" })).toBeEnabled()

    await user.click(control("Touch zone 1"))
    expect(
      screen.getByLabelText(
        "Write log: not for the selected control. Requires a key. The selected Stream Deck + touch zone supports tap actions only.",
      ),
    ).toBeInTheDocument()
  })

  it("hides what does not fit with “Compatible only”, remembered", async () => {
    const user = renderEditor(mk2)
    await user.click(control("Key 1"))
    const toggle = screen.getByRole("switch", { name: "Compatible only" })
    expect(toggle).toHaveAttribute("aria-checked", "false")
    await user.click(toggle)
    expect(screen.queryByText("Room volume")).not.toBeInTheDocument()
    expect(within(library()).queryByText("Volume")).not.toBeInTheDocument()
    expect(screen.getByText("Mute mic")).toBeInTheDocument()
    expect(localStorage.getItem(LIBRARY_COMPATIBLE_ONLY_KEY)).toBe("true")
    cleanup()

    // Kept for the next editor; without a selected control nothing is hidden.
    const again = renderEditor(mk2)
    expect(screen.getByRole("switch", { name: "Compatible only" })).toHaveAttribute(
      "aria-checked",
      "true",
    )
    expect(screen.getByText("Room volume")).toBeInTheDocument()
    await again.click(control("Key 2"))
    expect(screen.queryByText("Room volume")).not.toBeInTheDocument()
  })
})

describe("dropping", { timeout: 20_000 }, () => {
  it("refuses an incompatible action: nothing is bound, the drop is shown and announced", async () => {
    renderEditor(plus)
    drop("library:action:mute", "encoder:0")
    expect(describes("Dial 1")).toBe("Dial 1")
    const reason =
      "Requires a key. The selected Stream Deck + dial supports turn and press actions."
    expect(toast.error).toHaveBeenCalledWith("Mute mic was not put on Dial 1", {
      description: reason,
    })
    expect(control("Dial 1")).toHaveAttribute("data-drop", "rejected")
    const announcement = document.querySelector("[data-slot='drop-announcement']")
    expect(announcement).toHaveTextContent(`Mute mic cannot go on Dial 1. ${reason}`)
    expect(announcement).toHaveAttribute("aria-live", "assertive")
    // The state goes away after a moment.
    await waitFor(
      () => {
        expect(control("Dial 1")).not.toHaveAttribute("data-drop")
      },
      { timeout: 4000 },
    )
  })

  it("refuses an encoder-only type from the catalog on a key, and creates nothing", () => {
    renderEditor(plus)
    drop(catalogDragId(volume.id), "key:0")
    expect(api.createAction).not.toHaveBeenCalled()
    expect(describes("Key 1")).toBe("Key 1")
    expect(toast.error).toHaveBeenCalledWith("Volume was not put on Key 1", {
      description: "Requires a dial. The selected Stream Deck + key supports press actions only.",
    })
  })

  it("refuses to move a key's binding onto a dial", () => {
    renderEditor(
      plus,
      layoutFor(plus, [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "mute" },
        },
      ]),
    )
    drop("control:key:0", "encoder:1")
    expect(describes("Key 1")).toBe("Key 1: Press: Mute mic")
    expect(describes("Dial 2")).toBe("Dial 2")
    expect(toast.error).toHaveBeenCalledWith("Key 1 was not put on Dial 2", expect.anything())
  })

  it("binds a compatible action to a trigger it can run from", () => {
    renderEditor(plus)
    drop("library:action:next", "encoder:0")
    expect(describes("Dial 1")).toBe("Dial 1: Turn right: Forward")
    expect(toast.error).not.toHaveBeenCalled()
  })
})

describe("the inspector", { timeout: 20_000 }, () => {
  const stored = layoutFor(plus, [
    {
      control: { kind: "encoder", index: 0 },
      trigger: { type: "rotateClockwise" },
      target: { type: "action", action: "log" },
    },
  ])
  const preview: ControlFace[] = [
    {
      control: { kind: "encoder", index: 0 },
      face: { title: "Write log" } as ControlFace["face"],
      incompatible: [
        {
          trigger: { type: "rotateClockwise" },
          reason: { type: "controlKindUnsupported", kind: "encoder", supported: ["key"] },
          message: "the action cannot be bound to an encoder; it works on keys only",
        },
      ],
    },
  ]

  it("says a stored incompatible binding is kept but not run, and why", async () => {
    api.previewLayout.mockResolvedValue(preview)
    const user = renderEditor(plus, stored)
    await user.click(control("Dial 1"))
    expect(
      await screen.findByText(
        "Not run from this control: Requires a key. The selected Stream Deck + dial supports turn and press actions. It stays assigned but is not run.",
      ),
    ).toBeInTheDocument()
  })

  it("offers each trigger only what can run from it", async () => {
    const user = renderEditor(plus)
    await user.click(control("Dial 1"))
    await user.click(screen.getByRole("combobox", { name: "Turn right" }))
    const listbox = screen.getByRole("listbox")
    expect(within(listbox).getByRole("option", { name: "Forward" })).not.toHaveAttribute(
      "data-disabled",
    )
    expect(within(listbox).getByRole("option", { name: "Room volume" })).not.toHaveAttribute(
      "data-disabled",
    )
    // What cannot run from it is not offered (no disabled entries).
    expect(within(listbox).queryByRole("option", { name: /Mute mic/ })).toBeNull()
    expect(within(listbox).queryByRole("option", { name: /Write log/ })).toBeNull()
  })

  it("offers a dial's press gestures only to actions that react to them", async () => {
    const user = renderEditor(plus)
    await user.click(control("Dial 1"))
    // Next page reacts to a press of a dial but not to a double click.
    await user.click(screen.getByRole("combobox", { name: "Assign an action to: Double click" }))
    const listbox = screen.getByRole("listbox")
    expect(
      within(listbox).getByRole("option", { name: "Forward (not on this control)" }),
    ).toHaveAttribute("data-disabled")
    expect(
      within(listbox).getByRole("option", { name: "Room volume (not on this control)" }),
    ).toHaveAttribute("data-disabled")
  })
})

describe("saving", { timeout: 20_000 }, () => {
  it("shows the reason when the backend refuses a new incompatible binding", async () => {
    api.saveLayout.mockRejectedValue(
      new Error(
        "page `Home`: encoder 0 (RotateClockwise): the action cannot be bound to an encoder; it works on keys only",
      ),
    )
    const user = renderEditor(plus)
    // Any change: a new page, then back to the first.
    await user.click(screen.getByRole("button", { name: "Add page" }))
    await user.click(screen.getByRole("button", { name: "Previous page" }))
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(
      await screen.findByText(
        "page `Home`: encoder 0 (Turn right): the action cannot be bound to an encoder; it works on keys only",
      ),
    ).toBeInTheDocument()
    // At the dial too, named as in the editor.
    await user.click(control("Dial 1"))
    expect(
      screen.getByText(
        "Dial 1 (Turn right): the action cannot be bound to an encoder; it works on keys only",
      ),
    ).toBeInTheDocument()
  })

  it("says which stored incompatible bindings it kept", async () => {
    api.saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
      Promise.resolve({
        ...saved,
        agentReload: { state: "applied" },
        incompatible: [
          "page `Home`: touch zone 0 (Touch { gesture: Tap }): the action cannot be bound to a touch zone; it works on keys only",
        ],
      }),
    )
    const user = renderEditor(plus)
    // Any change: a new page, then back to the first.
    await user.click(screen.getByRole("button", { name: "Add page" }))
    await user.click(screen.getByRole("button", { name: "Previous page" }))
    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(toast.warning).toHaveBeenCalledWith("1 binding is kept but not run", {
        description:
          "Their action cannot run from their control: page `Home`: touch zone 0 (Tap): the action cannot be bound to a touch zone; it works on keys only",
      })
    })
  })
})
