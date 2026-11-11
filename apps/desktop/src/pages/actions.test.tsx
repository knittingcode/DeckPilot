// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { ActionsPage } from "@/pages/actions"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { ACTION_TYPES, action, builtInModules, profileView } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const docs = action("a-docs", "Open docs", ACTION_TYPES.openUrl.id, {
  parameters: { url: { type: "text", value: "https://example.invalid" } },
})
const containers = action("a-docker", "Docker status", ACTION_TYPES.dockerStatus.id)
const repo = action("a-git", "Shop status", ACTION_TYPES.gitStatus.id)

const modules = builtInModules({ git: { enabled: false } })
/** The title of the editor card. */
const TITLE = "[data-slot=card-title]"

function library(actions = [docs, containers, repo]) {
  return profileView(
    { actions },
    {
      actionUsages: {
        "a-docs": [{ type: "workflowStep", workflow: "w", workflowName: "Start", position: 1 }],
      },
    },
  )
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(cleanup)

async function renderLibrary(view = library()) {
  api.getProfile.mockResolvedValue(view)
  const rendered = renderWithApp(<ActionsPage />, { modules })
  await screen.findByRole("textbox", { name: "Search actions" })
  return rendered
}

describe("Action Library", () => {
  it("lists actions by category with usage and availability, and filters them", async () => {
    const { user } = await renderLibrary()
    expect(api.getProfile).toHaveBeenCalledWith("profile-1")
    const system = screen.getByRole("region", { name: "System" })
    expect(within(system).getByRole("button", { name: /Open docs/ })).toHaveTextContent("1")
    const git = screen.getByRole("region", { name: "Git" })
    expect(within(git).getByText("Unavailable")).toHaveAttribute("title", "Its module is disabled")

    await user.type(screen.getByRole("textbox", { name: "Search actions" }), "docker")
    expect(screen.queryByRole("region", { name: "System" })).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Docker status/ })).toBeInTheDocument()

    await user.clear(screen.getByRole("textbox", { name: "Search actions" }))
    await user.click(screen.getByRole("combobox", { name: "Filter by category" }))
    await user.click(screen.getByRole("option", { name: "Git" }))
    expect(screen.getAllByRole("region").map((r) => r.getAttribute("aria-label"))).toEqual(["Git"])

    await user.type(screen.getByRole("textbox", { name: "Search actions" }), "zzz")
    expect(screen.getByText("No actions match.")).toBeInTheDocument()
  })

  it("creates an action: module, then type, with required parameters checked", async () => {
    const created = action("a-new", "Build", ACTION_TYPES.runCommand.id)
    api.createAction.mockResolvedValue(created)
    const { user, titlebarAction, context } = await renderLibrary()

    titlebarAction("actions.new")
    expect(screen.getByText("Choose a module and an action type, then configure it")).toBeVisible()
    expect(screen.getByRole("combobox", { name: "Action type" })).toBeDisabled()

    await user.click(screen.getByRole("combobox", { name: "Module" }))
    // Only enabled modules that provide action types; Git is disabled.
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
      "General",
      "System",
      "Docker",
      "HTTP",
      "Notifications",
    ])
    await user.click(screen.getByRole("option", { name: "System" }))
    await user.click(screen.getByRole("combobox", { name: "Action type" }))
    await user.click(screen.getByRole("option", { name: "Run command" }))

    await user.click(screen.getByRole("button", { name: "Create action" }))
    expect(screen.getByText("Enter a name.")).toBeInTheDocument()
    expect(screen.getByText("Command is required.")).toBeInTheDocument()
    expect(api.createAction).not.toHaveBeenCalled()

    await user.type(screen.getByLabelText("Name"), "Build")
    await user.type(screen.getByLabelText(/^Command/), "make build")
    api.getProfile.mockResolvedValue(library([docs, containers, repo, created]))
    await user.click(screen.getByRole("button", { name: "Create action" }))

    expect(api.createAction).toHaveBeenCalledWith(
      "profile-1",
      expect.objectContaining({
        name: "Build",
        actionType: "system.run-command",
        parameters: { command: { type: "text", value: "make build" } },
      }),
    )
    expect(toast.success).toHaveBeenCalledWith("Action created")
    expect(context.refresh).toHaveBeenCalled()
    // The new action is selected once the profile is reloaded.
    expect(await screen.findByText("Build", { selector: TITLE })).toBeInTheDocument()
  })

  it("edits, duplicates and deletes an action with a confirmation", async () => {
    const copy = action("a-copy", "Docker status (copy)", ACTION_TYPES.dockerStatus.id)
    api.duplicateAction.mockResolvedValue(copy)
    const { user } = await renderLibrary()

    await user.click(screen.getByRole("button", { name: /Docker status/ }))
    expect(screen.getByText("Docker status", { selector: TITLE })).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Duplicate" }))
    expect(api.duplicateAction).toHaveBeenCalledWith(
      "profile-1",
      "a-docker",
      "Docker status (copy)",
    )
    await waitFor(() => {
      expect(api.getProfile).toHaveBeenCalledTimes(2)
    })

    await user.click(screen.getByRole("button", { name: /Docker status/ }))
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const dialog = screen.getByRole("alertdialog", { name: "Delete “Docker status”?" })
    // Focus moves into the dialog, onto a safe choice.
    expect(dialog).toContainElement(document.activeElement as HTMLElement)
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    expect(api.deleteAction).toHaveBeenCalledWith("profile-1", "a-docker")
    await waitFor(() => {
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    })
    expect(toast.success).toHaveBeenCalledWith("Action deleted")
  })

  it("explains actions whose module is disabled and blocks deleting used actions", async () => {
    const { user } = await renderLibrary()
    await user.click(screen.getByRole("button", { name: /Shop status/ }))
    expect(screen.getByText(/The Git module is disabled/)).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()

    await user.click(screen.getByRole("button", { name: /Open docs/ }))
    expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled()
  })

  it("starts with an empty state and the editor for a new action", async () => {
    await renderLibrary(library([]))
    expect(screen.getByText("No actions yet")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Create action" })).toBeInTheDocument()
  })

  it("asks for a profile first", async () => {
    const { user, context } = renderWithApp(<ActionsPage />, { editingProfileId: null, modules })
    await user.click(screen.getByRole("button", { name: "Create a profile" }))
    expect(context.navigate).toHaveBeenCalledWith("profiles")
  })
})
