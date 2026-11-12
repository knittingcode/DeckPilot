// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { LayoutEditor } from "@/layout-editor/editor"
import type { ActionType, DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("@/lib/app-context", () => ({
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh: vi.fn(),
  }),
}))

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "verified",
}

const logType: ActionType = {
  id: "general.log",
  name: "Log message",
  description: "",
  category: "general",
  requiresProject: false,
  parameters: [
    { key: "message", label: "Message", kind: { type: "text", multiline: false }, required: true },
    {
      key: "level",
      label: "Level",
      kind: { type: "choice", options: ["info", "warn"] },
      required: false,
    },
  ],
}

const teamName = { type: "reference", value: "shared.team.name" } as const

/**
 * Key 1: a raw press (0.6.0 style). Key 2: three gestures, one with a
 * shared-variable override, and its own long-press time. The Tools page has
 * gestures too, so a save must carry every page as it was.
 */
const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  explicitTree: true,
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "shop" },
          label: "Shop",
        },
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "click" },
          target: { type: "action", action: "shop", overrides: { message: teamName } },
          label: null,
          appearance: { background: "#204060" },
        },
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "doubleClick" },
          target: { type: "workflow", workflow: "deploy" },
        },
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "longPress" },
          target: { type: "openPage", page: "tools" },
        },
      ],
      gestures: [{ control: { kind: "key", index: 1 }, longPressMs: 1500 }],
    },
    {
      id: "tools",
      name: "Tools",
      parent: "home",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "back" },
        },
        {
          control: { kind: "key", index: 4 },
          trigger: { type: "longPress" },
          target: { type: "action", action: "mail", overrides: { level: teamName } },
        },
      ],
      gestures: [{ control: { kind: "key", index: 4 }, doubleClickMs: 500 }],
    },
  ],
}

const view = {
  activeProjectId: null,
  issues: [],
  actionUsages: {},
  workflowUsages: {},
  profile: {
    id: "p",
    name: "Work",
    description: "",
    actions: [
      {
        id: "shop",
        name: "Shop",
        actionType: logType.id,
        parameters: { message: { type: "text", value: "shop" } },
      },
      { id: "mail", name: "Mail", actionType: logType.id, parameters: {} },
    ],
    workflows: [{ id: "deploy", name: "Deploy", steps: [] }],
    projects: [],
    layouts: [layout],
  },
} as unknown as ProfileView

function renderEditor(saved: DeviceLayout = layout) {
  const user = userEvent.setup()
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={mk2}
      saved={saved}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const key = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => key(name).getAttribute("title")
const row = (name: string) => screen.getByRole("combobox", { name })

async function choose(user: ReturnType<typeof userEvent.setup>, gesture: string, option: string) {
  await user.click(row(gesture))
  await user.click(await screen.findByRole("option", { name: option }))
}

async function save(user: ReturnType<typeof userEvent.setup>): Promise<DeviceLayout> {
  await user.click(screen.getByRole("button", { name: /Save/ }))
  await waitFor(() => {
    expect(api.saveLayout).toHaveBeenCalled()
  })
  return api.saveLayout.mock.calls.at(-1)?.[1] as DeviceLayout
}

beforeAll(() => {
  // Radix selects rely on browser APIs that jsdom does not implement.
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.releasePointerCapture = () => undefined
  Element.prototype.scrollIntoView = () => undefined
})
beforeEach(() => {
  resetApi()
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.listActionTypes.mockResolvedValue([logType])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 350, longPressMs: 800 })
  api.saveLayout.mockImplementation((_, saved) => Promise.resolve(saved))
})
afterEach(cleanup)

