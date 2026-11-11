/**
 * Pure editing operations on a workflow graph. The rules mirror the
 * engine's validation (crates/deckpilot-core/src/graph/validate.rs) so the
 * editor refuses connections the engine would reject; the engine's own
 * validation remains the authority and is shown on the nodes.
 */
import type {
  ErrorPolicy,
  GraphEdge,
  GraphNode,
  GraphNodeKind,
  GraphPort,
  WorkflowGraph,
} from "@/lib/types"

export const GRAPH_VERSION = 1
export const MAX_NODES = 200
export const MAX_LOOP_ITERATIONS = 100
export const MAX_RETRIES = 5
export const MAX_DELAY_MS = 10 * 60 * 1000
/** Default deadline of a sub-workflow (`DEFAULT_CALL_TIMEOUT_MS`). */
export const DEFAULT_CALL_TIMEOUT_MS = 10 * 60 * 1000
/** Inputs and outputs a workflow can declare, each. */
export const MAX_WORKFLOW_VALUES = 32
const MAX_ID_LENGTH = 48

export type NodeType = GraphNodeKind["type"]

/** Output ports of a node kind, in display order. */
export function outputPorts(type: NodeType): GraphPort[] {
  switch (type) {
    case "action":
    case "call":
      return ["out", "error"]
    case "condition":
      return ["true", "false"]
    case "stop":
      return []
    default:
      return ["out"]
  }
}

/** Whether a node kind has an input. */
export function hasInput(type: NodeType) {
  return type !== "trigger"
}

export const PORT_LABEL: Record<GraphPort, string> = {
  out: "Next",
  error: "On error",
  true: "Yes",
  false: "No",
}

export const DEFAULT_POLICY: ErrorPolicy = {
  onError: "stop",
  retry: { attempts: 0, backoffMs: 1000 },
}

export function emptyGraph(): WorkflowGraph {
  return {
    version: GRAPH_VERSION,
    nodes: [{ id: "start", type: "trigger", position: { x: 0, y: 0 } }],
    edges: [],
  }
}

/** Whether `id` is a valid node id: lowercase letters, digits and `_`. */
export function isNodeId(id: string) {
  return id.length <= MAX_ID_LENGTH && /^[a-z][a-z0-9_]*$/.test(id)
}

/** A readable, unused node id derived from `base` (e.g. an action name). */
export function nodeId(base: string, taken: Iterable<string>) {
  const used = new Set(taken)
  let slug = base
    .normalize("NFKD")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
  if (!/^[a-z]/.test(slug)) slug = `node_${slug}`.replace(/_+$/, "")
  slug = slug.slice(0, MAX_ID_LENGTH - 4).replace(/_+$/, "") || "node"
  if (!used.has(slug)) return slug
  for (let n = 2; ; n++) {
    const candidate = `${slug}_${n}`
    if (!used.has(candidate)) return candidate
  }
}

export function addNode(
  graph: WorkflowGraph,
  kind: GraphNodeKind,
  position: { x: number; y: number },
  name: string,
): { graph: WorkflowGraph; id: string } {
  const id = nodeId(
    name,
    graph.nodes.map((n) => n.id),
  )
  const node = { ...kind, id, position } as GraphNode
  return { graph: { ...graph, nodes: [...graph.nodes, node] }, id }
}

export function updateNode(
  graph: WorkflowGraph,
  id: string,
  change: (node: GraphNode) => GraphNode,
): WorkflowGraph {
  return { ...graph, nodes: graph.nodes.map((n) => (n.id === id ? change(n) : n)) }
}

/** Removes nodes (never the start node) and their connections. */
export function removeNodes(graph: WorkflowGraph, ids: Iterable<string>): WorkflowGraph {
  const remove = new Set(ids)
  for (const node of graph.nodes) if (node.type === "trigger") remove.delete(node.id)
  return {
    ...graph,
    nodes: graph.nodes.filter((n) => !remove.has(n.id)),
    edges: graph.edges.filter((e) => !remove.has(e.from) && !remove.has(e.to)),
  }
}

export function sameEdge(a: GraphEdge, b: GraphEdge) {
  return a.from === b.from && a.port === b.port && a.to === b.to
}

