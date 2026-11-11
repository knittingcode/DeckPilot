import { describe, expect, it } from "vitest"

import { ExecutionStore, executionProgress } from "@/lib/executions"
import type { ExecutionSummary, NodeReport } from "@/lib/types"

function summary(change: Partial<ExecutionSummary> = {}): ExecutionSummary {
  return {
    id: "e1",
    workflow: "w1",
    workflowName: "Build",
    profile: "p1",
    trigger: "app",
    state: "running",
    startedAt: 1000,
    totalNodes: 3,
    completedNodes: 0,
    ...change,
  }
}

function node(id: string, change: Partial<NodeReport> = {}): NodeReport {
  return { node: id, status: "succeeded", attempts: 1, startedMs: 0, durationMs: 4, ...change }
}

describe("ExecutionStore", () => {
  it("follows a run from start to finish", () => {
    const store = new ExecutionStore()
    store.apply({ type: "executionStarted", execution: summary() })
    store.apply({
      type: "executionProgress",
      execution: "e1",
      event: { type: "nodeStarted", node: "build" },
      completed: 0,
      total: 3,
    })
    expect(store.get("e1")?.runningNodes).toEqual(["build"])
    store.apply({
      type: "executionProgress",
      execution: "e1",
      event: { type: "nodeFinished", report: node("build") },
      completed: 1,
      total: 3,
    })
    const run = store.get("e1")
    expect(run?.runningNodes).toEqual([])
    expect(run?.nodes.map((n) => n.node)).toEqual(["build"])
    expect(run && executionProgress(run)).toBe(33)

    store.apply({
      type: "executionFinished",
      execution: summary({ state: "failed", completedNodes: 3, message: "boom" }),
    })
    expect(store.get("e1")?.state).toBe("failed")
    // Node details survive the summary.
    expect(store.get("e1")?.nodes).toHaveLength(1)
  })

  it("keeps progress that arrived before the run's start", () => {
    const store = new ExecutionStore()
    store.load({ ...summary({ completedNodes: 2 }), nodes: [node("a"), node("b")] })
    store.apply({ type: "executionStarted", execution: summary() })
    expect(store.get("e1")?.completedNodes).toBe(2)
    expect(store.get("e1")?.nodes).toHaveLength(2)
  })

  it("asks for runs it has not seen start", () => {
    const store = new ExecutionStore()
    expect(
      store.apply({
        type: "executionProgress",
        execution: "unknown",
        event: { type: "nodeStarted", node: "a" },
        completed: 0,
        total: 1,
      }),
    ).toBe(false)
  })

  it("replaces its runs with the agent's list on reconnect", () => {
    const store = new ExecutionStore()
    store.load({ ...summary(), nodes: [node("a")] })
    store.load({ ...summary({ id: "gone" }), nodes: [] })
    store.reset([summary({ completedNodes: 1 }), summary({ id: "e2", startedAt: 2000 })])
    expect(store.get("gone")).toBeUndefined()
    expect(store.get("e1")?.nodes).toHaveLength(1)
    expect(store.get("e1")?.completedNodes).toBe(1)
    const ordered = ExecutionStore.ofProfile(store.getSnapshot(), "p1").map((r) => r.id)
    expect(ordered).toEqual(["e2", "e1"])
    expect(ExecutionStore.ofProfile(store.getSnapshot(), "other")).toEqual([])
  })
})
