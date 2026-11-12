// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { clearFocus, peekFocus } from "@/lib/focus"
import type { DeviceLayout, ReferenceSite, Workflow } from "@/lib/types"
import { ActionsPage } from "@/pages/actions"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { ACTION_TYPES, action, builtInModules, profileView, stepWorkflow } from "@/test/fixtures"
import { type AppRender, renderWithApp } from "@/test/render"
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
  return profileView({ actions, workflows: [stepWorkflow("w", "Start", ["a-docs"])] })
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
    expect(api.replaceActionReferences).not.toHaveBeenCalled()
    expect(api.unbindActionReferences).not.toHaveBeenCalled()
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
  const layout = {
    id: "l",
    name: "Main",
    model: "mk2",
    homePage: "home",
    pages: [
      {
        id: "home",
        name: "Home",
        bindings: [
          {
            control: { kind: "key", index: 2 },
            trigger: { type: "press" },
            target: { type: "action", action: "a-docs" },
          },
        ],
      },
    ],
  } as DeviceLayout
  // A graph workflow with a trigger, and a step workflow.
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
  const used = profileView({
    actions: [docs, containers, repo],
    workflows: [stepWorkflow("w", "Start", ["a-docs"]), graphWorkflow],
    layouts: [layout],
  })
  const keySite: ReferenceSite = {
    type: "binding",
    layout: "l",
    page: "home",
    control: { kind: "key", index: 2 },
    trigger: { type: "press" },
  }
  const stepSite: ReferenceSite = { type: "step", workflow: "w", index: 0 }
  const nodeSite: ReferenceSite = { type: "node", workflow: "g", node: "open" }
  const noChange = { changed: [], clearedSettings: [] }

  afterEach(() => {
    clearFocus("workflow")
    clearFocus("layout")
  })

  async function openDocs() {
    const rendered = await renderLibrary(used)
    await rendered.user.click(screen.getByRole("button", { name: /Open docs/ }))
    return rendered
  }

  async function openDelete(user: AppRender["user"]) {
    await user.click(screen.getByRole("button", { name: "Delete" }))
    return screen.getByRole("dialog", { name: "Delete “Open docs”?" })
  }

  it("lists keys, workflow steps, graph nodes and triggers, and opens them", async () => {
    const { user, context } = await openDocs()
    const list = screen.getByRole("list", { name: "References" })
    expect(
      within(list)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual([
      "Layout “Main”, page “Home”, Key 3 Open",
      "Workflow “Start”, step 1 Open",
      "Workflow “Morning”, node “Open the docs” Open",
      "Workflow “Morning” starts by trigger: Daily at 09:00 Open",
    ])
    // The count in the list: direct uses, not the trigger.
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
    api.replaceActionReferences.mockResolvedValue(noChange)
    api.unbindActionReferences.mockResolvedValue(noChange)
    const { user } = await openDocs()
    const dialog = await openDelete(user)
    expect(dialog).toHaveTextContent("used in 3 places")
    // The trigger is listed but follows its workflow.
    expect(within(dialog).getByRole("list", { name: "Triggers" })).toHaveTextContent(
      "Daily at 09:00",
    )
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
    // A graph node cannot be unbound (it can be opened in the workflow editor instead).
    expect(screen.queryByRole("option", { name: "Unbind" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("option", { name: "Replace with “Docker status”" }))
    expect(api.replaceActionReferences).not.toHaveBeenCalled()
    await user.click(apply)

    await waitFor(() => {
      expect(api.deleteAction).toHaveBeenCalledWith("profile-1", "a-docs")
    })
    expect(api.replaceActionReferences).toHaveBeenCalledExactlyOnceWith(
      "profile-1",
      "a-docs",
      "a-docker",
      { type: "sites", sites: [stepSite, nodeSite] },
    )
    expect(api.unbindActionReferences).toHaveBeenCalledWith("profile-1", "a-docs", {
      type: "sites",
      sites: [keySite],
    })
    expect(toast.success).toHaveBeenCalledWith("Action deleted")
  })

  it("unbinds all keys and steps at once; graph nodes still need a replacement", async () => {
    api.replaceActionReferences.mockResolvedValue({
      changed: [nodeSite],
      clearedSettings: [nodeSite],
    })
    api.unbindActionReferences.mockResolvedValue(noChange)
    const { user } = await openDocs()
    const dialog = await openDelete(user)
    await user.click(within(dialog).getByRole("combobox", { name: "For all references" }))
    await user.click(screen.getByRole("option", { name: "Unbind" }))
    const apply = within(dialog).getByRole("button", { name: "Apply and delete" })
    expect(apply).toBeDisabled()
    expect(dialog).toHaveTextContent("1 reference needs a choice.")
    await user.click(
      within(dialog).getByRole("combobox", {
        name: "Change for Workflow “Morning”, node “Open the docs”",
      }),
    )
    await user.click(screen.getByRole("option", { name: "Replace with “Shop status”" }))
    await user.click(apply)
    await waitFor(() => {
      expect(api.deleteAction).toHaveBeenCalledWith("profile-1", "a-docs")
    })
    expect(api.replaceActionReferences).toHaveBeenCalledWith("profile-1", "a-docs", "a-git", {
      type: "sites",
      sites: [nodeSite],
    })
    expect(api.unbindActionReferences).toHaveBeenCalledWith("profile-1", "a-docs", {
      type: "sites",
      sites: [keySite, stepSite],
    })
    // Overrides of another type's parameters were dropped; the user is told.
    expect(toast.warning).toHaveBeenCalledWith(expect.stringContaining("1 step or node"))
  })

  it("opens a graph node in the workflow editor from the dialog", async () => {
    const { user, context } = await openDocs()
    const dialog = await openDelete(user)
    await user.click(
      within(dialog).getByRole("button", {
        name: "Open in workflow editor: Workflow “Morning”, node “Open the docs”",
      }),
    )
    expect(context.navigate).toHaveBeenCalledWith("workflows")
    expect(peekFocus("workflow")).toEqual({ type: "workflow", workflow: "g" })
    expect(api.deleteAction).not.toHaveBeenCalled()
  })

  it("keeps the action when the backend refuses", async () => {
    api.replaceActionReferences.mockResolvedValue(noChange)
    api.deleteAction.mockRejectedValue(new Error("the action is still used"))
    const { user } = await openDocs()
    const dialog = await openDelete(user)
    await user.click(within(dialog).getByRole("combobox", { name: "For all references" }))
    await user.click(screen.getByRole("option", { name: "Replace with “Docker status”" }))
    await user.click(within(dialog).getByRole("button", { name: "Apply and delete" }))
    // What failed, the backend's reason, and that the replacement made before stays.
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The action was not deleted: The action is still used. The changes made before were kept; close this dialog to see the current references.",
    )
    expect(toast.success).not.toHaveBeenCalledWith("Action deleted")
  })

  it("says nothing changed when the first step fails", async () => {
    api.replaceActionReferences.mockRejectedValue("a selected place no longer uses this action")
    const { user } = await openDocs()
    const dialog = await openDelete(user)
    await user.click(within(dialog).getByRole("combobox", { name: "For all references" }))
    await user.click(screen.getByRole("option", { name: "Replace with “Docker status”" }))
    await user.click(within(dialog).getByRole("button", { name: "Apply and delete" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The references were not replaced: A selected place no longer uses this action. Nothing was changed.",
    )
    expect(api.deleteAction).not.toHaveBeenCalled()
  })

  it("does not change anything when cancelled", async () => {
    const { user } = await openDocs()
    const dialog = await openDelete(user)
    await user.click(within(dialog).getByRole("combobox", { name: "For all references" }))
    await user.click(screen.getByRole("option", { name: "Unbind" }))
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    expect(api.replaceActionReferences).not.toHaveBeenCalled()
    expect(api.unbindActionReferences).not.toHaveBeenCalled()
    expect(api.deleteAction).not.toHaveBeenCalled()
  })
})
