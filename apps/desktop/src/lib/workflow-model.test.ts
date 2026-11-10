import { describe, expect, it } from "vitest"

import {
  conditionAllowed,
  describeCondition,
  moveStep,
  newStep,
  removeStep,
  setOverride,
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
