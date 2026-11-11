import { describe, expect, it } from "vitest"

import type { LiveExecution } from "@/lib/executions"
import type { ActionType, WorkflowGraph } from "@/lib/types"

import { nodeRuns, runNodeInfo, runSummary } from "./run-model"

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

  it("names nodes and lists the outputs their actions declare", () => {
    const graph: WorkflowGraph = {
      version: 1,
      nodes: [
        { id: "t", type: "trigger", position: { x: 0, y: 0 } },
        {
          id: "a",
          type: "action",
          action: "i1",
          policy: { onError: "stop", retry: { attempts: 1, backoffMs: 0 } },
          position: { x: 0, y: 0 },
        },
      ],
      edges: [],
    }
    const type = {
      id: "http.get",
      name: "HTTP GET",
      node: {
        outputs: [
          { key: "body", label: "Body", kind: "text", always: true },
          { key: "token", label: "Token", kind: "text", always: false, sensitive: true },
        ],
      },
    } as unknown as ActionType
    const info = runNodeInfo(graph, {
      actions: [{ id: "i1", name: "Fetch", actionType: "http.get", parameters: {} }],
      types: [type],
      workflows: [],
      projects: [],
    })
    expect(info("a")).toEqual({
      title: "Fetch",
      declared: [
        { key: "body", label: "Body", sensitive: undefined },
        { key: "token", label: "Token", sensitive: true },
      ],
    })
    expect(info("t").declared).toBeUndefined()
    expect(info("gone")).toEqual({ title: "gone" })
  })

  it("summarizes progress and results", () => {
    expect(runSummary(run(), 5_000)).toBe("Running · 1 of 4 nodes")
    expect(runSummary(run({ state: "failed", finishedAt: 3_500 }), 9_000)).toBe("Failed · 2.5 s")
    expect(runSummary(run({ state: "interrupted", finishedAt: 1_250 }), 9_000)).toBe(
      "Interrupted · 250 ms",
    )
  })
})
