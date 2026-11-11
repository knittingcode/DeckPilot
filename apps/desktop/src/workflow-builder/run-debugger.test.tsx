// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it } from "vitest"

import type { LiveExecution } from "@/lib/executions"
import type { NodeReport } from "@/lib/types"

import { type NodeInfo, RunDebugger } from "./run-debugger"

afterEach(cleanup)

const report = (node: string, change: Partial<NodeReport> = {}): NodeReport => ({
  node,
  status: "succeeded",
  attempts: 1,
  startedMs: 0,
  durationMs: 100,
  ...change,
})

function run(change: Partial<LiveExecution> = {}): LiveExecution {
  return {
    id: "e1",
    workflow: "w1",
    workflowName: "Deploy",
    profile: "p1",
    trigger: "app",
    state: "failed",
    startedAt: 0,
    finishedAt: 2_000,
    totalNodes: 4,
    completedNodes: 4,
    nodes: [
      report("start", { durationMs: 0 }),
      report("build", { startedMs: 10, durationMs: 1_200, output: { artifact: "/out/app.zip" } }),
      report("lint", { startedMs: 15, durationMs: 400 }),
      report("upload", {
        status: "failed",
        startedMs: 1_300,
        durationMs: 600,
        attempts: 3,
        error: {
          code: "action.failed",
          actionCode: "http-503",
          message: "Service unavailable",
          retryable: true,
        },
        output: { status: "503" },
      }),
    ],
    ...change,
  }
}

const INFO: Record<string, NodeInfo> = {
  start: { title: "Start" },
  build: {
    title: "Build",
    declared: [
      { key: "artifact", label: "Artifact" },
      { key: "token", label: "Signing token", sensitive: true },
    ],
  },
  lint: { title: "Lint", declared: [] },
  upload: { title: "Upload", declared: [{ key: "status", label: "HTTP status" }] },
}

function renderDebugger(value: LiveExecution, now = 2_000) {
  const user = userEvent.setup()
  render(<RunDebugger run={value} now={now} info={(id) => INFO[id] ?? { title: id }} />)
  return user
}

describe("RunDebugger", () => {
  it("shows metrics, slowest nodes and the parallel timeline", () => {
    renderDebugger(run())
    const metrics = screen.getByRole("region", { name: "Metrics" })
    expect(within(metrics).getByText("2 s")).toBeInTheDocument()
    expect(within(metrics).getByText(/up to 2 in parallel/)).toBeInTheDocument()
    const slowest = within(metrics).getByRole("list", { name: "Slowest nodes" })
    expect(
      within(slowest)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Build", "Upload", "Lint"])
    const timeline = screen.getByRole("list", { name: "Execution timeline" })
    expect(within(timeline).getAllByRole("button")).toHaveLength(4)
    expect(
      within(timeline).getByRole("button", { name: /^Build: succeeded, .*in parallel/ }),
    ).toBeInTheDocument()
    expect(within(timeline).getByRole("button", { name: /^Upload: failed/ })).toBeInTheDocument()
  })

  it("explains failures and opens the failed node first", () => {
    renderDebugger(run())
    const failures = screen.getByRole("region", { name: "Failures" })
    expect(within(failures).getByText("action.failed · http-503")).toBeInTheDocument()
    expect(within(failures).getByText(/^Transient/)).toBeInTheDocument()
    const details = screen.getByRole("region", { name: "Upload details" })
    expect(within(details).getByRole("group", { name: "Failure" })).toHaveTextContent(
      "Service unavailable",
    )
    // A history record knows only the final attempt.
    const retries = within(details).getByRole("group", { name: "Retries" })
    expect(retries).toHaveTextContent("3 attempts")
    expect(retries).toHaveTextContent("Run history keeps only the final attempt")
    expect(within(details).getByText("503")).toBeInTheDocument()
  })

  it("shows retry waits seen live", () => {
    renderDebugger(
      run({
        retries: {
          upload: [
            { attempt: 2, waitMs: 500 },
            { attempt: 3, waitMs: 1_000 },
          ],
        },
      }),
    )
    const retries = screen.getByRole("group", { name: "Retries" })
    expect(retries).toHaveTextContent("waited 1.5 s in total")
    expect(
      within(retries)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["Attempt 2 after waiting 500 ms", "Attempt 3 after waiting 1 s"])
    expect(retries).not.toHaveTextContent("keeps only the final attempt")
  })

  it("previews declared outputs and never sensitive ones", async () => {
    const user = renderDebugger(run())
    await user.click(screen.getByRole("button", { name: /^Build: / }))
    const details = screen.getByRole("region", { name: "Build details" })
    expect(within(details).getByText("/out/app.zip")).toBeInTheDocument()
    expect(within(details).getByText("not kept (sensitive)")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: /^Lint: / }))
    expect(screen.getByText("The action declares no outputs.")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: /^Start: / }))
    const start = screen.getByRole("region", { name: "Start details" })
    expect(within(start).queryByText("Outputs")).not.toBeInTheDocument()
  })

  it("has clear loading, waiting and empty states", () => {
    renderDebugger(run({ nodes: [] }))
    expect(screen.getByRole("status")).toHaveTextContent("Loading node reports")
    cleanup()
    renderDebugger(run({ nodes: [], state: "running", finishedAt: undefined }))
    expect(screen.getByRole("status")).toHaveTextContent("Waiting for the first node")
    cleanup()
    renderDebugger(run({ nodes: [], totalNodes: 0, completedNodes: 0 }))
    expect(screen.getByText("This run did not run any nodes.")).toBeInTheDocument()
  })

  it("follows a running node up to now", () => {
    renderDebugger(
      run({
        state: "running",
        finishedAt: undefined,
        nodes: [report("start", { durationMs: 0 })],
        runningNodes: ["build"],
        nodeStarts: { build: 100 },
      }),
      900,
    )
    expect(
      screen.getByRole("button", {
        name: /^Build: running, started 100 ms in, running for 800 ms/,
      }),
    ).toBeInTheDocument()
    expect(screen.getByText("Elapsed")).toBeInTheDocument()
  })
})
