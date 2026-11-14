// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { SharedVariable } from "@/lib/types"
import { SharedVariablesSettings } from "@/pages/shared-variables"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { agentConnection, profileView, project } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"
import { type FakeVariable, fakeVariableAgent } from "@/test/variables"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

const shop = project("SHOP")

const root: SharedVariable = {
  definition: {
    key: "paths.root",
    label: "Code root",
    valueType: { type: "string" },
    scopes: ["global", "project"],
  },
  values: [
    { scope: { kind: "global" }, value: { type: "string", value: "~/code" } },
    {
      scope: { kind: "project", profile: "profile-1", project: shop.id },
      value: { type: "string", value: "~/code/shop" },
    },
  ],
  references: [
    {
      profile: "profile-1",
      profileName: "Work",
      use: { type: "action", action: "a1" },
      ownerName: "Open root",
      path: "parameters.path",
    },
  ],
}

const token: SharedVariable = {
  definition: {
    key: "deploy.token",
    valueType: { type: "string" },
    scopes: ["profile"],
    sensitive: true,
  },
  // A backend bug sending the value must not reveal it.
  values: [
    {
      scope: { kind: "profile", profile: "profile-1" },
      value: { type: "string", value: "hunter2-secret" },
    },
  ],
  references: [],
}

const github: SharedVariable = {
  definition: { key: "github.token", valueType: { type: "secretRef" }, scopes: ["global"] },
  values: [{ scope: { kind: "global" }, value: { type: "secretRef", value: "github" } }],
  references: [],
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  api.getProfile.mockResolvedValue(profileView({ projects: [shop] }))
  api.listCredentials.mockResolvedValue([
    { id: "github", description: "GitHub PAT", updatedAt: 0 },
    { id: "jira", updatedAt: 0 },
  ])
})
afterEach(cleanup)

async function renderPage(variables: SharedVariable[] = [root, token, github]) {
  api.listSharedVariables.mockResolvedValue(variables)
  const rendered = renderWithApp(<SharedVariablesSettings />)
  await screen.findByRole("textbox", { name: "Search variables" })
  return rendered
}

function table() {
  return screen.getByRole("table", { name: "Variables" })
}

/** The variable column of every row, in order. */
function rows() {
  return within(table())
    .getAllByRole("row")
    .slice(1)
    .map((row) => row.getAttribute("aria-label"))
}

