// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { DeviceLayout } from "@/lib/types"
import { LayoutDesignerPage } from "@/pages/layout-designer"
import { type EditorSelection, EditorSelectionContext } from "@/shell/editor-selection-context"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import {
  agentConnection,
  agentStatus,
  deviceInfo,
  deviceModel,
  profileView,
  profiles,
} from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { assignment, sharedLayout } from "@/test/shared-layouts"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const mk2 = deviceModel()
const MK2 = mk2.id
const XL = "elgato.streamdeck.xl"

const layout = (id: string, name: string, model = MK2): DeviceLayout => ({
  id,
  name,
  model,
  homePage: `${id}-home`,
  pages: [{ id: `${id}-home`, name: "Home", bindings: [] }],
})
const main = layout("main", "Main")
const teams = layout("teams", "Teams")

const work = assignment("profile-1", "Work", { isDefault: true })
const home = assignment("profile-2", "Home")

function selection(change: Partial<EditorSelection> = {}): EditorSelection {
  return {
    models: { status: "ready", data: [mk2] },
    modelId: MK2,
    device: null,
    layoutId: null,
    selectDevice: vi.fn(() => true),
    selectProfile: vi.fn(() => true),
    selectLayout: vi.fn(() => true),
    showLayout: vi.fn(),
    adoptModel: vi.fn(),
    setDirty: vi.fn(),
    guard: vi.fn(() => true),
    isDirty: vi.fn(() => false),
    ...change,
  }
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  api.getProfile.mockResolvedValue(
    profileView({ layouts: [main, teams], activeLayouts: { [MK2]: "main" } }),
  )
  api.listSharedLayouts.mockResolvedValue([
    sharedLayout({ id: "main", name: "Main", profiles: [work, home] }),
    sharedLayout({ id: "teams", name: "Teams", profiles: [assignment("profile-1", "Work")] }),
    sharedLayout({ id: "spare", name: "Spare", profiles: [] }),
    sharedLayout({ id: "media", name: "Media", profiles: [home] }),
    sharedLayout({ id: "big", name: "Big board", model: XL, modelName: "Stream Deck XL" }),
  ])
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.pageTree.mockResolvedValue({ pages: [], warnings: [] })
  api.listActionTypes.mockResolvedValue([])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.assignLayout.mockResolvedValue(null)
})
afterEach(cleanup)

async function renderDesigner(editor = selection(), agent = agentConnection()) {
  const view = renderWithApp(
    <EditorSelectionContext value={editor}>
      <LayoutDesignerPage />
    </EditorSelectionContext>,
    { profiles: profiles([{ id: "profile-1", name: "Work" }]), agent },
  )
  await screen.findByRole("textbox", { name: "Layout name" })
  return { ...view, editor }
}

describe("the Device Editor's layout chooser", { timeout: 20_000 }, () => {
  it("marks the default and shared layouts and offers the model's others", async () => {
    const { user } = await renderDesigner()
    const trigger = await screen.findByRole("button", { name: "Layout: Main" })
    await waitFor(() => {
      expect(screen.getByRole("note")).toHaveTextContent(
        "Shared: edits apply to every profile using this layout (Work, Home).",
      )
    })
    await user.click(trigger)
    const own = await screen.findByRole("group", { name: "Layouts of this profile" })
    const items = within(own).getAllByRole("menuitem")
    expect(items.map((i) => i.textContent)).toEqual(["Maindefault · shared · 2 profiles", "Teams"])
    const others = screen.getByRole("group", { name: "Other layouts for Stream Deck MK.2" })
    // Layouts for another model are not offered.
    expect(
      within(others)
        .getAllByRole("menuitem")
        .map((i) => i.getAttribute("aria-label")),
    ).toEqual(["Use Media in this profile", "Use Spare in this profile"])
    expect(within(others).getByText("Used by Home")).toBeInTheDocument()
    expect(within(others).getByText("Not used by any profile")).toBeInTheDocument()
  })

  it("selects a layout of the profile", async () => {
    const { user, editor } = await renderDesigner()
    await user.click(screen.getByRole("button", { name: "Layout: Main" }))
    await user.click(await screen.findByRole("menuitem", { name: /^Teams/ }))
    expect(editor.selectLayout).toHaveBeenCalledWith("teams")
  })

  it("uses another layout of the model in this profile and opens it", async () => {
    const { user, editor } = await renderDesigner()
    await user.click(screen.getByRole("button", { name: "Layout: Main" }))
    await user.click(await screen.findByRole("menuitem", { name: "Use Spare in this profile" }))
    await waitFor(() => {
      expect(api.assignLayout).toHaveBeenCalledWith("profile-1", "spare")
    })
    await waitFor(() => {
      expect(editor.showLayout).toHaveBeenCalledWith("spare", MK2)
    })
  })

  it("has no shared note for a layout only this profile uses", async () => {
    await renderDesigner(selection({ layoutId: "teams" }))
    await screen.findByRole("button", { name: "Layout: Teams" })
    await waitFor(() => {
      expect(api.listSharedLayouts).toHaveBeenCalled()
    })
    expect(screen.queryByText(/Shared: edits apply/)).not.toBeInTheDocument()
  })

  it("says which layout the device shows and opens it", async () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ layout: { type: "compatible", layout: "teams", name: "Teams" } })],
      }),
    })
    const { user, editor } = await renderDesigner(selection(), agent)
    expect(await screen.findByText("The device shows Teams.")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open Teams" }))
    expect(editor.showLayout).toHaveBeenCalledWith("teams", MK2)
  })

  it("does not hint when the device shows the open layout", async () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ layout: { type: "compatible", layout: "main", name: "Main" } })],
      }),
    })
    await renderDesigner(selection(), agent)
    await screen.findByRole("button", { name: "Layout: Main" })
    expect(screen.queryByText(/The device shows/)).not.toBeInTheDocument()
  })

  it("explains that a shared layout is deleted on the Layouts page", async () => {
    api.deleteLayout.mockRejectedValue(new Error("Home uses this layout too"))
    const { user } = await renderDesigner()
    await waitFor(() => {
      expect(screen.getByRole("note")).toHaveTextContent("Shared")
    })
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Other profiles use this layout too")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("“Main” was not deleted", {
        description:
          "Home uses this layout too. A layout other profiles use is deleted on the Layouts page, or removed from this profile in Settings › Profiles.",
      })
    })
  })

  it("keeps Use for this model as the profile's default", async () => {
    api.activateLayout.mockResolvedValue(null)
    const { user } = await renderDesigner(selection({ layoutId: "teams" }))
    expect(screen.getByRole("button", { name: /Use for this model/ })).toBeEnabled()
    await user.click(screen.getByRole("button", { name: /Use for this model/ }))
    await waitFor(() => {
      expect(api.activateLayout).toHaveBeenCalledWith("profile-1", "teams")
    })
  })
})
