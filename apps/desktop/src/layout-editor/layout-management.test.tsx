// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { DialogProvider } from "@/components/dialogs/dialog-service"
import type { DeviceLayout } from "@/lib/types"
import { LayoutDesignerPage } from "@/pages/layout-designer"

import { LayoutMenu } from "./layout-management"
import { resolveLayout } from "./layout-selection"
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
const big = layout("big", "Big board", XL)

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
    place: vi.fn(() => undefined),
    rememberPlace: vi.fn(),
    ...change,
  }
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  api.getProfile.mockResolvedValue(
    profileView({ layouts: [main, teams, big], activeLayouts: { [MK2]: "main" } }),
  )
  api.listSharedLayouts.mockResolvedValue([
    sharedLayout({ id: "main", name: "Main", profiles: [work, home] }),
    sharedLayout({ id: "teams", name: "Teams", profiles: [assignment("profile-1", "Work")] }),
    sharedLayout({ id: "spare", name: "Spare", profiles: [] }),
    sharedLayout({ id: "media", name: "Media", profiles: [home] }),
    sharedLayout({
      id: "big",
      name: "Big board",
      model: XL,
      modelName: "Stream Deck XL",
      profiles: [assignment("profile-1", "Work")],
    }),
  ])
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.pageTree.mockResolvedValue({ pages: [], warnings: [] })
  api.listActionTypes.mockResolvedValue([])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.assignLayout.mockResolvedValue(null)
  api.activateLayout.mockResolvedValue(null)
  api.createLayout.mockResolvedValue({ ...layout("fresh", "Fresh") })
  api.duplicateLayout.mockResolvedValue("main-copy")
  api.renameSharedLayout.mockResolvedValue(null)
  api.deleteLayout.mockResolvedValue(null)
})
afterEach(cleanup)

/**
 * The toolbar's layout selector beside the Device Editor, as in the main
 * window: both read the shell's editor selection.
 */
async function renderDesigner(editor = selection(), agent = agentConnection()) {
  const view = renderWithApp(
    <DialogProvider>
      <EditorSelectionContext value={editor}>
        <LayoutMenu />
        <LayoutDesignerPage />
      </EditorSelectionContext>
    </DialogProvider>,
    { profiles: profiles([{ id: "profile-1", name: "Work" }]), agent },
  )
  await screen.findByRole("group", { name: /controls$/ })
  return { ...view, editor }
}

const trigger = (name = /^Layout: Main/) => screen.findByRole("button", { name })

async function openMenu(user: { click: (e: Element) => Promise<void> }, name?: RegExp) {
  await waitFor(async () => {
    expect(await trigger(name)).toBeEnabled()
  })
  await user.click(await trigger(name))
  return screen.findByRole("menu")
}

describe("resolveLayout", () => {
  it("takes the chosen layout, else the default, else the first", () => {
    expect(resolveLayout([main, teams], "teams", "main")).toBe(teams)
    expect(resolveLayout([main, teams], null, "teams")).toBe(teams)
    expect(resolveLayout([main, teams], "gone", "gone")).toBe(main)
    expect(resolveLayout<DeviceLayout>([], null, null)).toBeUndefined()
  })
})

