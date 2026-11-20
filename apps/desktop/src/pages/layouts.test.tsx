// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { LayoutsPage } from "@/pages/layouts"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { deviceModel, profiles } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { assignment, sharedLayout, unknownModelLayout } from "@/test/shared-layouts"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const PROFILES = profiles([
  { id: "profile-1", name: "Work" },
  { id: "profile-2", name: "Home" },
  { id: "profile-3", name: "Travel" },
])

const desk = sharedLayout({
  profiles: [
    assignment("profile-1", "Work", { isDefault: true }),
    assignment("profile-2", "Home", { position: 1 }),
  ],
})
const spare = sharedLayout({ id: "layout-2", name: "Spare", pages: 1, bindings: 1 })
const old = unknownModelLayout({ profiles: [assignment("profile-3", "Travel")] })

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  api.listSharedLayouts.mockResolvedValue([desk, spare, old])
  api.listDeviceModels.mockResolvedValue([deviceModel()])
})
afterEach(cleanup)

async function renderPage() {
  const view = renderWithApp(<LayoutsPage />, { profiles: PROFILES })
  await screen.findByRole("table")
  return view
}

const row = (name: string) => {
  const cell = screen.getByRole("cell", { name })
  const element = cell.closest("tr")
  if (!element) throw new Error(`No row for ${name}`)
  return within(element)
}

