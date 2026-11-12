// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { TitlebarContext } from "@/hooks/use-titlebar"
import { AppContext } from "@/lib/app-context"
import { TitlebarStore } from "@/lib/titlebar"
import type { ActionType, ProfileView, Workflow } from "@/lib/types"
import { WorkflowsPage } from "@/pages/workflows"
import { profiles } from "@/test/fixtures"
import { appContext } from "@/test/render"
import type { BuilderProps } from "@/workflow-builder/builder"

const api = {
  getProfile: vi.fn(),
  listActionTypes: vi.fn(),
  saveWorkflow: vi.fn(),
  duplicateWorkflow: vi.fn(),
}

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))
// The graph editor has its own tests; here it only says that it is shown.
vi.mock("@/workflow-builder/workflow-run", () => ({
  WorkflowRun: (props: Pick<BuilderProps, "initial" | "isNew">) => (
    <section aria-label="Graph editor">
      {props.isNew ? "Unsaved" : "Saved"} {props.initial.name}
    </section>
  ),
}))

const logType: ActionType = {
  id: "general.log",
  name: "Log message",
  description: "",
  category: "General",
  parameters: [],
  requiresProject: false,
}

const morning: Workflow = {
  id: "w-morning",
  name: "Morning",
  steps: [{ action: "a-log", onFailure: "stop" }],
}

function view(workflows: Workflow[]): ProfileView {
  return {
    profile: {
      id: "profile-1",
      name: "Work",
      description: "",
      actions: [{ id: "a-log", name: "Say hello", actionType: logType.id, parameters: {} }],
      workflows,
      projects: [],
      layouts: [],
    },
    activeProjectId: null,
    issues: [],
    actionUsages: {},
    workflowUsages: {},
  }
}

const context = () => appContext({ profiles: profiles([{ id: "profile-1", name: "Work" }], null) })

beforeAll(() => {
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.releasePointerCapture = () => undefined
  Element.prototype.scrollIntoView = () => undefined
})
beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset()
  api.listActionTypes.mockResolvedValue([logType])
})
afterEach(cleanup)

async function renderPage(workflows: Workflow[]) {
  api.getProfile.mockResolvedValue(view(workflows))
  const titlebar = new TitlebarStore()
  render(
    <AppContext value={context()}>
      <TitlebarContext value={titlebar}>
        <WorkflowsPage />
      </TitlebarContext>
    </AppContext>,
  )
  await screen.findByRole("combobox", { name: "Workflow" })
  return {
    user: userEvent.setup(),
    run: (id: string) => {
      act(() => {
        titlebar
          .getSnapshot()
          .actions?.find((a) => a.id === id)
          ?.onSelect?.()
      })
    },
  }
}

describe("Workflows page selection", () => {
  it("keeps the current workflow while a duplicate is loading, then opens the copy", async () => {
    const copy: Workflow = { ...morning, id: "w-copy", name: "Morning (copy)" }
    api.duplicateWorkflow.mockResolvedValue(copy)
    const { user } = await renderPage([morning])

    let deliver: (value: ProfileView) => void = () => undefined
    api.getProfile.mockReturnValue(
      new Promise<ProfileView>((resolve) => {
        deliver = resolve
      }),
    )
    await user.click(screen.getByRole("button", { name: "Duplicate" }))
    await waitFor(() => {
      expect(api.getProfile).toHaveBeenCalledTimes(2)
    })
    // No empty "new workflow" editor while the profile reloads.
    expect(screen.queryByRole("region", { name: "Graph editor" })).not.toBeInTheDocument()
    expect(screen.getByLabelText("Name")).toHaveValue("Morning")

    act(() => {
      deliver(view([morning, copy]))
    })
    await waitFor(() => {
      expect(screen.getByLabelText("Name")).toHaveValue("Morning (copy)")
    })
    expect(screen.queryByRole("region", { name: "Graph editor" })).not.toBeInTheDocument()
  })

  it("falls back to the first workflow when the requested one never arrives", async () => {
    api.duplicateWorkflow.mockResolvedValue({ ...morning, id: "w-lost" })
    const { user } = await renderPage([morning])
    await user.click(screen.getByRole("button", { name: "Duplicate" }))
    await waitFor(() => {
      expect(api.getProfile).toHaveBeenCalledTimes(2)
    })
    expect(screen.getByLabelText("Name")).toHaveValue("Morning")
    expect(screen.queryByRole("region", { name: "Graph editor" })).not.toBeInTheDocument()
  })

  it("creates a step workflow explicitly", async () => {
    const { user, run } = await renderPage([morning])
    run("workflows.new-steps")
    expect(screen.getByText(/A step workflow runs its actions in order/)).toBeInTheDocument()
    expect(screen.getByRole("combobox", { name: "Workflow" })).toHaveTextContent(
      "New step workflow",
    )
    expect(screen.queryByRole("region", { name: "Graph editor" })).not.toBeInTheDocument()

    const name = screen.getByLabelText("Name")
    await user.clear(name)
    await user.type(name, "Evening")
    await user.click(screen.getByRole("combobox", { name: "Action to add" }))
    await user.click(screen.getByRole("option", { name: "Say hello" }))
    await user.click(screen.getByRole("button", { name: "Add step" }))
    expect(
      within(screen.getByRole("list", { name: "Steps" })).getAllByRole("listitem"),
    ).toHaveLength(1)

    const saved: Workflow = { id: "w-evening", name: "Evening", steps: morning.steps }
    api.saveWorkflow.mockResolvedValue(saved)
    api.getProfile.mockResolvedValue(view([morning, saved]))
    await user.click(screen.getByRole("button", { name: "Save" }))
    const [, workflow] = api.saveWorkflow.mock.calls[0] as [string, Workflow]
    expect(workflow).toMatchObject({ name: "Evening", steps: [{ action: "a-log" }] })
    expect(workflow.graph).toBeUndefined()
    // The saved workflow opens in the step editor, without a detour.
    await waitFor(() => {
      expect(screen.getByRole("combobox", { name: "Workflow" })).toHaveTextContent("Evening")
    })
    expect(screen.queryByRole("region", { name: "Graph editor" })).not.toBeInTheDocument()
  })

  it("offers both kinds of new workflow in the switcher", async () => {
    const { user } = await renderPage([morning])
    await user.click(screen.getByRole("combobox", { name: "Workflow" }))
    expect(screen.getByRole("option", { name: "New workflow" })).toBeInTheDocument()
    await user.click(screen.getByRole("option", { name: "New step workflow" }))
    expect(screen.getByText(/A step workflow runs its actions in order/)).toBeInTheDocument()
    await user.click(screen.getByRole("combobox", { name: "Workflow" }))
    await user.click(screen.getByRole("option", { name: "New workflow" }))
    expect(screen.getByRole("region", { name: "Graph editor" })).toHaveTextContent(
      "Unsaved New workflow",
    )
  })
})