describe("Shared Variables settings", () => {
  it("lists every variable in one table, searchable, with badges", async () => {
    const { user } = await renderPage()
    expect(api.listSharedVariables).toHaveBeenCalledWith("profile-1")
    const shared = ["shared.paths.root", "shared.deploy.token", "shared.github.token"]
    expect(rows().slice(-3)).toEqual(shared)
    const root = within(table()).getByRole("row", { name: "shared.paths.root" })
    expect(root).toHaveTextContent("Code root")
    expect(root).toHaveTextContent("Editable")
    expect(root).toHaveTextContent("Used 1×")
    expect(within(table()).getByRole("row", { name: "shared.deploy.token" })).toHaveTextContent(
      "Sensitive",
    )
    await user.type(screen.getByRole("textbox", { name: "Search variables" }), "github")
    expect(rows()).toEqual(["shared.github.token"])
    await user.type(screen.getByRole("textbox", { name: "Search variables" }), "zzz")
    expect(screen.getByText("No variables match.")).toBeInTheDocument()
  })

  it("starts empty", async () => {
    await renderPage([])
    expect(screen.getByText("No shared variables yet")).toBeInTheDocument()
  })

  it("validates a new variable before creating it", async () => {
    api.saveSharedVariable.mockResolvedValue(null)
    const { user } = await renderPage()
    await user.click(screen.getByRole("button", { name: "New variable" }))
    const key = screen.getByLabelText("Key")
    const create = screen.getByRole("button", { name: "Create variable" })

    await user.click(create)
    expect(screen.getByText("Enter a key.")).toBeInTheDocument()
    for (const [text, message] of [
      ["root", "Use 2 to 4 parts separated by dots, e.g. paths.root."],
      ["Paths.root", "Each part starts with a lowercase letter"],
      ["system.thing", "“system” is reserved."],
      ["paths.root", "A variable with this key exists."],
    ] as const) {
      await user.clear(key)
      await user.type(key, text)
      expect(screen.getByText(message, { exact: false })).toBeInTheDocument()
    }
    await user.clear(key)
    await user.type(key, "build.jobs")

    await user.click(screen.getByLabelText("Type"))
    await user.click(screen.getByRole("option", { name: "Integer" }))
    // Global is chosen by default; without any scope nothing is saved.
    await user.click(screen.getByRole("checkbox", { name: /^Global/ }))
    await user.click(create)
    expect(screen.getByText("Choose at least one scope.")).toBeInTheDocument()
    await user.click(screen.getByRole("checkbox", { name: /^Profile/ }))

    await user.click(screen.getByRole("checkbox", { name: /Default value/ }))
    await user.type(screen.getByLabelText("Default value"), "four")
    await user.click(create)
    expect(screen.getByText("Enter a whole number.")).toBeInTheDocument()
    expect(api.saveSharedVariable).not.toHaveBeenCalled()

    await user.clear(screen.getByLabelText("Default value"))
    await user.type(screen.getByLabelText("Default value"), "4")
    await user.click(create)
    expect(api.saveSharedVariable).toHaveBeenCalledWith({
      key: "build.jobs",
      label: "",
      description: "",
      valueType: { type: "integer" },
      scopes: ["profile"],
      sensitive: false,
      default: { type: "integer", value: 4 },
    })
    expect(toast.success).toHaveBeenCalledWith("Variable created")
  })

  it("edits a variable and its values per scope", async () => {
    api.saveSharedVariable.mockResolvedValue(null)
    const { user } = await renderPage()
    await user.click(screen.getByRole("button", { name: "Edit shared.paths.root" }))
    expect(screen.getByLabelText("Key")).toHaveAttribute("readonly")

    await user.clear(screen.getByLabelText("Label"))
    await user.type(screen.getByLabelText("Label"), "Root")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(api.saveSharedVariable).toHaveBeenCalledWith(
      expect.objectContaining({ key: "paths.root", label: "Root", scopes: ["global", "project"] }),
    )

    const values = screen.getByRole("list", { name: "Values" })
    expect(
      within(values)
        .getAllByRole("listitem")
        .map((li) => li.getAttribute("aria-label")),
    ).toEqual(["Global", "Project SHOP"])
    expect(within(values).getByText("~/code/shop")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Change Project SHOP" }))
    const field = screen.getByLabelText("Value for Project SHOP")
    expect(field).toHaveValue("~/code/shop")
    await user.clear(field)
    await user.type(field, "~/src/shop")
    await user.click(screen.getByRole("button", { name: "Save value" }))
    expect(api.setSharedVariableValue).toHaveBeenCalledWith(
      "paths.root",
      { kind: "project", profile: "profile-1", project: shop.id },
      { type: "string", value: "~/src/shop" },
    )

    await user.click(screen.getByRole("button", { name: "Clear Global" }))
    expect(api.clearSharedVariableValue).toHaveBeenCalledWith("paths.root", { kind: "global" })
  })

  it("blocks deleting a referenced variable and lists where it is used", async () => {
    const { user } = await renderPage()
    await user.click(screen.getByRole("button", { name: "Edit shared.paths.root" }))
    expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled()
    expect(screen.getByRole("list", { name: "Usages" })).toHaveTextContent(
      "Work: Action “Open root”, parameters.path",
    )
  })

  it("deletes an unused variable after a confirmation", async () => {
    const { user } = await renderPage()
    await user.click(screen.getByRole("button", { name: "Edit shared.github.token" }))
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const dialog = screen.getByRole("alertdialog", { name: "Delete “shared.github.token”?" })
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    expect(api.deleteSharedVariable).toHaveBeenCalledWith("github.token")
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Variable deleted")
    })
  })

  it("never shows sensitive values, not even in the field to change them", async () => {
    const { user } = await renderPage()
    expect(document.body).not.toHaveTextContent("hunter2-secret")
    await user.click(screen.getByRole("button", { name: "Edit shared.deploy.token" }))
    expect(screen.getByRole("list", { name: "Values" })).toHaveTextContent("Set (redacted)")
    expect(document.body).not.toHaveTextContent("hunter2-secret")
    // No default for sensitive variables.
    expect(screen.queryByRole("checkbox", { name: /Default value/ })).not.toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: /^Change Profile/ }))
    const field = screen.getByLabelText(/^Value for Profile/)
    expect(field).toHaveAttribute("type", "password")
    expect(field).toHaveValue("")
    expect(document.body.innerHTML).not.toContain("hunter2-secret")
    await user.type(field, "new-secret")
    await user.click(screen.getByRole("button", { name: "Save value" }))
    expect(api.setSharedVariableValue).toHaveBeenCalledWith(
      "deploy.token",
      { kind: "profile", profile: "profile-1" },
      { type: "string", value: "new-secret" },
    )
  })

  it("stores credential names for secret references, never secrets", async () => {
    const { user } = await renderPage()
    await user.click(screen.getByRole("button", { name: "Edit shared.github.token" }))
    expect(screen.getByText(/The secret stays in the credential store/)).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Values" })).toHaveTextContent("Credential github")
    // A secret reference is never sensitive itself: it holds only a name.
    expect(screen.queryByLabelText("Sensitive")).not.toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Change Global" }))
    await user.click(screen.getByLabelText("Value for Global"))
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
      "github — GitHub PAT",
      "jira",
    ])
    await user.click(screen.getByRole("option", { name: "jira" }))
    await user.click(screen.getByRole("button", { name: "Save value" }))
    expect(api.setSharedVariableValue).toHaveBeenCalledWith(
      "github.token",
      { kind: "global" },
      { type: "secretRef", value: "jira" },
    )
    // Only credential metadata was ever asked for.
    expect(api.listCredentials).toHaveBeenCalled()
    expect(api.setCredential).not.toHaveBeenCalled()
  })

  it("lists system variables read-only without an agent", async () => {
    await renderPage()
    const row = within(table()).getByRole("row", { name: "system.project.current.id" })
    // Owned by the projects module, even when an older agent cannot say so.
    expect(row).toHaveTextContent("projects module")
    expect(row).toHaveTextContent("Read-only")
    expect(row).toHaveTextContent("Not available from this agent")
    expect(within(row).queryByRole("button", { name: /^Edit/ })).not.toBeInTheDocument()
    expect(
      within(row).getByRole("button", { name: "Copy system.project.current.id" }),
    ).toBeEnabled()
    expect(within(table()).getByRole("row", { name: "workflow.execution.id" })).toHaveTextContent(
      "Only while the workflow runs",
    )
  })

  it("shows the backend's error when variables cannot be loaded", async () => {
    api.listSharedVariables.mockRejectedValue("unknown command list_shared_variables")
    renderWithApp(<SharedVariablesSettings />)
    expect(await screen.findByText("unknown command list_shared_variables")).toBeInTheDocument()
  })
})

