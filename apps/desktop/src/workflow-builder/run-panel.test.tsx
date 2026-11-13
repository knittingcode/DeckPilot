// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ComponentProps } from "react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import type { LiveExecution } from "@/lib/executions"
import { formatTimestamp } from "@/lib/format"
import { installDomStubs } from "@/test/dom"

import { RunHistory } from "./run-panel"

function run(change: Partial<LiveExecution> = {}): LiveExecution {
  return {
    id: "e1",
    workflow: "w1",
    workflowName: "Build",
    profile: "p1",
    trigger: "device",
    state: "succeeded",
    startedAt: 1_700_000_000_000,
    finishedAt: 1_700_000_002_000,
    totalNodes: 2,
    completedNodes: 2,
    nodes: [],
    runningNodes: [],
    attempts: {},
    ...change,
  }
}

function renderHistory(
  runs: LiveExecution[],
  change: Partial<ComponentProps<typeof RunHistory>> = {},
) {
  const props = {
    runs,
    shown: null,
    now: 1_700_000_010_000,
    onShow: vi.fn(),
    onCancel: vi.fn(),
    onClear: vi.fn(),
    projectKey: () => "no project",
    ...change,
  }
  render(<RunHistory {...props} />)
  return { props, user: userEvent.setup() }
}

beforeAll(installDomStubs)
afterEach(cleanup)

describe("RunHistory", () => {
  it("explains an empty history", () => {
    renderHistory([])
    expect(screen.getByText(/No runs yet/)).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Clear history" })).not.toBeInTheDocument()
  })

  it("says when earlier runs could not be loaded instead of claiming there are none", () => {
    renderHistory([], { problem: "agent unavailable" })
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Earlier runs could not be loaded: agent unavailable. They load again when the agent reconnects.",
    )
    expect(screen.queryByText(/No runs yet/)).not.toBeInTheDocument()
  })

  it("shows, cancels and clears runs", async () => {
    const live = run({ id: "e2", state: "running", completedNodes: 1, finishedAt: undefined })
    const failed = run({ id: "e1", state: "failed", message: "Step 2 exited with 1" })
    const { props, user } = renderHistory([live, failed], { shown: "e2" })
    const list = within(screen.getByRole("region", { name: "Runs" }))

    expect(list.getByRole("button", { name: /Running · 1 of 2 nodes/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    expect(list.getByRole("progressbar", { name: "Progress" })).toBeInTheDocument()
    const failedRun = list.getByRole("button", { name: /Failed/ })
    expect(failedRun).toHaveTextContent("Step 2 exited with 1")
    await user.click(failedRun)
    expect(props.onShow).toHaveBeenCalledWith("e1")

    const started = formatTimestamp(Math.floor(live.startedAt / 1000))
    await user.click(list.getByRole("button", { name: `Cancel the run started ${started}` }))
    expect(props.onCancel).toHaveBeenCalledWith("e2")

    await user.click(list.getByRole("button", { name: "Hide on canvas" }))
    expect(props.onShow).toHaveBeenCalledWith(null)

    await user.click(list.getByRole("button", { name: "Clear history" }))
    const dialog = await screen.findByRole("alertdialog")
    await user.click(within(dialog).getByRole("button", { name: "Clear" }))
    expect(props.onClear).toHaveBeenCalled()
  })
})
