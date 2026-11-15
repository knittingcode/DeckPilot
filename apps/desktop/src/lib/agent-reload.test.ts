import { describe, expect, it, vi } from "vitest"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), warning: vi.fn() } }))

import { toast } from "sonner"

import { appliedByAgent, reloadProblem, toastSaved } from "./agent-reload"

describe("appliedByAgent", () => {
  it("is true only for a confirmed reload", () => {
    expect(appliedByAgent({ state: "applied" })).toBe(true)
    expect(appliedByAgent({ state: "notRunning" })).toBe(false)
    expect(appliedByAgent({ state: "timedOut" })).toBe(false)
    expect(appliedByAgent(undefined)).toBe(false)
  })
})

describe("reloadProblem", () => {
  it("says nothing when the agent applied the change or reads it on start", () => {
    expect(reloadProblem({ state: "applied" })).toBeUndefined()
    expect(reloadProblem({ state: "notRunning" })).toBeUndefined()
    expect(reloadProblem(undefined)).toBeUndefined()
  })

  it("explains a reload the agent did not confirm", () => {
    expect(reloadProblem({ state: "timedOut" })).toMatch(/did not confirm/)
    expect(reloadProblem({ state: "failed", message: "the database is locked" })).toBe(
      "The agent did not apply the change: The database is locked.",
    )
  })
})

describe("toastSaved", () => {
  it("is a success when there is nothing to say", () => {
    toastSaved("Workflow saved", { state: "applied" })
    expect(toast.success).toHaveBeenCalledWith("Workflow saved")
    expect(toast.warning).not.toHaveBeenCalled()
  })

  it("is a warning with the reason otherwise", () => {
    toastSaved("Workflow saved", { state: "failed", message: "no" })
    expect(toast.warning).toHaveBeenCalledWith("Workflow saved", {
      description: "The agent did not apply the change: No.",
    })
    expect(toast.success).not.toHaveBeenCalled()
  })
})
