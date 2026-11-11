import { describe, expect, it } from "vitest"

import type { LiveExecution } from "@/lib/executions"

import { nodeRuns, runSummary } from "./run-model"

function run(change: Partial<LiveExecution> = {}): LiveExecution {
  return {
    id: "e1",
    workflow: "w1",
    workflowName: "Build",
    profile: "p1",
    trigger: "device",
    state: "running",
    startedAt: 1_000,
    totalNodes: 4,
    completedNodes: 1,
    nodes: [{ node: "start", status: "succeeded", attempts: 0, startedMs: 0, durationMs: 0 }],
    runningNodes: ["build"],
    attempts: { build: 2 },
    ...change,
  }
}

describe("run model", () => {
  it("maps reports and running nodes onto the canvas", () => {
    const runs = nodeRuns(run())
    expect(runs?.start).toMatchObject({ state: "done", report: { status: "succeeded" } })
    expect(runs?.build).toEqual({ state: "running", attempt: 2 })
    expect(nodeRuns(undefined)).toBeUndefined()
  })

  it("summarizes progress and results", () => {
    expect(runSummary(run(), 5_000)).toBe("Running · 1 of 4 nodes")
    expect(runSummary(run({ state: "failed", finishedAt: 3_500 }), 9_000)).toBe("Failed · 2.5 s")
    expect(runSummary(run({ state: "interrupted", finishedAt: 1_250 }), 9_000)).toBe(
      "Interrupted · 250 ms",
    )
  })
})