export function removeEdges(graph: WorkflowGraph, edges: GraphEdge[]): WorkflowGraph {
  return { ...graph, edges: graph.edges.filter((e) => !edges.some((r) => sameEdge(e, r))) }
}

/** Whether `to` can reach `from` (a connection from → to would close a cycle). */
function reaches(graph: WorkflowGraph, start: string, target: string) {
  const seen = new Set<string>()
  const stack = [start]
  while (stack.length > 0) {
    const id = stack.pop() as string
    if (id === target) return true
    if (seen.has(id)) continue
    seen.add(id)
    for (const edge of graph.edges) if (edge.from === id) stack.push(edge.to)
  }
  return false
}

/** Why `edge` cannot be added, or `null` if it can. */
export function connectionProblem(graph: WorkflowGraph, edge: GraphEdge): string | null {
  const from = graph.nodes.find((n) => n.id === edge.from)
  const to = graph.nodes.find((n) => n.id === edge.to)
  if (!from || !to) return "both nodes must exist"
  if (from.id === to.id) return "a node cannot connect to itself"
  if (!outputPorts(from.type).includes(edge.port)) return "this output does not exist"
  if (!hasInput(to.type)) return "nothing can connect into the start node"
  if (graph.edges.some((e) => sameEdge(e, edge))) return "the connection exists already"
  if (
    from.type !== "parallel" &&
    graph.edges.some((e) => e.from === from.id && e.port === edge.port)
  ) {
    return "an output connects to one node; branch with a Parallel or Condition node"
  }
  if (to.type !== "join" && graph.edges.some((e) => e.to === to.id)) {
    return "only a Join node can have several incoming connections"
  }
  if (reaches(graph, to.id, from.id)) return "this would create a cycle; use a Loop node to repeat"
  return null
}

export function connect(graph: WorkflowGraph, edge: GraphEdge): WorkflowGraph {
  if (connectionProblem(graph, edge)) return graph
  return { ...graph, edges: [...graph.edges, edge] }
}

/** Nodes that always run before `id`: everything that reaches it. */
export function upstream(graph: WorkflowGraph, id: string): string[] {
  const result = new Set<string>()
  const stack = [id]
  while (stack.length > 0) {
    const current = stack.pop() as string
    for (const edge of graph.edges) {
      if (edge.to === current && !result.has(edge.from)) {
        result.add(edge.from)
        stack.push(edge.from)
      }
    }
  }
  return graph.nodes.map((n) => n.id).filter((n) => result.has(n))
}

/** A copied selection: nodes and the connections between them. */
export interface Clipboard {
  nodes: GraphNode[]
  edges: GraphEdge[]
}

export function copy(graph: WorkflowGraph, ids: Iterable<string>): Clipboard | null {
  const selected = new Set(ids)
  const nodes = graph.nodes.filter((n) => selected.has(n.id) && n.type !== "trigger")
  if (nodes.length === 0) return null
  const kept = new Set(nodes.map((n) => n.id))
  return {
    nodes: structuredClone(nodes),
    edges: graph.edges.filter((e) => kept.has(e.from) && kept.has(e.to)),
  }
}

/**
 * Pastes a copy with fresh ids, offset so it does not cover the original.
 * References to pasted nodes inside the pasted nodes are kept as they were
 * (they point at the originals); the engine's validation reports them if
 * they no longer run before.
 */
export function paste(
  graph: WorkflowGraph,
  clipboard: Clipboard,
  offset = 40,
): { graph: WorkflowGraph; ids: string[] } {
  const taken = new Set(graph.nodes.map((n) => n.id))
  const renamed = new Map<string, string>()
  const nodes = clipboard.nodes.slice(0, MAX_NODES - graph.nodes.length).map((node) => {
    const id = nodeId(node.id, taken)
    taken.add(id)
    renamed.set(node.id, id)
    return {
      ...structuredClone(node),
      id,
      position: { x: node.position.x + offset, y: node.position.y + offset },
    }
  })
  const edges = clipboard.edges
    .filter((e) => renamed.has(e.from) && renamed.has(e.to))
    .map((e) => ({ ...e, from: renamed.get(e.from) as string, to: renamed.get(e.to) as string }))
  return {
    graph: { ...graph, nodes: [...graph.nodes, ...nodes], edges: [...graph.edges, ...edges] },
    ids: nodes.map((n) => n.id),
  }
}

