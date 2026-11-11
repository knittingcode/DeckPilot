import { describe, expect, it } from "vitest"

import type { ActionInstance, ActionType, Workflow, WorkflowGraph } from "@/lib/types"

import { effectiveProjectScope, selectsProjects } from "./project-scope-model"
import { projectOutcomeText } from "./run-model"

function type(id: string, changesContext: boolean): ActionType {
  return {
    id,
    name: id,
    description: "",
    category: "",
    parameters: [],
    requiresProject: false,
    node: {
      version: 1,
      icon: null,
      outputs: [],
      timeoutMs: 30000,
      retrySafe: false,
      idempotent: false,
      changesContext,
    },
  }
}

const types = [type("projects.select", true), type("general.log", false)]
const actions: ActionInstance[] = [
  { id: "select", name: "Select", actionType: "projects.select", parameters: {} },
  { id: "log", name: "Log", actionType: "general.log", parameters: {} },
]
const policy = { onError: "stop" as const, retry: { attempts: 0, backoffMs: 0 } }
const at = { x: 0, y: 0 }

function graphWith(action: string): WorkflowGraph {
  return {
    version: 1,
    nodes: [
      { id: "start", type: "trigger", position: at },
      { id: "a", type: "action", action, policy, position: at },
    ],
    edges: [{ from: "start", port: "out", to: "a" }],
  }
}

describe("project scope", () => {
  it("keeps the stored choice", () => {
    const workflow: Workflow = { id: "w", name: "W", steps: [], projectScope: "execution" }
    expect(effectiveProjectScope(workflow, graphWith("select"), actions, types)).toBe("execution")
  })

  it("lets workflows saved before the setting keep changing the active project", () => {
    const old: Workflow = { id: "w", name: "W", steps: [] }
    expect(effectiveProjectScope(old, graphWith("select"), actions, types)).toBe("application")
    expect(effectiveProjectScope(old, graphWith("log"), actions, types)).toBe("execution")
    const steps: Workflow = { id: "w", name: "W", steps: [{ action: "select", onFailure: "stop" }] }
    expect(effectiveProjectScope(steps, graphWith("log"), actions, types)).toBe("application")
    expect(selectsProjects(graphWith("missing"), actions, types)).toBe(false)
  })

  it("describes what a run did to the active project", () => {
    const key = (id: string | null | undefined) => (id === "p1" ? "SHOP" : "?")
    expect(projectOutcomeText({ type: "applied", project: "p1" }, key)).toBe(
      "Switched DeckPilot to SHOP",
    )
    expect(projectOutcomeText({ type: "keptInRun", project: "p1" }, key)).toBe(
      "Worked on SHOP within the run",
    )
    expect(
      projectOutcomeText(
        { type: "notApplied", project: "p1", reason: "the active project changed during the run" },
        key,
      ),
    ).toBe("Did not switch to SHOP: the active project changed during the run")
    expect(projectOutcomeText({ type: "unchanged" }, key)).toBeNull()
    expect(projectOutcomeText(undefined, key)).toBeNull()
  })
})
