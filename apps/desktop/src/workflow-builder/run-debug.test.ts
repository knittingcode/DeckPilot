import { describe, expect, it } from "vitest"

import type { LiveExecution } from "@/lib/executions"
import type { NodeReport } from "@/lib/types"

import {
  PREVIEW_LENGTH,
  barGeometry,
  buildTimeline,
  debugState,
  describeFailure,
  outputPreviews,
  retryHistory,
  runFailures,
  runMetrics,
} from "./run-debug"

function report(
  node: string,
  startedMs: number,
  durationMs: number,
  change: Partial<NodeReport> = {},
): NodeReport {
  return { node, status: "succeeded", attempts: 1, startedMs, durationMs, ...change }
}

function run(change: Partial<LiveExecution> = {}): LiveExecution {
  return {
    id: "e1",
    workflow: "w1",
    workflowName: "Build",
    profile: "p1",
    trigger: "app",
    state: "succeeded",
    startedAt: 10_000,
    finishedAt: 11_000,
    totalNodes: 5,
    completedNodes: 5,
    nodes: [
      report("start", 0, 0),
      report("fork", 0, 0),
      report("a", 10, 600),
      report("b", 20, 300, { attempts: 3 }),
      report("join", 620, 0),
    ],
    ...change,
  }
}

const title = (node: string) => node.toUpperCase()

describe("timeline", () => {
  it("places parallel branches on separate lanes", () => {
    const timeline = buildTimeline(run(), 99_999, title)
    expect(timeline.spanMs).toBe(1000)
    expect(timeline.rows.map((r) => r.node)).toEqual(["fork", "start", "a", "b", "join"])
    const a = timeline.rows.find((r) => r.node === "a")
    const b = timeline.rows.find((r) => r.node === "b")
    expect(a?.lane).not.toBe(b?.lane)
    expect(a?.parallel).toBe(true)
    expect(b?.parallel).toBe(true)
    expect(timeline.rows.find((r) => r.node === "join")?.parallel).toBe(false)
    expect(a?.title).toBe("A")
    expect(timeline.lanes).toBe(2)
  })

  it("keeps sequential nodes on one lane", () => {
    const timeline = buildTimeline(
      run({ nodes: [report("x", 0, 100), report("y", 100, 100)] }),
      0,
      title,
    )
    expect(timeline.rows.map((r) => r.lane)).toEqual([0, 0])
    expect(timeline.rows.every((r) => !r.parallel)).toBe(true)
  })

  it("draws running nodes up to now, estimating unknown starts", () => {
    const live = run({
      state: "running",
      finishedAt: undefined,
      nodes: [report("start", 0, 50)],
      runningNodes: ["seen", "unseen"],
      nodeStarts: { seen: 100 },
      attempts: { seen: 2 },
    })
    const timeline = buildTimeline(live, 10_400, title)
    const seen = timeline.rows.find((r) => r.node === "seen")
    const unseen = timeline.rows.find((r) => r.node === "unseen")
    expect(seen).toMatchObject({ status: "running", startMs: 100, durationMs: 300, attempts: 2 })
    expect(seen?.estimated).toBe(false)
    expect(unseen).toMatchObject({ startMs: 50, durationMs: 350, estimated: true })
    expect(timeline.spanMs).toBe(400)
  })

  it("keeps bars inside the chart with a visible minimum", () => {
    const row = buildTimeline(run(), 0, title).rows[0]
    if (!row) throw new Error("no row")
    expect(barGeometry({ ...row, startMs: 0, durationMs: 0 }, 1000)).toEqual({
      left: 0,
      width: 0.75,
    })
    expect(barGeometry({ ...row, startMs: 900, durationMs: 500 }, 1000)).toEqual({
      left: 90,
      width: 10,
    })
  })
})

describe("metrics", () => {
  it("reports total, node time, slowest nodes and parallelism", () => {
    const r = run()
    const metrics = runMetrics(r, buildTimeline(r, 0, title), 0)
    expect(metrics.totalMs).toBe(1000)
    expect(metrics.nodeTimeMs).toBe(900)
    expect(metrics.slowest.map((s) => s.node)).toEqual(["a", "b"])
    expect(metrics.counts.succeeded).toBe(5)
    expect(metrics.retriedNodes).toBe(1)
    expect(metrics.maxParallel).toBe(2)
  })

  it("counts nodes retried live", () => {
    const r = run({ nodes: [report("x", 0, 10)], retries: { x: [{ attempt: 2, waitMs: 100 }] } })
    expect(runMetrics(r, buildTimeline(r, 0, title), 0).retriedNodes).toBe(1)
  })
})