describe("gesture editor", () => {
  it("adds and removes single click, double click and long press", async () => {
    const user = renderEditor()
    await user.click(key("Key 3"))
    expect(screen.getByRole("heading", { name: "Gestures" })).toBeInTheDocument()

    await choose(user, "Double click", "Deploy")
    expect(describes("Key 3")).toBe("Key 3: Double click: Deploy")
    await choose(user, "Long press", "Mail")
    await choose(user, "Single click", "Shop")
    expect(describes("Key 3")).toBe("Key 3: Double click: Deploy · Long press: Mail · Click: Shop")
    expect(screen.getByRole("note")).toHaveTextContent("after the double-click window (350 ms)")
    expect(screen.getByLabelText("Double-click window (ms)")).toHaveAttribute(
      "placeholder",
      "Default (350)",
    )

    await choose(user, "Double click", "Nothing")
    expect(screen.getByRole("note")).toHaveTextContent("when the key is released")
    await choose(user, "Long press", "Nothing")
    expect(screen.getByRole("note")).toHaveTextContent("as soon as the key is pressed")
    await choose(user, "Single click", "Nothing")
    expect(describes("Key 3")).toBe("Key 3")

    // An empty key's single click alone is the immediate press (D2).
    await choose(user, "Single click (immediate)", "Mail")
    expect(describes("Key 3")).toBe("Key 3: Press: Mail")
    const saved = await save(user)
    expect(saved.pages[0]?.bindings.at(-1)).toEqual({
      control: { kind: "key", index: 2 },
      trigger: { type: "press" },
      target: { type: "action", action: "mail" },
      label: null,
    })
  })

  it("asks before turning a press into a click and changes nothing on cancel", async () => {
    const user = renderEditor()
    await user.click(key("Key 1"))
    expect(row("Single click (immediate)")).toHaveTextContent("Shop")

    await choose(user, "Long press", "Mail")
    const dialog = await screen.findByRole("alertdialog", { name: "Switch Key 1 to gestures?" })
    expect(dialog).toHaveTextContent("acts as soon as it is pressed")
    expect(dialog).toHaveTextContent("before the long-press time (800 ms)")
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument()

    await choose(user, "Double click", "Deploy")
    const confirm = await screen.findByRole("alertdialog")
    expect(confirm).toHaveTextContent("after the double-click window (350 ms)")
    await user.click(within(confirm).getByRole("button", { name: "Switch to gestures" }))
    expect(describes("Key 1")).toBe("Key 1: Click: Shop · Double click: Deploy")
    expect(row("Single click")).toHaveTextContent("Shop")

    const saved = await save(user)
    const keyOne = saved.pages[0]?.bindings.filter((b) => b.control.index === 0)
    expect(keyOne).toEqual([
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "click" },
        target: { type: "action", action: "shop" },
        label: "Shop",
      },
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "doubleClick" },
        target: { type: "workflow", workflow: "deploy" },
        label: null,
      },
    ])
  })

  it("keeps raw release bindings and says the conversion removes them", async () => {
    const user = renderEditor()
    await user.click(key("Key 1"))
    await choose(user, "Release (immediate)", "Mail")
    expect(describes("Key 1")).toBe("Key 1: Press: Shop · Release: Mail")

    await choose(user, "Long press", "Deploy")
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Its release binding (“Mail”) is removed")
    await user.click(within(dialog).getByRole("button", { name: "Switch to gestures" }))
    expect(describes("Key 1")).toBe("Key 1: Click: Shop · Long press: Deploy")
    // A gesture key offers no raw release.
    expect(screen.queryByRole("combobox", { name: "Release (immediate)" })).not.toBeInTheDocument()
  })

  it("overrides parameters per gesture and saves a variable reference as a reference", async () => {
    const user = renderEditor()
    await user.click(key("Key 2"))
    const toggle = screen.getByRole("button", {
      name: /Parameters for this gesture \(1 overridden\)/,
    })
    expect(toggle).toHaveAttribute("aria-expanded", "false")
    await user.click(toggle)
    expect(toggle).toHaveAttribute("aria-expanded", "true")
    expect(screen.getByRole("checkbox", { name: "Override “Message”" })).toBeChecked()

    // A second override next to the reference, from the shared editors.
    await user.click(screen.getByRole("checkbox", { name: "Override “Level”" }))
    await user.click(screen.getByRole("combobox", { name: "Level" }))
    await user.click(await screen.findByRole("option", { name: "warn" }))

    const saved = await save(user)
    const click = saved.pages[0]?.bindings.find(
      (b) => b.control.index === 1 && b.trigger.type === "click",
    )
    expect(click?.target).toEqual({
      type: "action",
      action: "shop",
      overrides: { message: teamName, level: { type: "choice", value: "warn" } },
    })
    // The saved action itself is not touched.
    expect(api.updateAction).not.toHaveBeenCalled()
    expect(api.createAction).not.toHaveBeenCalled()

    // Removing every override leaves no empty `overrides`.
    await user.click(screen.getByRole("checkbox", { name: "Override “Message”" }))
    await user.click(screen.getByRole("checkbox", { name: "Override “Level”" }))
    const cleared = await save(user)
    expect(
      cleared.pages[0]?.bindings.find((b) => b.control.index === 1 && b.trigger.type === "click")
        ?.target,
    ).toEqual({ type: "action", action: "shop" })
  })

  it("validates per-key timings and stores only overrides", async () => {
    const user = renderEditor()
    await user.click(key("Key 2"))
    const long = screen.getByLabelText("Long-press time (ms)")
    expect(long).toHaveValue(1500)
    const double = screen.getByLabelText("Double-click window (ms)")
    expect(double).toHaveValue(null)
    expect(double).toHaveAttribute("placeholder", "Default (350)")

    await user.type(double, "90")
    expect(double).toHaveAttribute("aria-invalid", "true")
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Double-click window must be between 150 and 1000 ms.",
    )
    expect(double).toHaveAccessibleDescription(/between 150 and 1000 ms/)
    await user.clear(double)
    await user.type(double, "450")
    expect(double).not.toHaveAttribute("aria-invalid")

    await user.click(screen.getByRole("button", { name: "Use the default long-press time" }))
    expect(long).toHaveValue(null)

    const saved = await save(user)
    expect(saved.pages[0]?.gestures).toEqual([
      { control: { kind: "key", index: 1 }, doubleClickMs: 450 },
    ])
  })

  it("saves every gesture, override, timing and page it did not edit", async () => {
    const user = renderEditor()
    await user.click(key("Key 2"))
    await user.type(screen.getByRole("textbox", { name: "Label for Double click" }), "Ship")
    const saved = await save(user)
    const expected = structuredClone(layout)
    const double = expected.pages[0]?.bindings[2]
    if (double) double.label = "Ship"
    expect(saved).toEqual(expected)
  })

  it("shows the backend's validation messages at the control", async () => {
    api.saveLayout.mockRejectedValue(
      new Error(
        "page `Home`: key 1: the long-press time must be between 300 and 5000 ms, not 9000 ms\npage `Home`: key 12 shows an icon that is not in the profile",
      ),
    )
    const user = renderEditor()
    await user.click(key("Key 2"))
    await user.type(screen.getByRole("textbox", { name: "Label for Long press" }), "x")
    await user.click(screen.getByRole("button", { name: /Save/ }))
    // Inline at the control, named as in the editor (the backend counts from 0).
    const inline = await screen.findByText(
      "Key 2: the long-press time must be between 300 and 5000 ms, not 9000 ms",
      { selector: "li" },
    )
    const list = inline.closest("ul")
    expect(list).toHaveAttribute("role", "alert")
    expect(list).not.toHaveTextContent("icon")
  })
})
