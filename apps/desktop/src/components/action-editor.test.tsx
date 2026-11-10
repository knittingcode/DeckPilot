// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { ActionEditor } from "./action-editor"
import type { ActionInstance, ActionType } from "@/lib/types"

const createAction = vi.fn()
const updateAction = vi.fn()

vi.mock("@/lib/api", () => ({
  api: {
    createAction: (...args: unknown[]) => createAction(...args) as unknown,
    updateAction: (...args: unknown[]) => updateAction(...args) as unknown,
    duplicateAction: vi.fn(),
    deleteAction: vi.fn(),
    runAction: vi.fn(),
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))

const logType: ActionType = {
  id: "deckpilot.core.log",
  name: "Log message",
  description: "Writes a message to the execution log.",
  category: "General",
  requiresProject: false,
  parameters: [
    {
      key: "message",
      label: "Message",
      kind: { type: "text", multiline: true, maxLength: 1000 },
      required: true,
    },
  ],
}

function renderEditor(action: ActionInstance | null, usages = 0) {
  const onSaved = vi.fn()
  render(
    <ActionEditor
      profileId="profile"
      types={[logType]}
      action={action}
      usages={Array.from({ length: usages }, (_, i) => ({
        type: "workflowStep" as const,
        workflow: "w",
        workflowName: "Start Work",
        position: i + 1,
      }))}
      canRun={false}
      onSaved={onSaved}
      onDeleted={vi.fn()}
    />,
  )
  return { onSaved }
}

describe("ActionEditor", () => {
  beforeEach(() => {
    createAction.mockReset()
    updateAction.mockReset()
  })
  afterEach(cleanup)

  it("shows validation errors instead of saving incomplete actions", async () => {
    const user = userEvent.setup()
    renderEditor(null)
    await user.click(screen.getByRole("button", { name: /create action/i }))
    expect(screen.getByText("Enter a name.")).toBeInTheDocument()
    expect(screen.getByText("Message is required.")).toBeInTheDocument()
    expect(createAction).not.toHaveBeenCalled()
  })

  it("creates an action from the schema-generated form", async () => {
    const user = userEvent.setup()
    const created = { id: "a1", name: "Greet", actionType: logType.id, parameters: {} }
    createAction.mockResolvedValue(created)
    const { onSaved } = renderEditor(null)

    await user.type(screen.getByLabelText("Name"), "Greet")
    await user.type(screen.getByLabelText("Key label"), "Hi {{project.key}")
    await user.type(screen.getByLabelText(/^Message/), "Hello from {{profile}")
    await user.click(screen.getByRole("button", { name: /create action/i }))

    expect(createAction).toHaveBeenCalledWith("profile", {
      name: "Greet",
      description: "",
      icon: null,
      label: "Hi {project.key}",
      actionType: logType.id,
      parameters: { message: { type: "text", value: "Hello from {profile}" } },
    })
    expect(onSaved).toHaveBeenCalledWith(created)
  })

  it("keeps the identifier when editing and blocks deleting used actions", async () => {
    const user = userEvent.setup()
    const action: ActionInstance = {
      id: "a1",
      name: "Greet",
      actionType: logType.id,
      parameters: { message: { type: "text", value: "hi" } },
    }
    updateAction.mockResolvedValue(action)
    renderEditor(action, 2)

    expect(screen.getByRole("button", { name: /delete/i })).toBeDisabled()
    expect(screen.getByText("Workflow “Start Work”, step 2")).toBeInTheDocument()

    const name = screen.getByLabelText("Name")
    await user.clear(name)
    await user.type(name, "Greet everyone")
    await user.click(screen.getByRole("button", { name: /^save$/i }))
    expect(updateAction).toHaveBeenCalledWith(
      "profile",
      expect.objectContaining({ id: "a1", name: "Greet everyone" }),
    )
  })
})
