import { describe, expect, it } from "vitest"

import type { WorkflowGraph } from "@/lib/types"

import {
  DEFAULT_POLICY,
  addNode,
  autoLayout,
  commit,
  connect,
  connectionProblem,
  copy,
  emptyGraph,
  history,
  isNodeId,
  nodeId,
  paste,
  redo,
  removeNodes,
  replace,
  settle,
  undo,
  upstream,
} from "./graph"

function chain(): WorkflowGraph {
  let graph = emptyGraph()
  graph = addNode(
    graph,
    { type: "action", action: "a1", policy: DEFAULT_POLICY },
    { x: 1, y: 0 },
    "Build",
  ).graph
  graph = addNode(
    graph,
    { type: "condition", predicate: { type: "succeeded", node: "build" } },
    { x: 2, y: 0 },
    "Check",
  ).graph
  graph = connect(graph, { from: "start", port: "out", to: "build" })
  return connect(graph, { from: "build", port: "out", to: "check" })
}

describe("node ids", () => {
  it("are readable slugs and unique", () => {
    expect(nodeId("Open Repository — SHOP", [])).toBe("open_repository_shop")
    expect(nodeId("Build", ["build"])).toBe("build_2")
    expect(nodeId("42 things", [])).toBe("node_42_things")
    expect(nodeId("", [])).toBe("node")
    expect(isNodeId(nodeId("Ü".repeat(80), []))).toBe(true)
  })
})

describe("connections", () => {
  it("follow the engine's rules", () => {
    const graph = chain()
    expect(connectionProblem(graph, { from: "check", port: "true", to: "start" })).toMatch(/start/)
    expect(connectionProblem(graph, { from: "check", port: "out", to: "build" })).toMatch(/output/)
    expect(connectionProblem(graph, { from: "check", port: "true", to: "build" })).toMatch(
      /several|cycle/,
    )
    expect(connectionProblem(graph, { from: "start", port: "out", to: "check" })).toMatch(
      /one node|several/,
    )
    expect(connectionProblem(graph, { from: "build", port: "out", to: "check" })).toMatch(/exists/)
    expect(connectionProblem(graph, { from: "build", port: "error", to: "build" })).toMatch(
      /itself/,
    )
  })

  it("reject cycles but allow joins and parallel fan-out", () => {
    let graph = chain()
    graph = addNode(graph, { type: "join" }, { x: 3, y: 0 }, "Join").graph
    graph = connect(graph, { from: "check", port: "true", to: "join" })
    expect(connectionProblem(graph, { from: "check", port: "false", to: "join" })).toBeNull()
    graph = addNode(graph, { type: "parallel" }, { x: 0, y: 0 }, "Fork").graph
    graph = addNode(graph, { type: "delay", ms: 10 }, { x: 0, y: 0 }, "A").graph
    graph = addNode(graph, { type: "delay", ms: 10 }, { x: 0, y: 0 }, "B").graph
    graph = connect(graph, { from: "fork", port: "out", to: "a" })
    expect(connectionProblem(graph, { from: "fork", port: "out", to: "b" })).toBeNull()
    graph = connect(graph, { from: "join", port: "out", to: "fork" })
    expect(connectionProblem(graph, { from: "a", port: "out", to: "join" })).toMatch(/cycle/)
  })
})

describe("editing", () => {
  it("never removes the start node", () => {
    const graph = removeNodes(chain(), ["start", "build"])
    expect(graph.nodes.map((n) => n.id)).toEqual(["start", "check"])
    expect(graph.edges).toEqual([])
  })

  it("lists the nodes that run before a node", () => {
    expect(upstream(chain(), "check")).toEqual(["start", "build"])
    expect(upstream(chain(), "start")).toEqual([])
  })

  it("pastes copies with new ids and their inner connections", () => {
    const graph = chain()
    const clipboard = copy(graph, ["start", "build", "check"])
    expect(clipboard?.nodes.map((n) => n.id)).toEqual(["build", "check"])
    const pasted = paste(graph, clipboard ?? { nodes: [], edges: [] })
    expect(pasted.ids).toEqual(["build_2", "check_2"])
    expect(pasted.graph.edges.at(-1)).toEqual({ from: "build_2", port: "out", to: "check_2" })
    expect(pasted.graph.nodes.find((n) => n.id === "build_2")?.position).toEqual({ x: 41, y: 40 })
  })

  it("lays out nodes by depth", () => {
    const graph = autoLayout(chain())
    const x = Object.fromEntries(graph.nodes.map((n) => [n.id, n.position.x]))
    expect(x).toEqual({ start: 0, build: 280, check: 560 })
  })
})

describe("history", () => {
  it("undoes and redoes committed changes", () => {
    let state = history(emptyGraph())
    state = commit(state, chain())
    expect(undo(state).present.nodes).toHaveLength(1)
    expect(redo(undo(state)).present.nodes).toHaveLength(3)
    // Unchanged graphs add no step.
    expect(commit(state, structuredClone(state.present))).toBe(state)
  })

  it("turns a drag into one step", () => {
    const before = chain()
    let state = history(before)
    const moved = (x: number) => ({
      ...before,
      nodes: before.nodes.map((n) => (n.id === "build" ? { ...n, position: { x, y: 0 } } : n)),
    })
    state = replace(state, moved(10))
    state = replace(state, moved(20))
    state = settle(state, before)
    expect(state.past).toHaveLength(1)
    expect(undo(state).present).toEqual(before)
  })
})