const COLUMN = 280
const ROW = 140
/** A node's footprint on the canvas, for overlap checks. */
const NODE_WIDTH = 224
const NODE_HEIGHT = 96

/** Replaces `old` with `edge`, if the engine's rules allow the new one. */
export function reconnect(
  graph: WorkflowGraph,
  old: GraphEdge,
  edge: GraphEdge,
): { graph: WorkflowGraph; problem: string | null } {
  const without = removeEdges(graph, [old])
  const problem = connectionProblem(without, edge)
  if (problem) return { graph, problem }
  return { graph: { ...without, edges: [...without.edges, edge] }, problem: null }
}

function overlaps(a: { x: number; y: number }, b: { x: number; y: number }) {
  return Math.abs(a.x - b.x) < NODE_WIDTH && Math.abs(a.y - b.y) < NODE_HEIGHT
}

/**
 * The nearest position at or below `wanted` where a new node covers no
 * existing one, so adding nodes never moves or hides what the user placed.
 */
export function freePosition(
  graph: WorkflowGraph,
  wanted: { x: number; y: number },
): { x: number; y: number } {
  let position = wanted
  for (let step = 0; step < MAX_NODES; step++) {
    if (!graph.nodes.some((node) => overlaps(node.position, position))) return position
    position = { x: wanted.x, y: wanted.y + (step + 1) * (NODE_HEIGHT + 24) }
  }
  return position
}

/** Where on its source a connection leaves, so Yes runs above No. */
const PORT_OFFSET: Record<GraphPort, number> = { out: 0, true: -0.3, false: 0.3, error: 0.4 }

/** Columns by longest distance from the start; unreachable nodes last. */
function columns(graph: WorkflowGraph): Map<string, number> {
  const depth = new Map<string, number>()
  const indegree = new Map(graph.nodes.map((n) => [n.id, 0]))
  for (const edge of graph.edges) indegree.set(edge.to, (indegree.get(edge.to) ?? 0) + 1)
  const queue = graph.nodes.filter((n) => indegree.get(n.id) === 0).map((n) => n.id)
  for (const id of queue) {
    if (graph.nodes.find((n) => n.id === id)?.type === "trigger") depth.set(id, 0)
  }
  const remaining = new Map(indegree)
  while (queue.length > 0) {
    const id = queue.shift() as string
    for (const edge of graph.edges.filter((e) => e.from === id)) {
      if (depth.has(id)) {
        depth.set(edge.to, Math.max(depth.get(edge.to) ?? 0, (depth.get(id) ?? 0) + 1))
      }
      const left = (remaining.get(edge.to) ?? 1) - 1
      remaining.set(edge.to, left)
      if (left === 0) queue.push(edge.to)
    }
  }
  const last = Math.max(0, ...depth.values()) + 1
  return new Map(graph.nodes.map((n) => [n.id, depth.get(n.id) ?? last]))
}

/** Connections that cross between neighbouring columns of `order`. */
export function crossings(graph: WorkflowGraph, order: Map<string, number>): number {
  const column = columns(graph)
  const spans = graph.edges
    .filter((e) => (column.get(e.to) ?? 0) === (column.get(e.from) ?? 0) + 1)
    .map((e) => ({
      column: column.get(e.from) ?? 0,
      from: (order.get(e.from) ?? 0) + PORT_OFFSET[e.port],
      to: order.get(e.to) ?? 0,
    }))
  let count = 0
  for (let i = 0; i < spans.length; i++) {
    for (let j = i + 1; j < spans.length; j++) {
      const [a, b] = [spans[i], spans[j]]
      if (a && b && a.column === b.column && (a.from - b.from) * (a.to - b.to) < 0) count++
    }
  }
  return count
}

