// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import type { ActionType, DeviceLayout, ProfileView } from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { DEVICE_MODELS } from "@/test/fixtures"

import { dragLook } from "./drag-look"
import { DragPreview } from "./drag-preview"
import { LayoutEditor } from "./editor"

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: vi.fn(),
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => [],
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh: vi.fn() }),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

beforeAll(installDomStubs)
afterEach(cleanup)

const context = {
  actions: [{ id: "mail", name: "Mail", icon: "mail" }] as ProfileView["profile"]["actions"],
  workflows: [],
  types: new Map([["system.openUrl", { id: "system.openUrl", name: "Open URL" } as ActionType]]),
  pageName: (id: string) => (id === "tools" ? "Tools" : undefined),
  images: new Map([["key:2", "data:image/png;base64,AAAA"]]),
}

describe("the drag preview", () => {
  it("names what is dragged, with its icon", () => {
    const label = (source: Parameters<typeof dragLook>[0]) => dragLook(source, context).label
    expect(label({ type: "target", target: { type: "action", action: "mail" } })).toBe("Mail")
    expect(label({ type: "target", target: { type: "openPage", page: "tools" } })).toBe(
      "Open Tools",
    )
    expect(label({ type: "target", target: { type: "back" } })).toBe("Back")
    expect(label({ type: "actionType", actionType: "system.openUrl" })).toBe("Open URL")
    expect(label({ type: "newFolder" })).toBe("New folder")
    expect(label({ type: "newMultiAction" })).toBe("Multi Action")

    render(
      <DragPreview
        look={dragLook({ type: "target", target: { type: "action", action: "mail" } }, context)}
      />,
    )
    const chip = document.querySelector("[data-slot=drag-preview]")
    expect(chip).toHaveTextContent("Mail")
    expect(chip?.querySelector("svg")).toHaveClass("size-4")
  })

  it("shows a dragged key as the device draws it", () => {
    const look = dragLook({ type: "control", control: { kind: "key", index: 2 } }, context)
    expect(look.label).toBe("Key 3")
    render(<DragPreview look={look} />)
    expect(document.querySelector("[data-slot=drag-preview] img")).toHaveAttribute(
      "src",
      "data:image/png;base64,AAAA",
    )
  })

  it("follows a drag from the library in the editor", async () => {
    const layout: DeviceLayout = {
      id: "layout",
      name: "Main",
      model: DEVICE_MODELS.mk2.id,
      homePage: "home",
      pages: [{ id: "home", name: "Home", bindings: [] }],
    }
    const view = {
      activeProjectId: null,
      profile: {
        id: "p",
        name: "Work",
        actions: [{ id: "mail", name: "Mail", icon: null }],
        workflows: [],
        projects: [],
        layouts: [layout],
      },
    } as unknown as ProfileView
    const user = userEvent.setup()
    render(
      <LayoutEditor
        profileId="p"
        view={view}
        model={DEVICE_MODELS.mk2}
        saved={layout}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />,
    )
    expect(document.querySelector("[data-slot=drag-preview]")).toBeNull()
    // A keyboard drag, as dnd-kit's keyboard sensor starts one.
    screen.getByRole("button", { name: "Drag Mail onto a control" }).focus()
    await user.keyboard(" ")
    const chip = await vi.waitFor(() => {
      const found = document.querySelector("[data-slot=drag-preview]")
      expect(found).not.toBeNull()
      return found
    })
    expect(chip).toHaveTextContent("Mail")
    expect(chip?.querySelector("svg")).toBeInTheDocument()
    await user.keyboard("{Escape}")
    await vi.waitFor(() => {
      expect(document.querySelector("[data-slot=drag-preview]")).toBeNull()
    })
  })
})