/** Core and module variables as a protocol 9 agent lists them. */
function registryVariables(): FakeVariable[] {
  const projects = { kind: "module", module: "projects" } as const
  const key: FakeVariable = {
    entry: {
      reference: "projects.current.key",
      label: "Project key",
      description: "Key of the project in the context",
      group: "module",
      valueType: { type: "string" },
      module: "projects",
      owner: projects,
      readOnly: true,
      declaredScope: "project",
      availability: "available",
    },
    resolve: () => ({ type: "string", value: "SHOP" }),
  }
  return [
    key,
    {
      entry: {
        ...key.entry,
        reference: "system.project.current.key",
        group: "system",
        deprecated: { canonical: "projects.current.key", since: "0.6.0" },
      },
      resolve: key.resolve,
    },
    {
      entry: {
        reference: "system.platform",
        label: "Platform",
        group: "system",
        valueType: { type: "string" },
        owner: { kind: "core" },
        readOnly: true,
        declaredScope: "machine",
        availability: "available",
      },
      resolve: () => ({ type: "string", value: "macOs" }),
    },
    {
      entry: {
        reference: "vault.token",
        label: "Vault token",
        group: "module",
        valueType: { type: "string" },
        owner: { kind: "module", module: "vault" },
        readOnly: true,
        sensitive: true,
        declaredScope: "machine",
        availability: "available",
      },
      resolve: () => ({ type: "string", value: "vault-secret-value" }),
    },
    {
      entry: {
        reference: "workSession.state",
        label: "Work session state",
        group: "module",
        valueType: { type: "string" },
        owner: { kind: "module", module: "work-session" },
        readOnly: true,
        declaredScope: "workSession",
        availability: "unavailable",
      },
      resolve: () => ({ status: "unavailable", owner: "work-session" }),
    },
  ]
}

