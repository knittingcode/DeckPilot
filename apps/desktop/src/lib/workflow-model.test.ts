import { describe, expect, it } from "vitest"

import {
  conditionAllowed,
  describeCondition,
  moveStep,
  newStep,
  removeStep,
  setOverride,
  shownWorkflow,
  validateWorkflow,
} from "./workflow-model"
import type { Workflow } from "./types"

const workflow: Workflow = {
  id: "w",
  name: "Start Work",
  steps: [newStep("a"), newStep("b"), newStep("c")],
}

describe("workflow editing", () => {
  it("reorders steps", () => {
    expect(moveStep(workflow, 0, 2).steps.map((s) => s.action)).toEqual(["b", "c", "a"])
    expect(moveStep(workflow, 2, 0).steps.map((s) => s.action)).toEqual(["c", "a", "b"])
    expect(moveStep(workflow, 5, 0)).toBe(workflow)
    expect(removeStep(workflow, 1).steps.map((s) => s.action)).toEqual(["a", "c"])
  })

  it("manages per-step overrides", () => {
    const overrides = setOverride({}, "message", { type: "text", value: "hi" })
    expect(overrides).toEqual({ message: { type: "text", value: "hi" } })
    expect(setOverride(overrides, "message", undefined)).toEqual({})
  })

  it("validates names, actions and conditions", () => {
    const first = { ...newStep("a"), condition: { type: "previousFailed" } as const }
    const invalid: Workflow = { id: "w", name: " ", steps: [first, newStep("zzz")] }
    expect(validateWorkflow(invalid, new Set(["a"]))).toHaveLength(3)
    expect(validateWorkflow(workflow, new Set(["a", "b", "c"]))).toEqual([])
    expect(conditionAllowed({ type: "previousSucceeded" }, 1)).toBe(true)
  })

  it("describes conditions", () => {
    const projects = [{ id: "p", key: "SHOP", name: "Shop", resources: {} }]
    expect(describeCondition({ type: "activeProject", project: "p" }, projects)).toBe(
      "while SHOP is active",
    )
    expect(describeCondition(undefined, projects)).toBe("always")
  })
})

describe("shown workflow", () => {
  const a: Workflow = { id: "a", name: "A", steps: [] }
  const b: Workflow = { id: "b", name: "B", steps: [] }
  const before = { revision: 1 }
  const after = { revision: 2 }
  const drafts = ["new", "new-steps"]

  it("shows loaded workflows and drafts directly", () => {
    expect(shownWorkflow({ id: "b" }, [a, b], before, drafts)).toBe("b")
    expect(shownWorkflow({ id: "new-steps" }, [a], before, drafts)).toBe("new-steps")
  })

  it("keeps showing the previous workflow until the reload brings the new one", () => {
    const selection = { id: "copy", waiting: { data: before, shown: "a" } }
    expect(shownWorkflow(selection, [a, b], before, drafts)).toBe("a")
    expect(shownWorkflow(selection, [a, b, { ...a, id: "copy" }], after, drafts)).toBe("copy")
  })

  it("falls back once a reload arrived without the workflow", () => {
    const selection = { id: "gone", waiting: { data: before, shown: "a" } }
    expect(shownWorkflow(selection, [b], after, drafts)).toBe("b")
    expect(shownWorkflow(selection, [], after, drafts)).toBe("new")
  })
})
