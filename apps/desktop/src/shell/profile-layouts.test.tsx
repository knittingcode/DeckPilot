// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { ProfileLayoutEntry, SharedLayoutSummary } from "@/lib/types"
import { ProfileLayouts } from "@/shell/profile-layouts"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"
import { sharedLayout, unknownModelLayout } from "@/test/shared-layouts"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const desk = sharedLayout({ id: "desk", name: "Desk" })
const music = sharedLayout({ id: "music", name: "Music" })
const xl = sharedLayout({
  id: "xl",
  name: "Big board",
  model: "elgato.streamdeck.xl",
  modelName: "Stream Deck XL",
})
const spare = sharedLayout({ id: "spare", name: "Spare" })
const old = unknownModelLayout({ id: "old", name: "Old pad" })

function entry(
  layout: SharedLayoutSummary,
  position: number,
  change: Partial<ProfileLayoutEntry> = {},
): ProfileLayoutEntry {
  return { layout, position, isDefault: false, unavailable: [], ...change }
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  // Deliberately out of order: the component follows `position`.
  api.profileLayouts.mockResolvedValue([
    entry(xl, 2, { isDefault: true }),
    entry(desk, 0, { isDefault: true }),
    entry(music, 1, {
      unavailable: [
        "Key 3 on Main: workflow Deploy of profile Home",
        "Key 5 on Main: project Shop",
      ],
    }),
  ])
  api.listSharedLayouts.mockResolvedValue([desk, music, xl, spare, old])
  for (const command of [
    api.reorderProfileLayouts,
    api.setDefaultLayout,
    api.unassignLayout,
    api.assignLayout,
  ]) {
    command.mockResolvedValue(null)
  }
})
afterEach(cleanup)

async function renderLayouts(onOpenLayout?: (id: string) => void) {
  const view = renderWithApp(<ProfileLayouts profileId="profile-1" onOpenLayout={onOpenLayout} />)
  await screen.findByRole("list")
  return view
}

const item = (name: string) => within(screen.getByRole("listitem", { name }))

describe("layouts of a profile", () => {
  it("lists the assigned layouts in the profile's order", async () => {
    await renderLayouts()
    expect(api.profileLayouts).toHaveBeenCalledWith("profile-1")
    expect(screen.getAllByRole("listitem").map((li) => li.getAttribute("aria-label"))).toEqual([
      "Desk",
      "Music",
      "Big board",
    ])
    expect(item("Desk").getByText("Default for Stream Deck MK.2")).toBeInTheDocument()
    expect(item("Big board").getByText("Default for Stream Deck XL")).toBeInTheDocument()
    expect(item("Desk").getByText("Stream Deck MK.2")).toBeInTheDocument()
  })

  it("reorders with the full new order", async () => {
    const { user } = await renderLayouts()
    expect(item("Desk").getByRole("button", { name: "Move Desk up" })).toBeDisabled()
    expect(item("Big board").getByRole("button", { name: "Move Big board down" })).toBeDisabled()
    await user.click(item("Music").getByRole("button", { name: "Move Music up" }))
    await waitFor(() => {
      expect(api.reorderProfileLayouts).toHaveBeenCalledWith("profile-1", ["music", "desk", "xl"])
    })
    await waitFor(() => {
      expect(item("Music").getByRole("button", { name: "Move Music down" })).toBeEnabled()
    })
    await user.click(item("Music").getByRole("button", { name: "Move Music down" }))
    await waitFor(() => {
      expect(api.reorderProfileLayouts).toHaveBeenLastCalledWith("profile-1", [
        "desk",
        "xl",
        "music",
      ])
    })
  })

  it("makes a layout the default for its model", async () => {
    const { user } = await renderLayouts()
    await user.click(
      item("Music").getByRole("button", { name: "Make default for Stream Deck MK.2" }),
    )
    await waitFor(() => {
      expect(api.setDefaultLayout).toHaveBeenCalledWith("profile-1", "music")
    })
  })

  it("removes a layout from the profile after saying the layout is kept", async () => {
    const { user, context } = await renderLayouts()
    await user.click(item("Desk").getByRole("button", { name: "Remove Desk from the profile" }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("The layout itself is kept")
    await user.click(within(dialog).getByRole("button", { name: "Remove" }))
    await waitFor(() => {
      expect(api.unassignLayout).toHaveBeenCalledWith("profile-1", "desk")
    })
    expect(toast.success).toHaveBeenCalledWith(
      "Layout removed from the profile; the layout is kept",
    )
    expect(context.refresh).toHaveBeenCalled()
  })

  it("assigns a layout not in the profile yet", async () => {
    const { user } = await renderLayouts()
    await user.click(screen.getByRole("button", { name: "Assign layout" }))
    const menu = await screen.findByRole("menu")
    const items = within(menu).getAllByRole("menuitem")
    expect(items.map((i) => i.textContent)).toEqual([
      "Old padacme.pad: unknown model, cannot be assigned",
      "SpareStream Deck MK.2 · fits 1 of 2 other models",
    ])
    expect(items[0]).toHaveAttribute("aria-disabled", "true")
    await user.click(within(menu).getByRole("menuitem", { name: /Spare/ }))
    await waitFor(() => {
      expect(api.assignLayout).toHaveBeenCalledWith("profile-1", "spare")
    })
  })

  it("shows the keys that cannot run in this profile", async () => {
    await renderLayouts()
    const notice = item("Music").getByRole("note", { name: /unavailable/ })
    expect(notice).toHaveTextContent("2 keys unavailable")
    expect(notice).toHaveAccessibleName(
      "2 keys unavailable in this profile: Key 3 on Main: workflow Deploy of profile Home; Key 5 on Main: project Shop",
    )
    expect(item("Desk").queryByRole("note", { name: /unavailable/ })).not.toBeInTheDocument()
  })

  it("warns about an unknown model and opens layouts", async () => {
    api.profileLayouts.mockResolvedValue([entry(old, 0)])
    const onOpen = vi.fn()
    const { user } = await renderLayouts(onOpen)
    expect(item("Old pad").getByText("Unknown model")).toBeInTheDocument()
    await user.click(item("Old pad").getByRole("button", { name: "Open Old pad" }))
    expect(onOpen).toHaveBeenCalledWith("old")
  })
})