describe("Variables page with the unified registry", () => {
  it("shows owners, values when safe, and folds legacy ids into canonical rows", async () => {
    fakeVariableAgent(registryVariables()).install(api)
    api.listSharedVariables.mockResolvedValue([root])
    const { user } = renderWithApp(<SharedVariablesSettings />, {
      agent: agentConnection({ capabilities: ["variables", "variableRegistry"] }),
      modules: [{ id: "projects", name: "Projects" } as never],
    })
    await screen.findByRole("table", { name: "Variables" })
    expect(rows()).toEqual([
      "projects.current.key",
      "system.platform",
      "vault.token",
      "workSession.state",
      "shared.paths.root",
    ])
    const key = within(table()).getByRole("row", { name: "projects.current.key" })
    expect(key).toHaveTextContent("Projects module")
    expect(key).toHaveTextContent("Read-only")
    expect(key).toHaveTextContent("{{ system.project.current.key }} (deprecated)")
    await waitFor(() => {
      expect(key).toHaveTextContent("SHOP")
    })
    expect(within(key).queryByRole("button", { name: /^Edit/ })).not.toBeInTheDocument()
    expect(within(table()).getByRole("row", { name: "system.platform" })).toHaveTextContent(
      "DeckPilot",
    )
    expect(within(table()).getByRole("row", { name: "vault.token" })).toHaveTextContent(
      "Redacted (sensitive)",
    )
    expect(document.body).not.toHaveTextContent("vault-secret-value")
    // Sensitive variables are not even asked for.
    for (const [, items] of api.previewReferences.mock.calls) {
      expect(items.map((i) => i.reference)).not.toContain("vault.token")
    }
    const session = within(table()).getByRole("row", { name: "workSession.state" })
    await waitFor(() => {
      expect(session).toHaveTextContent("Unavailable")
    })
    expect(session).toHaveTextContent("Its module is disabled or not installed")

    // Filters by owner, type and scope.
    await user.click(screen.getByRole("combobox", { name: "Owner filter" }))
    await user.click(screen.getByRole("option", { name: "You" }))
    expect(rows()).toEqual(["shared.paths.root"])
    await user.click(screen.getByRole("combobox", { name: "Owner filter" }))
    await user.click(screen.getByRole("option", { name: "Any owner" }))
    await user.click(screen.getByRole("combobox", { name: "Scope filter" }))
    await user.click(screen.getByRole("option", { name: "This computer" }))
    expect(rows()).toEqual(["system.platform", "vault.token"])
    await user.click(screen.getByRole("combobox", { name: "Scope filter" }))
    await user.click(screen.getByRole("option", { name: "Project" }))
    expect(rows()).toEqual(["projects.current.key", "shared.paths.root"])
    // A legacy id finds its canonical row.
    await user.type(screen.getByRole("textbox", { name: "Search variables" }), "system.project")
    expect(rows()).toEqual(["projects.current.key"])
  })

  it("copies references and edits only user variables", async () => {
    fakeVariableAgent(registryVariables()).install(api)
    api.listSharedVariables.mockResolvedValue([root])
    const { user } = renderWithApp(<SharedVariablesSettings />, {
      agent: agentConnection({ capabilities: ["variables", "variableRegistry"] }),
    })
    await screen.findByRole("table", { name: "Variables" })
    // The registry rows come from a second request that can finish after the
    // table is first shown.
    await user.click(await screen.findByRole("button", { name: "Copy projects.current.key" }))
    // user-event's clipboard stands in for the system's.
    expect(await navigator.clipboard.readText()).toBe("{{ projects.current.key }}")
    expect(toast.success).toHaveBeenCalledWith("Reference copied")
    await user.click(screen.getByRole("button", { name: "Edit shared.paths.root" }))
    expect(screen.getByLabelText("Key")).toHaveValue("paths.root")
  })
})