describe("the toolbar's layout selector", { timeout: 20_000 }, () => {
  it("marks the default and shared layouts and offers the model's others", async () => {
    const { user } = await renderDesigner()
    expect(await trigger(/^Layout: Main, default$/)).toBeInTheDocument()
    await waitFor(() => {
      expect(screen.getByRole("note")).toHaveTextContent(
        "Shared: edits apply to every profile using this layout (Work, Home).",
      )
    })
    await openMenu(user)
    const own = await screen.findByRole("group", { name: "Layouts of this profile" })
    const items = within(own).getAllByRole("menuitem")
    expect(items.map((i) => i.textContent)).toEqual(["Maindefault · shared · 2 profiles", "Teams"])
    expect(items[0]).toHaveAttribute("aria-current", "true")
    const others = screen.getByRole("group", { name: "Other layouts for Stream Deck MK.2" })
    // Layouts for another model are not offered there.
    expect(
      within(others)
        .getAllByRole("menuitem")
        .map((i) => i.getAttribute("aria-label")),
    ).toEqual(["Use Media in this profile", "Use Spare in this profile"])
    expect(within(others).getByText("Used by Home")).toBeInTheDocument()
    expect(within(others).getByText("Not used by any profile")).toBeInTheDocument()
  })

  it("lists the profile's layouts of other models, but they cannot be chosen", async () => {
    const { user, editor } = await renderDesigner()
    await openMenu(user)
    const group = screen.getByRole("group", { name: "Layouts for other models" })
    const item = within(group).getByRole("menuitem", { name: /Big board/ })
    expect(item).toHaveAttribute("aria-disabled", "true")
    expect(item).toHaveTextContent("Stream Deck XL")
    await user.click(item)
    expect(editor.selectLayout).not.toHaveBeenCalled()
  })

  it("selects a layout of the profile by its id", async () => {
    const { user, editor } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /^Teams/ }))
    expect(editor.selectLayout).toHaveBeenCalledWith("teams")
  })

  it("is used with the keyboard", async () => {
    const { user, editor } = await renderDesigner()
    await waitFor(async () => {
      expect(await trigger()).toBeEnabled()
    })
    ;(await trigger()).focus()
    await user.keyboard("{Enter}")
    await screen.findByRole("menu")
    // The first item has the focus; the next one is Teams.
    await user.keyboard("{ArrowDown}")
    expect(document.activeElement).toHaveTextContent(/^Teams/)
    await user.keyboard("{Enter}")
    expect(editor.selectLayout).toHaveBeenCalledWith("teams")
  })

  it("asks before discarding unsaved changes, and keeps them when cancelled", async () => {
    const editor = selection({ isDirty: vi.fn(() => true) })
    const { user } = await renderDesigner(editor)
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /^Teams/ }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Discard unsaved changes?")
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    })
    expect(editor.selectLayout).not.toHaveBeenCalled()

    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /^Teams/ }))
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: "Discard changes",
      }),
    )
    await waitFor(() => {
      expect(editor.selectLayout).toHaveBeenCalledWith("teams")
    })
    expect(editor.setDirty).toHaveBeenCalledWith(false)
  })

  it("uses another layout of the model in this profile and opens it, without copying it", async () => {
    const { user, editor } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: "Use Spare in this profile" }))
    await waitFor(() => {
      expect(api.assignLayout).toHaveBeenCalledWith("profile-1", "spare")
    })
    await waitFor(() => {
      expect(editor.showLayout).toHaveBeenCalledWith("spare", MK2)
    })
    expect(api.duplicateLayout).not.toHaveBeenCalled()
    expect(api.duplicateSharedLayout).not.toHaveBeenCalled()
  })

  it("has no shared note for a layout only this profile uses", async () => {
    await renderDesigner(selection({ layoutId: "teams" }))
    await trigger(/^Layout: Teams$/)
    await waitFor(() => {
      expect(api.listSharedLayouts).toHaveBeenCalled()
    })
    expect(screen.queryByText(/Shared: edits apply/)).not.toBeInTheDocument()
  })

  it("opens the layout the device shows when none is chosen", async () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ layout: { type: "compatible", layout: "teams", name: "Teams" } })],
      }),
    })
    await renderDesigner(selection(), agent)
    await trigger(/^Layout: Teams/)
    expect(screen.queryByText(/The device shows/)).not.toBeInTheDocument()
  })

  it("says which layout the device shows and opens it", async () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ layout: { type: "compatible", layout: "teams", name: "Teams" } })],
      }),
    })
    // Main chosen in the toolbar while the device shows Teams.
    const { user, editor } = await renderDesigner(selection({ layoutId: "main" }), agent)
    expect(await screen.findByText("The device shows Teams.")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open Teams" }))
    expect(editor.showLayout).toHaveBeenCalledWith("teams", MK2)
    // The selector marks it as the one on the device.
    await openMenu(user)
    const item = await screen.findByRole("menuitem", { name: /^Teams/ })
    expect(within(item).getByRole("img", { name: "Shown on the device" })).toBeInTheDocument()
  })

  it("does not hint when the device shows the open layout", async () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ layout: { type: "compatible", layout: "main", name: "Main" } })],
      }),
    })
    await renderDesigner(selection(), agent)
    await trigger()
    expect(screen.queryByText(/The device shows/)).not.toBeInTheDocument()
  })

  it("creates a layout for the model and opens it", async () => {
    const { user, editor, context } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /New layout/ }))
    const dialog = await screen.findByRole("dialog", { name: "New layout for Stream Deck MK.2" })
    await user.type(within(dialog).getByLabelText("Name"), "Fresh")
    await user.click(within(dialog).getByRole("button", { name: "Create" }))
    await waitFor(() => {
      expect(api.createLayout).toHaveBeenCalledWith("profile-1", MK2, "Fresh", true)
    })
    await waitFor(() => {
      expect(editor.showLayout).toHaveBeenCalledWith("fresh", MK2)
    })
    expect(context.refresh).toHaveBeenCalled()
  })

  it("duplicates the open layout under a new name", async () => {
    const { user, editor } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /Duplicate layout/ }))
    const dialog = await screen.findByRole("dialog", { name: "Duplicate “Main”" })
    const name = within(dialog).getByLabelText("Name")
    expect(name).toHaveValue("Main (copy)")
    await user.clear(name)
    await user.type(name, "Main 2")
    await user.click(within(dialog).getByRole("button", { name: "Duplicate" }))
    await waitFor(() => {
      expect(api.duplicateLayout).toHaveBeenCalledWith("profile-1", "main", "Main 2")
    })
    await waitFor(() => {
      expect(editor.showLayout).toHaveBeenCalledWith("main-copy", MK2)
    })
  })

  it("renames the open layout, for every profile using it", async () => {
    const { user, context } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /Rename layout/ }))
    const dialog = await screen.findByRole("dialog", { name: "Rename “Main”" })
    expect(dialog).toHaveTextContent("every profile using it shows the new name")
    const name = within(dialog).getByLabelText("Name")
    await user.clear(name)
    await user.type(name, "Desk")
    await user.click(within(dialog).getByRole("button", { name: "Rename" }))
    await waitFor(() => {
      expect(api.renameSharedLayout).toHaveBeenCalledWith("main", "Desk")
    })
    expect(context.refresh).toHaveBeenCalled()
  })

  it("asks to discard unsaved changes before renaming", async () => {
    const { user } = await renderDesigner(selection({ isDirty: vi.fn(() => true) }))
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /Rename layout/ }))
    const confirm = await screen.findByRole("alertdialog")
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    })
    expect(screen.queryByRole("dialog", { name: /Rename/ })).not.toBeInTheDocument()
  })

  it("explains that a shared layout is deleted on the Layouts page", async () => {
    api.deleteLayout.mockRejectedValue(new Error("Home uses this layout too"))
    const { user } = await renderDesigner()
    await waitFor(() => {
      expect(screen.getByRole("note")).toHaveTextContent("Shared")
    })
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /Delete layout/ }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Other profiles use this layout too")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    // The refusal is shown in the dialog, which stays open.
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "“Main” was not deleted: Home uses this layout too. A layout other profiles use is deleted on the Layouts page, or removed from this profile in Settings › Profiles.",
    )
    expect(dialog).toBeInTheDocument()
    expect(toast.error).not.toHaveBeenCalled()
  })

  it("deletes a layout only this profile uses", async () => {
    const { user, context } = await renderDesigner(selection({ layoutId: "teams" }))
    await openMenu(user, /^Layout: Teams$/)
    await user.click(await screen.findByRole("menuitem", { name: /Delete layout/ }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("The layout and its pages are removed")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(api.deleteLayout).toHaveBeenCalledWith("profile-1", "teams")
    })
    expect(context.refresh).toHaveBeenCalled()
  })

  it("sets the open layout as the profile's default for the model", async () => {
    const { user } = await renderDesigner(selection({ layoutId: "teams" }))
    await openMenu(user, /^Layout: Teams$/)
    await user.click(await screen.findByRole("menuitem", { name: /Set as default/ }))
    await waitFor(() => {
      expect(api.activateLayout).toHaveBeenCalledWith("profile-1", "teams")
    })
  })

  it("does not offer to set the default layout as the default again", async () => {
    const { user } = await renderDesigner()
    await openMenu(user)
    expect(await screen.findByRole("menuitem", { name: /Default for this model/ })).toHaveAttribute(
      "aria-disabled",
      "true",
    )
  })

  it("opens the Layout Library to manage layouts", async () => {
    const { user, context } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /Manage layouts/ }))
    expect(context.navigate).toHaveBeenCalledWith("layouts")
  })
})

describe("the Device Editor without layout controls", { timeout: 20_000 }, () => {
  it("has no layout name field, layout chooser or layout buttons of its own", async () => {
    renderWithApp(
      <DialogProvider>
        <EditorSelectionContext value={selection()}>
          <LayoutDesignerPage />
        </EditorSelectionContext>
      </DialogProvider>,
      { profiles: profiles([{ id: "profile-1", name: "Work" }]) },
    )
    await screen.findByRole("group", { name: /controls$/ })
    expect(screen.queryByRole("textbox", { name: "Layout name" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /^Layout:/ })).not.toBeInTheDocument()
    for (const name of [/New layout/, /Duplicate/, /Use for this model/, /^Delete$/])
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument()
    // The editing bar keeps the editing commands.
    const bar = screen.getByRole("toolbar", { name: "Layout editing" })
    for (const name of ["Undo", "Redo", "Revert", "Save"])
      expect(within(bar).getByRole("button", { name })).toBeInTheDocument()
  })
})
