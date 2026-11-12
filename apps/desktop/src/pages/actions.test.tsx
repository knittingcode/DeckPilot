// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { clearFocus, peekFocus } from "@/lib/focus"
import type { Usage, Workflow } from "@/lib/types"
import { ActionsPage } from "@/pages/actions"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { ACTION_TYPES, action, builtInModules, profileView, stepWorkflow } from "@/test/fixtures"
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

  it("creates an action: type first, then name and parameters", async () => {
    const created = action("a-new", "Build", ACTION_TYPES.runCommand.id)
    api.createAction.mockResolvedValue(created)
    const { user, titlebarAction, context } = await renderLibrary()

    titlebarAction("actions.new")
    expect(screen.getByText("Step 1 of 2: choose what the action does")).toBeVisible()
    // Types of enabled modules only, grouped by module; Git is disabled.
    const chooser = screen.getByRole("group", { name: "Action types" })
    expect(
      within(chooser)
        .getAllByRole("region")
        .map((r) => r.getAttribute("aria-label")),
    ).toEqual(["General", "System", "Docker", "HTTP", "Notifications"])
    expect(within(chooser).queryByRole("button", { name: /Git status/ })).not.toBeInTheDocument()
    await user.type(screen.getByRole("textbox", { name: "Search action types" }), "command")
    expect(
      within(chooser)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Run commandRun command."])
    await user.click(within(chooser).getByRole("button", { name: /Run command/ }))

    expect(screen.getByText("Step 2 of 2: name it and set its parameters")).toBeVisible()
    expect(screen.getByText(/System › Run command/)).toBeInTheDocument()
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

  it("goes back to the type from the second step", async () => {
    const { user, titlebarAction } = await renderLibrary()
    titlebarAction("actions.new")
    const chooser = screen.getByRole("group", { name: "Action types" })
    await user.click(within(chooser).getByRole("button", { name: /Docker status/ }))
    expect(screen.getByText(/Docker › Docker status/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Change type" }))
    expect(screen.getByRole("textbox", { name: "Search action types" })).toBeInTheDocument()
  })

  it("does not offer deprecated types for new actions", async () => {
    const next = { ...ACTION_TYPES.log, id: "general.old", name: "Old log", deprecated: true }
    const withDeprecated = builtInModules({
      git: { enabled: false },
      general: { actionTypes: [ACTION_TYPES.log, next] },
    })
    api.getProfile.mockResolvedValue(library())
    const { user, titlebarAction } = renderWithApp(<ActionsPage />, { modules: withDeprecated })
    await screen.findByRole("textbox", { name: "Search actions" })
    titlebarAction("actions.new")
    const chooser = screen.getByRole("group", { name: "Action types" })
    const general = within(chooser).getByRole("region", { name: "General" })
    expect(within(general).getByRole("button", { name: /Log message/ })).toBeInTheDocument()
    expect(within(general).queryByRole("button", { name: /Old log/ })).not.toBeInTheDocument()
    await user.type(screen.getByRole("textbox", { name: "Search action types" }), "old")
    expect(screen.getByText("No types match.")).toBeInTheDocument()
  })

  it("points to the module settings when no module offers action types", async () => {
    api.getProfile.mockResolvedValue(library([]))
    const { user, context } = renderWithApp(<ActionsPage />, { modules: [] })
    await user.click(await screen.findByRole("button", { name: "Create action" }))
    expect(screen.getByText("No action types available")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open module settings" }))
    expect(context.openSettings).toHaveBeenCalledWith("modules")
  })

  it("edits, duplicates and deletes an unused action with a confirmation", async () => {
    const copy = action("a-copy", "Docker status (copy)", ACTION_TYPES.dockerStatus.id)
    api.duplicateAction.mockResolvedValue(copy)
    const { user } = await renderLibrary()

    await user.click(screen.getByRole("button", { name: /Docker status/ }))
    expect(screen.getByText("Docker status", { selector: TITLE })).toBeInTheDocument()
    api.getProfile.mockResolvedValue(library([docs, containers, repo, copy]))
    await user.click(screen.getByRole("button", { name: "Duplicate" }))
    expect(api.duplicateAction).toHaveBeenCalledWith(
      "profile-1",
      "a-docker",
      "Docker status (copy)",
    )
    expect(toast.success).toHaveBeenCalledWith("Action duplicated")
    // The copy is selected once the profile is reloaded.
    expect(await screen.findByText("Docker status (copy)", { selector: TITLE })).toBeInTheDocument()

    const [original] = within(screen.getByRole("region", { name: "Docker" })).getAllByRole("button")
    if (!original) throw new Error("no action")
    await user.click(original)
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const dialog = screen.getByRole("dialog", { name: "Delete “Docker status”?" })
    expect(dialog).toHaveTextContent("The action is removed from this profile.")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    expect(api.changeActionReferences).not.toHaveBeenCalled()
    expect(api.deleteAction).toHaveBeenCalledWith("profile-1", "a-docker")
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(toast.success).toHaveBeenCalledWith("Action deleted")
  })

  it("explains actions whose module is disabled", async () => {
    const { user } = await renderLibrary()
    await user.click(screen.getByRole("button", { name: /Shop status/ }))
    expect(screen.getByText(/The Git module is disabled/)).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
  })

  it("starts with an empty state that leads to creating the first action", async () => {
    const { user } = await renderLibrary(library([]))
    expect(screen.getByText("No actions yet")).toBeInTheDocument()
    expect(screen.getByText("Create your first action")).toBeInTheDocument()
    // Nothing is created by itself.
    expect(api.createAction).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Create action" }))
    expect(screen.getByRole("textbox", { name: "Search action types" })).toBeInTheDocument()
  })

  it("asks for a profile first", async () => {
    const { user, context } = renderWithApp(<ActionsPage />, { editingProfileId: null, modules })
    await user.click(screen.getByRole("button", { name: "Create a profile" }))
    expect(context.navigate).toHaveBeenCalledWith("profiles")
  })
})

describe("Action references", () => {
  const step: Usage = { type: "workflowStep", workflow: "w", workflowName: "Start", position: 1 }
  const key: Usage = {
    type: "layoutBinding",
    layout: "l",
    layoutName: "Main",
    page: "home",
    pageName: "Home",
    control: { kind: "key", index: 2 },
  }
  // A graph workflow with a trigger: the backend reports no usage for its node.
  const graphWorkflow: Workflow = {
    ...stepWorkflow("g", "Morning"),
    triggers: [{ type: "schedule", schedule: { every: "daily", at: "09:00" } }],
    graph: {
      version: 1,
      nodes: [
        {
          id: "open",
          label: "Open the docs",
          type: "action",
          action: "a-docs",
          policy: { onError: "stop", retry: { attempts: 0, backoffMs: 0 } },
          position: { x: 0, y: 0 },
        },
      ],
      edges: [],
    },
  }
  const used = profileView(
    { actions: [docs, containers, repo], workflows: [stepWorkflow("w", "Start"), graphWorkflow] },
    { actionUsages: { "a-docs": [step, key] } },
  )
  const node: Usage = {
    type: "workflowNode",
    workflow: "g",
    workflowName: "Morning",
    node: "open",
    nodeLabel: "Open the docs",
  }

  afterEach(() => {
    clearFocus("workflow")
    clearFocus("layout")
  })

  async function openDocs() {
    const rendered = await renderLibrary(used)
    await rendered.user.click(screen.getByRole("button", { name: /Open docs/ }))
    return rendered
  }

  it("lists keys, workflow steps and graph nodes, with triggers, and opens them", async () => {
    const { user, context } = await openDocs()
    const list = screen.getByRole("list", { name: "References" })
    expect(
      within(list)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual([
      "Workflow “Start”, step 1 Open",
      "Layout “Main”, page “Home”, Key 3 Open",
      "Workflow “Morning”, node “Open the docs”Workflow started by 1 trigger Open",
    ])
    // The count in the list includes the graph node.
    expect(screen.getByRole("button", { name: /^Open docs/ })).toHaveTextContent("3")

    await user.click(
      screen.getByRole("button", { name: "Open Workflow “Morning”, node “Open the docs”" }),
    )
    expect(context.navigate).toHaveBeenCalledWith("workflows")
    expect(peekFocus("workflow")).toEqual({ type: "workflow", workflow: "g" })

    await user.click(screen.getByRole("button", { name: "Open Layout “Main”, page “Home”, Key 3" }))
    expect(context.navigate).toHaveBeenCalledWith("designer")
    expect(peekFocus("layout")).toEqual({ type: "layout", layout: "l", page: "home" })
  })

  it("deletes only after every reference was replaced or unbound", async () => {
    api.changeActionReferences.mockResolvedValue([])
    const { user } = await openDocs()
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const dialog = screen.getByRole("dialog", { name: "Delete “Open docs”?" })
    expect(dialog).toHaveTextContent("used in 3 places")
    const apply = within(dialog).getByRole("button", { name: "Apply and delete" })
    expect(apply).toBeDisabled()
    expect(dialog).toHaveTextContent("3 references need a choice.")

    await user.click(
      within(dialog).getByRole("combobox", { name: "Change for Workflow “Start”, step 1" }),
    )
    // The action being deleted is not offered as its own replacement.
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Replace with “Docker status”",
      "Replace with “Shop status”",
      "Unbind",
    ])
    await user.click(screen.getByRole("option", { name: "Replace with “Docker status”" }))
    await user.click(
      within(dialog).getByRole("combobox", {
        name: "Change for Layout “Main”, page “Home”, Key 3",
      }),
    )
    await user.click(screen.getByRole("option", { name: "Unbind" }))
    expect(apply).toBeDisabled()
    await user.click(
      within(dialog).getByRole("combobox", {
        name: "Change for Workflow “Morning”, node “Open the docs”",
      }),
    )
    await user.click(screen.getByRole("option", { name: "Replace with “Shop status”" }))
    expect(api.changeActionReferences).not.toHaveBeenCalled()
    await user.click(apply)

    expect(api.changeActionReferences).toHaveBeenCalledWith("profile-1", "a-docs", [
      { usage: step, change: { type: "replace", action: "a-docker" } },
      { usage: key, change: { type: "unbind" } },
      { usage: node, change: { type: "replace", action: "a-git" } },
    ])
    expect(api.deleteAction).toHaveBeenCalledWith("profile-1", "a-docs")
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Action deleted")
    })
  })

  it("unbinds all references at once", async () => {
    api.changeActionReferences.mockResolvedValue([])
    const { user } = await openDocs()
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const dialog = screen.getByRole("dialog", { name: "Delete “Open docs”?" })
    await user.click(within(dialog).getByRole("combobox", { name: "For all references" }))
    await user.click(screen.getByRole("option", { name: "Unbind" }))
    await user.click(within(dialog).getByRole("button", { name: "Apply and delete" }))
    expect(api.changeActionReferences).toHaveBeenCalledWith(
      "profile-1",
      "a-docs",
      [step, key, node].map((usage) => ({ usage, change: { type: "unbind" } })),
    )
    expect(api.deleteAction).toHaveBeenCalledWith("profile-1", "a-docs")
  })

  it("keeps the action when references remain after the change", async () => {
    api.changeActionReferences.mockResolvedValue([key])
    const { user } = await openDocs()
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const dialog = screen.getByRole("dialog", { name: "Delete “Open docs”?" })
    await user.click(within(dialog).getByRole("combobox", { name: "For all references" }))
    await user.click(screen.getByRole("option", { name: "Replace with “Docker status”" }))
    await user.click(within(dialog).getByRole("button", { name: "Apply and delete" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Still used in Layout “Main”, page “Home”, Key 3. Nothing was deleted.",
    )
    expect(api.deleteAction).not.toHaveBeenCalled()
  })

  it("does not delete anything when cancelled", async () => {
    const { user } = await openDocs()
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const dialog = screen.getByRole("dialog", { name: "Delete “Open docs”?" })
    await user.click(within(dialog).getByRole("combobox", { name: "For all references" }))
    await user.click(screen.getByRole("option", { name: "Unbind" }))
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    expect(api.changeActionReferences).not.toHaveBeenCalled()
    expect(api.deleteAction).not.toHaveBeenCalled()
  })
})
