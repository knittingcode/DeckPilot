// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { EditorSelection } from "@/shell/editor-selection-context"
import { EditorSelectionContext } from "@/shell/editor-selection-context"
import { ProfilesSettings } from "@/pages/profiles"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { profileView, profiles } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { assignment, sharedLayout } from "@/test/shared-layouts"
import { resetToasts } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const PROFILES = profiles([
  { id: "profile-1", name: "Work" },
  { id: "profile-2", name: "Home" },
])
const desk = sharedLayout({
  id: "desk",
  name: "Desk",
  profiles: [assignment("profile-1", "Work", { isDefault: true }), assignment("profile-2", "Home")],
})
const spare = sharedLayout({ id: "spare", name: "Spare" })

function selection(): EditorSelection {
  return {
    models: { status: "ready", data: [] },
    modelId: null,
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
  }
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  api.getProfile.mockResolvedValue(profileView({ id: "profile-1", name: "Work" }))
  api.profileLayouts.mockResolvedValue([
    { layout: desk, position: 0, isDefault: true, unavailable: [] },
  ])
  api.listSharedLayouts.mockResolvedValue([desk, spare])
  api.assignLayout.mockResolvedValue(null)
})
afterEach(cleanup)

function renderPage() {
  const editor = selection()
  const view = renderWithApp(
    <EditorSelectionContext value={editor}>
      <ProfilesSettings />
    </EditorSelectionContext>,
    { profiles: PROFILES },
  )
  return { ...view, editor }
}

describe("Settings › Profiles", () => {
  it("shows the layouts of the profile being edited and assigns more", async () => {
    const { user } = renderPage()
    const section = await screen.findByRole("region", { name: "Layouts" })
    const row = await within(section).findByRole("listitem", { name: "Desk" })
    expect(api.profileLayouts).toHaveBeenCalledWith("profile-1")
    expect(within(row).getByText("Default for Stream Deck MK.2")).toBeInTheDocument()
    expect(within(row).getByText("Also in Home")).toBeInTheDocument()
    // The plain count is gone: the section lists them.
    expect(screen.queryByText("Layouts", { selector: "dt" })).not.toBeInTheDocument()

    await user.click(within(section).getByRole("button", { name: "Assign layout" }))
    await user.click(await screen.findByRole("menuitem", { name: /Spare/ }))
    await waitFor(() => {
      expect(api.assignLayout).toHaveBeenCalledWith("profile-1", "spare")
    })
  })

  it("opens a layout of the profile in the Device Editor", async () => {
    const { user, editor, context } = renderPage()
    const row = await screen.findByRole("listitem", { name: "Desk" })
    await user.click(within(row).getByRole("button", { name: "Open Desk" }))
    expect(editor.selectProfile).toHaveBeenCalledWith("profile-1")
    expect(editor.showLayout).toHaveBeenCalledWith("desk", "elgato.streamdeck.mk2")
    expect(context.navigate).toHaveBeenCalledWith("designer")
  })

  it("says that duplicating copies the layouts and deleting keeps them", async () => {
    const { user } = renderPage()
    await screen.findByRole("region", { name: "Layouts" })
    await user.click(screen.getByRole("button", { name: "More for Home" }))
    await user.click(await screen.findByRole("menuitem", { name: /Duplicate/ }))
    expect(await screen.findByRole("dialog")).toHaveTextContent("independent copies of its layouts")
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "More for Home" }))
    await user.click(await screen.findByRole("menuitem", { name: /Delete/ }))
    const confirm = await screen.findByRole("alertdialog")
    expect(confirm).toHaveTextContent("Its layouts are kept: they stay in Layouts")
    expect(confirm).not.toHaveTextContent("projects and layouts are deleted")
  })
})
