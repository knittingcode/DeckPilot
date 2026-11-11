// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { Conversion } from "@/lib/types"
import { WorkflowsPage } from "@/pages/workflows"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { ACTION_TYPES, action, profileView, stepWorkflow } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"
import type { BuilderProps } from "@/workflow-builder/builder"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
// The graph editor has its own tests; here it only shows what it was given.
vi.mock("@/workflow-builder/workflow-run", () => ({
  WorkflowRun: (props: Pick<BuilderProps, "initial" | "graph" | "isNew" | "notes" | "toolbar">) => (
    <section aria-label="Graph editor">
      {props.toolbar}
      <p>Editing {props.initial.name}</p>
      <p>{props.isNew ? "Unsaved workflow" : "Saved workflow"}</p>
      <p>{props.graph.nodes.length} nodes</p>
      {props.notes.map((note) => (
        <p key={note}>{note}</p>
      ))}
    </section>
  ),
}))

const log = action("a-log", "Say hello", ACTION_TYPES.log.id)
const url = action("a-url", "Open docs", ACTION_TYPES.openUrl.id)
const git = action("a-git", "Repo status", ACTION_TYPES.gitStatus.id)
const morning = stepWorkflow("w-morning", "Morning", ["a-log"])

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(cleanup)

async function renderWorkflows(workflows = [morning]) {
  api.getProfile.mockResolvedValue(profileView({ actions: [log, url, git], workflows }))
  // Git is not enabled: its actions cannot be added.
  api.listActionTypes.mockResolvedValue([ACTION_TYPES.log, ACTION_TYPES.openUrl])
  const rendered = renderWithApp(<WorkflowsPage />)
  await screen.findByRole("combobox", { name: "Workflow" })
  return rendered
}

describe("Workflows page", () => {
  it("edits a step workflow: adds a step, validates and saves", async () => {
    api.saveWorkflow.mockResolvedValue(morning)
    const { user, context } = await renderWorkflows()
    expect(screen.getByText(/This workflow runs its steps in order/)).toBeInTheDocument()

    await user.click(screen.getByRole("combobox", { name: "Action to add" }))
    expect(screen.getByRole("option", { name: "Repo status (unavailable)" })).toHaveAttribute(
      "aria-disabled",
      "true",
    )
    await user.click(screen.getByRole("option", { name: "Open docs" }))
    await user.click(screen.getByRole("button", { name: "Add step" }))
    expect(screen.getByRole("combobox", { name: "Action of step 2" })).toHaveTextContent(
      "Open docs",
    )

    const name = screen.getByLabelText("Name")
    await user.clear(name)
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(screen.getByText("Enter a name.")).toBeInTheDocument()
    expect(api.saveWorkflow).not.toHaveBeenCalled()

    await user.type(name, "Morning routine")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(api.saveWorkflow).toHaveBeenCalledWith(
      "profile-1",
      expect.objectContaining({
        id: "w-morning",
        name: "Morning routine",
        steps: [
          expect.objectContaining({ action: "a-log" }),
          expect.objectContaining({ action: "a-url", onFailure: "stop" }),
        ],
      }),
    )
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Workflow saved")
    })
    expect(context.refresh).toHaveBeenCalled()
  })

  it("reorders and removes steps with labelled buttons", async () => {
    const { user } = await renderWorkflows([stepWorkflow("w-two", "Two", ["a-log", "a-url"])])
    expect(screen.getByRole("button", { name: "Drag step 1" })).toBeInTheDocument()
    await user.click(screen.getAllByRole("button", { name: "Move down" })[0] as HTMLElement)
    expect(screen.getByRole("combobox", { name: "Action of step 1" })).toHaveTextContent(
      "Open docs",
    )
    await user.click(screen.getByRole("button", { name: "Remove step 2" }))
    expect(screen.queryByRole("combobox", { name: "Action of step 2" })).not.toBeInTheDocument()
  })

  it("opens a step workflow in the graph editor", async () => {
    const conversion: Conversion = {
      graph: {
        version: 1,
        nodes: [
          { id: "start", type: "trigger", position: { x: 0, y: 0 } },
          {
            id: "s1",
            type: "action",
            action: "a-log",
            policy: { onError: "stop", retry: { attempts: 1, backoffMs: 0 } },
            position: { x: 0, y: 100 },
          },
        ],
        edges: [],
      },
      notes: ["Step 1 runs after the trigger."],
    }
    api.convertWorkflow.mockResolvedValue(conversion)
    const { user } = await renderWorkflows()
    await user.click(screen.getByRole("button", { name: "Open in graph editor" }))
    expect(api.convertWorkflow).toHaveBeenCalledWith("profile-1", morning)
    const editor = await screen.findByRole("region", { name: "Graph editor" })
    expect(within(editor).getByText("Editing Morning")).toBeInTheDocument()
    expect(within(editor).getByText("2 nodes")).toBeInTheDocument()
    expect(within(editor).getByText("Step 1 runs after the trigger.")).toBeInTheDocument()
  })

  it("reports a failed conversion and stays in the step editor", async () => {
    api.convertWorkflow.mockRejectedValue(new Error("Unknown step"))
    const { user } = await renderWorkflows()
    await user.click(screen.getByRole("button", { name: "Open in graph editor" }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("Unknown step")
    })
    expect(screen.queryByRole("region", { name: "Graph editor" })).not.toBeInTheDocument()
  })

  it("starts new workflows in the graph editor", async () => {
    const { titlebarAction } = await renderWorkflows()
    titlebarAction("workflows.new")
    const editor = await screen.findByRole("region", { name: "Graph editor" })
    expect(within(editor).getByText("Unsaved workflow")).toBeInTheDocument()
    expect(within(editor).getByText("Editing New workflow")).toBeInTheDocument()
  })

  it("duplicates and deletes a step workflow", async () => {
    const copy = stepWorkflow("w-copy", "Morning (copy)", ["a-log"])
    api.duplicateWorkflow.mockResolvedValue(copy)
    const { user } = await renderWorkflows()
    api.getProfile.mockResolvedValue(
      profileView({ actions: [log, url, git], workflows: [morning, copy] }),
    )
    await user.click(screen.getByRole("button", { name: "Duplicate" }))
    expect(api.duplicateWorkflow).toHaveBeenCalledWith("profile-1", "w-morning", "Morning (copy)")
    // The copy is opened once the profile is reloaded.
    await waitFor(() => {
      expect(screen.getByRole("combobox", { name: "Workflow" })).toHaveTextContent("Morning (copy)")
    })
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const confirm = screen.getByRole("alertdialog", { name: "Delete “Morning (copy)”?" })
    await user.click(within(confirm).getByRole("button", { name: "Delete" }))
    expect(api.deleteWorkflow).toHaveBeenCalledWith("profile-1", "w-copy")
  })
})