describe("retries", () => {
  it("uses live waits when they were seen", () => {
    const r = run({
      retries: {
        b: [
          { attempt: 2, waitMs: 200 },
          { attempt: 3, waitMs: 400 },
        ],
      },
    })
    expect(retryHistory(r, "b")).toEqual({
      attempts: 3,
      waits: [
        { attempt: 2, waitMs: 200 },
        { attempt: 3, waitMs: 400 },
      ],
      totalWaitMs: 600,
      live: true,
    })
  })

  it("knows only the final attempt from a history record", () => {
    expect(retryHistory(run(), "b")).toMatchObject({ attempts: 3, waits: [], live: false })
    expect(retryHistory(run({ attempts: { z: 2 } }), "z").attempts).toBe(2)
  })
})

describe("failures", () => {
  it("lists failures with code, action code and a retry hint, without cancellations", () => {
    const r = run({
      state: "failed",
      nodes: [
        report("late", 50, 10, {
          status: "failed",
          error: { code: "timeout", message: "Timed out after 5 s", retryable: true },
        }),
        report("early", 5, 10, {
          status: "failed",
          error: {
            code: "action.failed",
            actionCode: "exit-nonzero",
            message: "Exited with 2",
            retryable: false,
          },
        }),
        report("stopped", 0, 1, {
          status: "cancelled",
          error: { code: "cancelled", message: "Cancelled", retryable: false },
        }),
      ],
    })
    const failures = runFailures(r, title)
    expect(failures.map((f) => f.node)).toEqual(["early", "late"])
    expect(failures[0]).toMatchObject({
      title: "EARLY",
      error: { actionCode: "exit-nonzero" },
      explanation: "The action reported a failure.",
    })
    expect(failures[0]?.hint).toMatch(/^Not transient/)
    expect(failures[1]?.hint).toMatch(/^Transient/)
    expect(describeFailure({ code: "new", message: "", retryable: false }).explanation).toBe(
      "The node failed.",
    )
  })
})

describe("output previews", () => {
  it("shows declared values only and never sensitive ones", () => {
    const long = "x".repeat(PREVIEW_LENGTH + 5)
    const node = report("a", 0, 1, { output: { path: "/tmp/a", log: long, extra: "hidden" } })
    const previews = outputPreviews(node, [
      { key: "path", label: "Path" },
      { key: "log", label: "Log" },
      { key: "token", label: "Token", sensitive: true },
      { key: "url", label: "URL" },
    ])
    expect(previews).toEqual([
      { key: "path", label: "Path", state: "value", value: "/tmp/a", truncated: false },
      {
        key: "log",
        label: "Log",
        state: "value",
        value: `${"x".repeat(PREVIEW_LENGTH)}…`,
        truncated: true,
      },
      { key: "token", label: "Token", state: "sensitive" },
      { key: "url", label: "URL", state: "missing" },
    ])
    expect(previews.some((p) => p.key === "extra")).toBe(false)
  })

  it("marks sensitive outputs even if a value were present", () => {
    const node = report("a", 0, 1, { output: { token: "secret" } })
    expect(outputPreviews(node, [{ key: "token", label: "Token", sensitive: true }])).toEqual([
      { key: "token", label: "Token", state: "sensitive" },
    ])
  })
})

describe("debug state", () => {
  it("distinguishes loading, waiting, empty and ready", () => {
    expect(debugState(run())).toEqual({ type: "ready" })
    expect(debugState(run({ nodes: [] }))).toEqual({ type: "loading" })
    expect(debugState(run({ nodes: [], state: "running" }))).toEqual({ type: "waiting" })
    expect(debugState(run({ nodes: [], state: "running", runningNodes: ["a"] }))).toEqual({
      type: "ready",
    })
    expect(debugState(run({ nodes: [], totalNodes: 0, completedNodes: 0 }))).toEqual({
      type: "empty",
    })
  })
})