describe("Layouts page", () => {
  it("lists the layouts with their device, size and the profiles using them", async () => {
    await renderPage()
    expect(screen.getAllByRole("row")).toHaveLength(4)
    const deskRow = row("Desk")
    expect(deskRow.getByText("Stream Deck MK.2")).toBeInTheDocument()
    expect(deskRow.getByText("2 pages · 9 keys")).toBeInTheDocument()
    const users = deskRow.getByRole("list", { name: "Profiles using Desk" })
    expect(within(users).getByText("Work")).toBeInTheDocument()
    expect(within(users).getByText("Home")).toBeInTheDocument()
    expect(within(users).getByText("· default")).toBeInTheDocument()
    expect(row("Spare").getByText("Not used")).toBeInTheDocument()
    expect(row("Spare").getByText("1 page · 1 key")).toBeInTheDocument()
    // Compatibility: own model and the others, explained.
    expect(deskRow.getByRole("note")).toHaveAccessibleName(
      "Compatibility: Stream Deck MK.2: Made for this model; Stream Deck XL: Every key fits; Stream Deck Mini: Misses 1 key (1 trigger)",
    )
    expect(deskRow.getByText("fits 1, misses 1")).toBeInTheDocument()
  })

  it("marks a layout whose device model is unknown", async () => {
    await renderPage()
    const oldRow = row("Old pad")
    expect(oldRow.getByText("Unknown model")).toBeInTheDocument()
    expect(oldRow.getByText("acme.pad")).toBeInTheDocument()
    expect(row("Desk").queryByText("Unknown model")).not.toBeInTheDocument()
  })

  it("searches by name", async () => {
    const { user } = await renderPage()
    await user.type(screen.getByRole("textbox", { name: "Search layouts" }), "spa")
    expect(screen.getAllByRole("row")).toHaveLength(2)
    expect(screen.getByRole("cell", { name: "Spare" })).toBeInTheDocument()
  })

  it("shows an empty state with a way to create a layout", async () => {
    api.listSharedLayouts.mockResolvedValue([])
    const { user } = renderWithApp(<LayoutsPage />, { profiles: PROFILES })
    expect(await screen.findByText("No layouts yet")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "New layout" }))
    expect(await screen.findByRole("dialog", { name: "New layout" })).toBeInTheDocument()
  })

  it("asks before deleting, lists the profiles and passes exactly them", async () => {
    api.deleteSharedLayout.mockResolvedValue(null)
    const { user, context } = await renderPage()
    await user.click(row("Desk").getByRole("button", { name: "More actions for Desk" }))
    await user.click(screen.getByRole("menuitem", { name: /Delete/ }))
    const dialog = await screen.findByRole("alertdialog")
    const listed = within(dialog).getByRole("list", { name: "Profiles using the layout" })
    expect(
      within(listed)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["Work (default layout)", "Home"])
    expect(dialog).toHaveTextContent("They fall back to their other layouts")
    expect(dialog).toHaveTextContent("The profiles themselves are not deleted")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(api.deleteSharedLayout).toHaveBeenCalledWith("layout-1", ["profile-1", "profile-2"])
    })
    expect(toast.success).toHaveBeenCalledWith("Layout deleted")
    expect(context.refresh).toHaveBeenCalled()
  })

  it("reports a refused delete", async () => {
    api.deleteSharedLayout.mockRejectedValue("the profiles using the layout changed")
    const { user } = await renderPage()
    await user.click(row("Spare").getByRole("button", { name: "More actions for Spare" }))
    await user.click(screen.getByRole("menuitem", { name: /Delete/ }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("No profile uses this layout")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    // The confirmation stays open and shows the refusal inline.
    expect(await within(dialog).findByText("the profiles using the layout changed")).toBeVisible()
    expect(screen.getByRole("alertdialog")).toBeInTheDocument()
    expect(api.deleteSharedLayout).toHaveBeenCalledWith("layout-2", [])
  })

  it("duplicates under a name that defaults to a copy", async () => {
    api.duplicateSharedLayout.mockResolvedValue(sharedLayout({ id: "layout-3" }))
    const { user } = await renderPage()
    await user.click(row("Desk").getByRole("button", { name: "More actions for Desk" }))
    await user.click(screen.getByRole("menuitem", { name: /Duplicate/ }))
    const dialog = await screen.findByRole("dialog", { name: "Duplicate layout" })
    expect(within(dialog).getByRole("textbox", { name: "Name of the copy" })).toHaveValue(
      "Desk copy",
    )
    await user.click(within(dialog).getByRole("button", { name: "Duplicate" }))
    await waitFor(() => {
      expect(api.duplicateSharedLayout).toHaveBeenCalledWith("layout-1", "Desk copy", null)
    })
  })

  it("renames", async () => {
    api.renameSharedLayout.mockResolvedValue(null)
    const { user } = await renderPage()
    await user.click(row("Desk").getByRole("button", { name: "More actions for Desk" }))
    await user.click(screen.getByRole("menuitem", { name: /Rename/ }))
    const dialog = await screen.findByRole("dialog", { name: "Rename layout" })
    const name = within(dialog).getByRole("textbox", { name: "Layout name" })
    await user.clear(name)
    await user.type(name, "Studio{Enter}")
    await waitFor(() => {
      expect(api.renameSharedLayout).toHaveBeenCalledWith("layout-1", "Studio")
    })
  })

  it("keeps the rename dialog open and shows a refusal inline", async () => {
    api.renameSharedLayout.mockRejectedValue("a layout with this name exists")
    const { user } = await renderPage()
    await user.click(row("Desk").getByRole("button", { name: "More actions for Desk" }))
    await user.click(screen.getByRole("menuitem", { name: /Rename/ }))
    const dialog = await screen.findByRole("dialog", { name: "Rename layout" })
    const name = within(dialog).getByRole("textbox", { name: "Layout name" })
    await user.clear(name)
    await user.type(name, "Spare{Enter}")
    expect(await within(dialog).findByText("a layout with this name exists")).toBeVisible()
    expect(toast.error).not.toHaveBeenCalled()
    // Cancel with a changed name asks before discarding it.
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    const discard = await screen.findByRole("alertdialog", { name: "Discard your changes?" })
    await user.click(within(discard).getByRole("button", { name: "Discard" }))
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Rename layout" })).not.toBeInTheDocument()
    })
  })

  it("assigns to and removes from profiles", async () => {
    api.assignLayout.mockResolvedValue(null)
    api.unassignLayout.mockResolvedValue(null)
    const { user } = await renderPage()
    await user.click(row("Desk").getByRole("button", { name: "Profiles using Desk" }))
    const travel = await screen.findByRole("checkbox", { name: "Travel" })
    expect(screen.getByRole("checkbox", { name: "Work" })).toBeChecked()
    expect(travel).not.toBeChecked()
    await user.click(travel)
    await waitFor(() => {
      expect(api.assignLayout).toHaveBeenCalledWith("profile-3", "layout-1")
    })
    await waitFor(() => {
      expect(screen.getByRole("checkbox", { name: "Home" })).toBeEnabled()
    })
    await user.click(screen.getByRole("checkbox", { name: "Home" }))
    await waitFor(() => {
      expect(api.unassignLayout).toHaveBeenCalledWith("profile-2", "layout-1")
    })
    expect(toast.success).toHaveBeenCalledWith("Removed from Home; the layout is kept")
  })

  it("does not assign a layout of an unknown model to more profiles", async () => {
    const { user } = await renderPage()
    await user.click(row("Old pad").getByRole("button", { name: "Profiles using Old pad" }))
    expect(await screen.findByRole("checkbox", { name: "Work" })).toBeDisabled()
    // Removing it stays possible.
    expect(screen.getByRole("checkbox", { name: "Travel" })).toBeEnabled()
  })

  it("opens a used layout in the Device Editor", async () => {
    const { user, context } = await renderPage()
    await user.click(row("Desk").getByRole("button", { name: "Open Desk in the Device Editor" }))
    expect(context.navigate).toHaveBeenCalledWith("designer")
  })

  it("offers to use an unused layout in the profile being edited before opening it", async () => {
    api.assignLayout.mockResolvedValue(null)
    const { user, context } = await renderPage()
    await user.click(row("Spare").getByRole("button", { name: "Open Spare in the Device Editor" }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Use “Spare” in Work?")
    expect(context.navigate).not.toHaveBeenCalled()
    await user.click(within(dialog).getByRole("button", { name: "Use in Work" }))
    await waitFor(() => {
      expect(api.assignLayout).toHaveBeenCalledWith("profile-1", "layout-2")
    })
    await waitFor(() => {
      expect(context.navigate).toHaveBeenCalledWith("designer")
    })
  })

  it("creates a layout for a model, assigned to the profile being edited", async () => {
    api.createSharedLayout.mockResolvedValue(sharedLayout({ id: "layout-9", name: "Stream" }))
    const { user, titlebarAction } = await renderPage()
    titlebarAction("layouts.new")
    const dialog = await screen.findByRole("dialog", { name: "New layout" })
    await waitFor(() => {
      expect(within(dialog).getByRole("combobox", { name: "Device model" })).toHaveTextContent(
        "Stream Deck MK.2",
      )
    })
    expect(within(dialog).getByRole("combobox", { name: "Assign to profile" })).toHaveTextContent(
      "Work",
    )
    await user.type(within(dialog).getByRole("textbox", { name: "Name" }), "Stream")
    await user.click(within(dialog).getByRole("button", { name: "Create" }))
    await waitFor(() => {
      expect(api.createSharedLayout).toHaveBeenCalledWith(
        "elgato.streamdeck.mk2",
        "Stream",
        "profile-1",
      )
    })
  })
})
