import { describe, expect, it } from "vitest"

import { moduleInfo } from "@/lib/test-fixtures"
import type { ActionInstance, ActionType, GraphNode } from "@/lib/types"

import { flowEntries, moduleGroups, searchCatalog } from "./catalog"
import { describeNode, formatMs } from "./describe"

const repository: ActionType = {
  id: "projects.repository",
  name: "Project repository",
  description: "Resolves the repository",
  category: "projects",
  parameters: [],
  requiresProject: false,
  node: {
    version: 1,
    icon: "git-branch",
    outputs: [{ key: "path", label: "Path", kind: "path", always: true }],
    timeoutMs: 30000,
    retrySafe: true,
    idempotent: true,
    changesContext: false,
  },
}

const actions: ActionInstance[] = [
  { id: "a1", name: "Repo SHOP", actionType: "projects.repository", parameters: {} },
  { id: "a2", name: "Say hello", actionType: "general.log", parameters: {} },
]

describe("catalog", () => {
  it("groups the profile's actions under the module that provides their type", () => {
    const modules = [
      moduleInfo({ id: "projects", actionTypes: [repository] }),
      moduleInfo({ id: "disabled", enabled: false, actionTypes: [repository] }),
      moduleInfo({ id: "empty" }),
    ]
    const groups = moduleGroups(modules, actions)
    expect(groups.map((g) => g.id)).toEqual(["projects"])
    expect(groups[0]?.entries.map((e) => e.title)).toEqual(["Repo SHOP"])
    expect(groups[0]?.entries[0]?.icon).toBe("git-branch")
    expect(groups[0]?.entries[0]?.kind).toMatchObject({ type: "action", action: "a1" })
  })

  it("offers no new nodes of deprecated types but keeps their saved actions", () => {
    const next: ActionType = {
      ...repository,
      id: "projects.next",
      name: "Next project",
      deprecated: true,
    }
    const cycle: ActionType = { ...repository, id: "projects.cycle", name: "Cycle projects" }
    const saved: ActionInstance[] = [
      { id: "n1", name: "Old next", actionType: "projects.next", parameters: {} },
    ]
    const groups = moduleGroups([moduleInfo({ id: "projects", actionTypes: [next, cycle] })], saved)
    expect(groups[0]?.types.map((t) => t.id)).toEqual(["projects.cycle"])
    expect(groups[0]?.entries.map((e) => e.title)).toEqual(["Old next"])
    expect(groups[0]?.entries[0]?.description).toBe("Next project")
  })

  it("offers a loop and a sub-workflow only when another workflow exists", () => {
    expect(flowEntries(undefined).some((e) => e.kind.type === "loop")).toBe(false)
    expect(flowEntries(undefined).some((e) => e.kind.type === "call")).toBe(false)
    expect(flowEntries(undefined).some((e) => e.kind.type === "output")).toBe(true)
    expect(flowEntries("w2").find((e) => e.kind.type === "call")).toMatchObject({
      title: "Sub-workflow",
      kind: { workflow: "w2" },
    })
    expect(flowEntries("w2").find((e) => e.kind.type === "loop")?.kind).toMatchObject({
      workflow: "w2",
      times: 2,
    })
  })

  it("searches titles, descriptions and module names", () => {
    const groups = [
      { id: "flow", title: "Flow", entries: flowEntries(undefined), types: [] },
      ...moduleGroups([moduleInfo({ id: "projects", actionTypes: [repository] })], actions),
    ]
    expect(searchCatalog(groups, "wait").flatMap((g) => g.entries.map((e) => e.title))).toEqual([
      "Join",
      "Delay",
    ])
    expect(searchCatalog(groups, "projects")[0]?.entries).toHaveLength(1)
    expect(searchCatalog(groups, "zzz")).toEqual([])
    expect(searchCatalog(groups, " ")).toBe(groups)
  })
})

describe("node descriptions", () => {
  const lookup = { actions, types: [repository], workflows: [], projects: [] }
  const at = { x: 0, y: 0 }

  it("name actions and summarize flow nodes", () => {
    const action: GraphNode = {
      id: "repo",
      type: "action",
      action: "a1",
      policy: { onError: "stop", retry: { attempts: 0, backoffMs: 0 } },
      position: at,
    }
    expect(describeNode(action, lookup)).toEqual({
      title: "Repo SHOP",
      detail: "Project repository",
    })
    expect(describeNode({ ...action, action: "gone" }, lookup).title).toBe("Missing action")
    expect(describeNode({ id: "d", type: "delay", ms: 90000, position: at }, lookup).detail).toBe(
      "1 min 30 s",
    )
    expect(
      describeNode(
        {
          id: "c",
          type: "condition",
          predicate: { type: "succeeded", node: "repo" },
          position: at,
        },
        lookup,
      ).detail,
    ).toBe("repo succeeded")
    expect(formatMs(250)).toBe("250 ms")
    expect(formatMs(1500)).toBe("1.5 s")
  })
})
