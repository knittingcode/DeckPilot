// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { Button } from "@/components/ui/button"
import type { SharedLayoutSummary } from "@/lib/types"
import { type EditorSelection, EditorSelectionContext } from "@/shell/editor-selection-context"
import { profileToOpen, useOpenLayout } from "@/shell/open-layout"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { profiles } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { assignment, sharedLayout } from "@/test/shared-layouts"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const PROFILES = profiles([
  { id: "profile-1", name: "Work" },
  { id: "profile-2", name: "Home" },
])

function selection(change: Partial<EditorSelection> = {}): EditorSelection {
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
    ...change,
  }
}

function Opener({ layout }: { layout: SharedLayoutSummary }) {
  const { open, dialog } = useOpenLayout()
  return (
    <>
      <Button
        onClick={() => {
          open(layout)
        }}
      >
        Open
      </Button>
      {dialog}
    </>
  )
}

function renderOpener(layout: SharedLayoutSummary, editor = selection()) {
  const view = renderWithApp(
    <EditorSelectionContext value={editor}>
      <Opener layout={layout} />
    </EditorSelectionContext>,
    { profiles: PROFILES },
  )
  return { ...view, editor }
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(cleanup)

describe("opening a shared layout in the Device Editor", () => {
  it("picks the profile being edited, else the default's, else the first", () => {
    const layout = sharedLayout({
      profiles: [
        assignment("profile-2", "Home"),
        assignment("profile-3", "Travel", { isDefault: true }),
      ],
    })
    expect(profileToOpen(layout, "profile-2")).toBe("profile-2")
    expect(profileToOpen(layout, "profile-1")).toBe("profile-3")
    expect(profileToOpen(sharedLayout({ profiles: [assignment("profile-2", "Home")] }), null)).toBe(
      "profile-2",
    )
    expect(profileToOpen(sharedLayout(), "profile-1")).toBeNull()
  })

  it("selects a layout the profile being edited uses", async () => {
    const layout = sharedLayout({ id: "desk", profiles: [assignment("profile-1", "Work")] })
    const { user, editor, context } = renderOpener(layout)
    await user.click(screen.getByRole("button", { name: "Open" }))
    expect(editor.selectProfile).toHaveBeenCalledWith("profile-1")
    expect(editor.showLayout).toHaveBeenCalledWith("desk", "elgato.streamdeck.mk2")
    expect(context.navigate).toHaveBeenCalledWith("designer")
    expect(api.assignLayout).not.toHaveBeenCalled()
  })

  it("switches to a profile that uses it", async () => {
    const layout = sharedLayout({ id: "desk", profiles: [assignment("profile-2", "Home")] })
    const { user, editor } = renderOpener(layout)
    await user.click(screen.getByRole("button", { name: "Open" }))
    expect(editor.selectProfile).toHaveBeenCalledWith("profile-2")
    expect(editor.showLayout).toHaveBeenCalledWith("desk", "elgato.streamdeck.mk2")
  })

  it("never replaces a layout with unsaved changes", async () => {
    const layout = sharedLayout({ id: "desk", profiles: [assignment("profile-1", "Work")] })
    const { user, editor, context } = renderOpener(
      layout,
      selection({ layoutId: "other", guard: vi.fn(() => false) }),
    )
    await user.click(screen.getByRole("button", { name: "Open" }))
    expect(editor.showLayout).not.toHaveBeenCalled()
    expect(context.navigate).not.toHaveBeenCalled()
  })

  it("asks to use a layout no profile uses in the profile being edited", async () => {
    api.assignLayout.mockResolvedValue(null)
    const layout = sharedLayout({ id: "spare", name: "Spare" })
    const { user, editor, context } = renderOpener(layout)
    await user.click(screen.getByRole("button", { name: "Open" }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Use “Spare” in Work?")
    expect(dialog).toHaveTextContent("No profile uses this layout yet.")
    await user.click(within(dialog).getByRole("button", { name: "Use in Work" }))
    await waitFor(() => {
      expect(editor.showLayout).toHaveBeenCalledWith("spare", "elgato.streamdeck.mk2")
    })
    expect(api.assignLayout).toHaveBeenCalledWith("profile-1", "spare")
    expect(context.refresh).toHaveBeenCalled()
    expect(context.navigate).toHaveBeenCalledWith("designer")
  })

  it("opens nothing when the user declines or the assignment fails", async () => {
    const layout = sharedLayout({ id: "spare", name: "Spare" })
    const { user, editor } = renderOpener(layout)
    await user.click(screen.getByRole("button", { name: "Open" }))
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Cancel" }),
    )
    expect(api.assignLayout).not.toHaveBeenCalled()

    api.assignLayout.mockRejectedValue(new Error("unknown device model"))
    await user.click(screen.getByRole("button", { name: "Open" }))
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Use in Work" }),
    )
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("“Spare” was not added to Work", {
        description: "Unknown device model.",
      })
    })
    expect(editor.showLayout).not.toHaveBeenCalled()
  })
})
