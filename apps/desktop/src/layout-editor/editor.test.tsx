// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { LayoutEditor } from "./editor"
import type { DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"

// Radix menus and selects rely on browser APIs that jsdom does not implement.
Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const saveLayout = vi.fn()

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => saveLayout(...args) as unknown,
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh: vi.fn(),
  }),
}))
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

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
}

const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: plus.id,
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
          control: { kind: "key", index: 3 },
          trigger: { type: "press" },
          target: { type: "openPage", page: "tools" },
        },
      ],
    },
    { id: "tools", name: "Tools", bindings: [] },
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
      model={plus}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const control = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => control(name).getAttribute("title")

beforeEach(() => {
  saveLayout.mockResolvedValue(undefined)
})
afterEach(cleanup)

describe("LayoutEditor", () => {
  it("draws every control of the model and describes bound ones", () => {
    renderEditor()
    expect(screen.getAllByRole("button", { name: /^Key \d+$/ })).toHaveLength(8)
    expect(screen.getAllByRole("button", { name: /^Dial \d+$/ })).toHaveLength(4)
    expect(screen.getAllByRole("button", { name: /^Touch zone \d+$/ })).toHaveLength(4)
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    expect(describes("Key 2")).toBe("Key 2")
    expect(screen.getByText("Page 1 of 2")).toBeInTheDocument()
  })

  it("selects with click, Cmd/Ctrl-click and Shift-click", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    expect(control("Key 1")).toHaveAttribute("aria-pressed", "true")
    expect(screen.getByText("Key 1", { selector: "[data-slot=card-title]" })).toBeInTheDocument()

    await user.keyboard("{Control>}")
    await user.click(control("Key 3"))
    await user.keyboard("{/Control}")
    expect(control("Key 1")).toHaveAttribute("aria-pressed", "true")
    expect(control("Key 3")).toHaveAttribute("aria-pressed", "true")
    expect(control("Key 2")).toHaveAttribute("aria-pressed", "false")
    expect(screen.getByText("2 controls selected")).toBeInTheDocument()

    await user.click(control("Key 2"))
    await user.keyboard("{Shift>}")
    await user.click(control("Key 5"))
    await user.keyboard("{/Shift}")
    expect(screen.getByText("4 controls selected")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Clear selection" }))
    expect(control("Key 2")).toHaveAttribute("aria-pressed", "false")
  })

  it("navigates with arrow keys and clears with Delete, undo and redo", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{ArrowRight}")
    expect(control("Key 2")).toHaveFocus()
    expect(control("Key 2")).toHaveAttribute("aria-pressed", "true")
    await user.keyboard("{ArrowLeft}{Delete}")
    expect(describes("Key 1")).toBe("Key 1")
    expect(screen.getByText("Unsaved")).toBeInTheDocument()

    await user.keyboard("{Control>}z{/Control}")
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    await user.keyboard("{Control>}{Shift>}z{/Shift}{/Control}")
    expect(describes("Key 1")).toBe("Key 1")
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled()
    await user.keyboard("{Control>}y{/Control}")
    expect(describes("Key 1")).toBe("Key 1")
  })

  it("copies, pastes and duplicates with the keyboard, across pages", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{Control>}c{/Control}")
    await user.click(control("Key 6"))
    await user.keyboard("{Control>}v{/Control}")
    expect(describes("Key 6")).toBe("Key 6: Press: Shop")

    await user.keyboard("{Control>}d{/Control}")
    expect(describes("Key 7")).toBe("Key 7: Press: Shop")

    await user.click(screen.getByRole("tab", { name: "Tools" }))
    await user.click(control("Key 2"))
    await user.keyboard("{Control>}v{/Control}")
    expect(describes("Key 2")).toBe("Key 2: Press: Shop")

    await user.click(screen.getByRole("tab", { name: "Home" }))
    await user.click(control("Key 1"))
    await user.keyboard("{Control>}x{/Control}")
    expect(describes("Key 1")).toBe("Key 1")
    await user.click(control("Key 8"))
    await user.keyboard("{Control>}v{/Control}")
    expect(describes("Key 8")).toBe("Key 8: Press: Shop")
  })

  it("offers assign, clear and folder navigation in the context menu", async () => {
    const user = renderEditor()
    fireEvent.contextMenu(control("Key 2"))
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitem", { name: /Clear/ })).toHaveAttribute("data-disabled")
    await user.click(within(menu).getByRole("menuitem", { name: /Assign/ }))
    fireEvent.click(await screen.findByRole("menuitem", { name: "Mail" }))
    expect(describes("Key 2")).toBe("Key 2: Press: Mail")
    expect(screen.queryByRole("menu")).not.toBeInTheDocument()

    fireEvent.contextMenu(control("Key 2"))
    await user.click(await screen.findByRole("menuitem", { name: /Clear/ }))
    expect(describes("Key 2")).toBe("Key 2")

    fireEvent.contextMenu(control("Key 4"))
    await user.click(await screen.findByRole("menuitem", { name: /Open “Tools”/ }))
    expect(screen.getByRole("tab", { name: "Tools" })).toHaveAttribute("aria-selected", "true")
    expect(screen.getByText("Page 2 of 2")).toBeInTheDocument()
    expect(screen.getByText("This folder is empty")).toBeInTheDocument()
    expect(screen.getByRole("note")).toHaveTextContent("no “Back” binding")

    await user.click(screen.getByRole("button", { name: "Back" }))
    expect(screen.getByRole("tab", { name: "Home" })).toHaveAttribute("aria-selected", "true")
  })

  it("saves the edited layout", async () => {
    const user = renderEditor()
    await user.click(control("Key 1"))
    await user.keyboard("{Delete}")
    await user.click(screen.getByRole("button", { name: /Save/ }))
    const [, saved] = saveLayout.mock.calls[0] as [string, DeviceLayout]
    expect(saved.pages[0]?.bindings).toHaveLength(1)
  })
})