/** The vertical order of nodes within each column. */
function layerOrder(graph: WorkflowGraph): Map<string, number> {
  const column = columns(graph)
  const layers = new Map<number, string[]>()
  const byPosition = [...graph.nodes].sort(
    (a, b) => a.position.y - b.position.y || a.id.localeCompare(b.id),
  )
  for (const node of byPosition) {
    const c = column.get(node.id) ?? 0
    layers.set(c, [...(layers.get(c) ?? []), node.id])
  }
  const order = new Map<string, number>()
  const index = () => {
    for (const ids of layers.values()) ids.forEach((id, i) => order.set(id, i))
  }
  index()
  const sorted = [...layers.keys()].sort((a, b) => a - b)
  // Barycenter sweeps: order each column by the mean position of its
  // neighbours in the previous (forward) or next (backward) column.
  const sweep = (from: number[], neighbours: (id: string) => number[]) => {
    for (const c of from) {
      const ids = layers.get(c) ?? []
      const key = new Map(
        ids.map((id) => {
          const values = neighbours(id)
          const mean = values.length
            ? values.reduce((sum, v) => sum + v, 0) / values.length
            : (order.get(id) ?? 0)
          return [id, mean]
        }),
      )
      ids.sort((a, b) => (key.get(a) ?? 0) - (key.get(b) ?? 0) || a.localeCompare(b))
      index()
    }
  }
  const previous = (id: string) =>
    graph.edges
      .filter((e) => e.to === id && (column.get(e.from) ?? 0) < (column.get(id) ?? 0))
      .map((e) => (order.get(e.from) ?? 0) + PORT_OFFSET[e.port])
  const next = (id: string) =>
    graph.edges
      .filter((e) => e.from === id && (column.get(e.to) ?? 0) > (column.get(id) ?? 0))
      .map((e) => (order.get(e.to) ?? 0) - PORT_OFFSET[e.port])
  for (let pass = 0; pass < 4; pass++) {
    sweep(sorted.slice(1), previous)
    sweep(sorted.slice(0, -1).reverse(), next)
  }
  sweep(sorted.slice(1), previous)
  return order
}

/**
 * Arranges nodes in columns by their longest distance from the start, and
 * orders each column to keep branches together and connections from
 * crossing (Yes above No). Columns are centred on the start's row.
 * Unconnected nodes go into a last column. Only runs when asked for.
 */
export function autoLayout(graph: WorkflowGraph): WorkflowGraph {
  const column = columns(graph)
  const order = layerOrder(graph)
  const sizes = new Map<number, number>()
  for (const id of order.keys()) {
    const c = column.get(id) ?? 0
    sizes.set(c, (sizes.get(c) ?? 0) + 1)
  }
  return {
    ...graph,
    nodes: graph.nodes.map((n) => {
      const c = column.get(n.id) ?? 0
      const size = sizes.get(c) ?? 1
      return {
        ...n,
        position: { x: c * COLUMN, y: ((order.get(n.id) ?? 0) - (size - 1) / 2) * ROW },
      }
    }),
  }
}

/** The order of nodes in the layout, for tests and crossing counts. */
export function layoutOrder(graph: WorkflowGraph): Map<string, number> {
  return layerOrder(graph)
}

/** Undo and redo of committed changes. */
export interface History {
  past: WorkflowGraph[]
  present: WorkflowGraph
  future: WorkflowGraph[]
}

const HISTORY_LIMIT = 100

export function history(graph: WorkflowGraph): History {
  return { past: [], present: graph, future: [] }
}

export function commit(state: History, graph: WorkflowGraph): History {
  if (graph === state.present || JSON.stringify(graph) === JSON.stringify(state.present)) {
    return state
  }
  return {
    past: [...state.past, state.present].slice(-HISTORY_LIMIT),
    present: graph,
    future: [],
  }
}

/** Replaces the present without an undo step (e.g. while dragging). */
export function replace(state: History, graph: WorkflowGraph): History {
  return { ...state, present: graph }
}

/**
 * Ends a series of replacements (a drag) that started at `before`: one
 * undo step returns to it.
 */
export function settle(state: History, before: WorkflowGraph): History {
  if (JSON.stringify(before) === JSON.stringify(state.present)) return state
  return { past: [...state.past, before].slice(-HISTORY_LIMIT), present: state.present, future: [] }
}

export function undo(state: History): History {
  const previous = state.past.at(-1)
  if (!previous) return state
  return {
    past: state.past.slice(0, -1),
    present: previous,
    future: [state.present, ...state.future],
  }
}

export function redo(state: History): History {
  const next = state.future[0]
  if (!next) return state
  return { past: [...state.past, state.present], present: next, future: state.future.slice(1) }
}
