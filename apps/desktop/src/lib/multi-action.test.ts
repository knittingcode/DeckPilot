import { describe, expect, it } from "vitest"

import {
  DEFAULT_DELAY_MS,
  MAX_DELAY_MS,
  MIN_DELAY_MS,
  delayAction,
  delayMilliseconds,
  delayStep,
  freeWorkflowName,
  isStepWorkflow,
  newMultiAction,
  withDelay,
} from "./multi-action"
import type { Workflow } from "./types"

const delay = {
  id: "d",
  actionType: "general.delay",
  parameters: { milliseconds: { type: "integer" as const, value: 250 } },
}
const log = { id: "l", actionType: "general.log", parameters: {} }

describe("Multi Action helpers", () => {
  it("names new Multi Actions apart from the profile's workflows", () => {
    expect(freeWorkflowName([], "Multi Action")).toBe("Multi Action")
    expect(freeWorkflowName([{ name: "multi action" }], "Multi Action")).toBe("Multi Action 2")
    const fresh = newMultiAction([{ name: "Multi Action" }, { name: "Multi Action 2" }])
    expect(fresh).toMatchObject({ name: "Multi Action 3", steps: [], projectScope: "execution" })
    expect(isStepWorkflow(fresh)).toBe(true)
  })

  it("edits step workflows only, never graphs", () => {
    const graph = { id: "g", name: "G", steps: [], graph: {} } as unknown as Workflow
    expect(isStepWorkflow(graph)).toBe(false)
    expect(isStepWorkflow(undefined)).toBe(false)
  })

  it("stores a delay as an ordinary step with its own duration", () => {
    const step = delayStep("d")
    expect(step).toEqual({
      action: "d",
      onFailure: "stop",
      condition: { type: "always" },
      overrides: { milliseconds: { type: "integer", value: DEFAULT_DELAY_MS } },
    })
    expect(delayAction(step, [log, delay])).toBe(delay)
    expect(delayAction({ ...step, action: "l" }, [log, delay])).toBeUndefined()
  })

  it("reads the step's duration, then the action's, then the default", () => {
    expect(delayMilliseconds(delayStep("d", 1200), delay)).toBe(1200)
    expect(delayMilliseconds({ ...delayStep("d"), overrides: {} }, delay)).toBe(250)
    expect(delayMilliseconds({ ...delayStep("d"), overrides: {} }, undefined)).toBe(
      DEFAULT_DELAY_MS,
    )
  })

  it("keeps durations in the range the agent accepts", () => {
    const step = delayStep("d")
    expect(delayMilliseconds(withDelay(step, 0), delay)).toBe(MIN_DELAY_MS)
    expect(delayMilliseconds(withDelay(step, 10 ** 9), delay)).toBe(MAX_DELAY_MS)
    expect(delayMilliseconds(withDelay(step, 333.6), delay)).toBe(334)
  })
})
