// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"

import type { GraphNode, Workflow, WorkflowGraph } from "@/lib/types"

import type { Lookup } from "./describe"
import { DEFAULT_POLICY } from "./graph"
import { NodeInspector } from "./inspector"
import { outputOptions } from "./mapping"

const at = { x: 0, y: 0 }

const greet: Workflow = {
  id: "w-greet",
  name: "Greet",
  steps: [],
  inputs: [
    { key: "name", label: "Name", kind: "text", required: true },
    { key: "times", kind: "integer", default: "2" },
  ],
  outputs: [
    { key: "greeting", label: "Greeting", kind: "text" },
    { key: "token", kind: "text", sensitive: true },
  ],
}

const lookup: Lookup = {
  actions: [],
  types: [],
  workflows: [greet],
  projects: [],
  inputs: [{ key: "who", label: "Who", kind: "text" }],
  outputs: [{ key: "result", label: "Result", kind: "integer" }],
}

function graphWith(node: GraphNode): WorkflowGraph {
  return {
    version: 1,
    nodes: [{ id: "start", type: "trigger", position: at }, node],
    edges: [{ from: "start", port: "out", to: node.id }],
  }
}

function inspect(node: GraphNode, onChange: (node: GraphNode) => void = () => undefined) {
  return render(
    <NodeInspector
      graph={graphWith(node)}
      node={node}
      issues={[
        { node: node.id, severity: "error", message: "Greet needs a value for its input `name`" },
      ]}
      lookup={lookup}
      projects={[]}
      loopTargets={[{ id: greet.id, name: greet.name }]}
      onChange={(change) => {
        onChange(change(node))
      }}
      onDelete={() => undefined}
    />,
  )
}

describe("sub-workflow inspector", () => {
  afterEach(cleanup)

  const call: GraphNode = {
    id: "greet",
    type: "call",
    workflow: greet.id,
    inputs: {
      times: { type: "template", template: "{{ steps.start.output.who }}" },
      gone: { type: "template", template: "x" },
    },
    policy: DEFAULT_POLICY,
    position: at,
  }

  it("maps the called workflow's inputs and lists its outputs", () => {
    let changed: GraphNode | undefined
    inspect(call, (node) => (changed = node))
    expect(screen.getByRole("heading", { name: "Sub-workflow", level: 2 })).toBeTruthy()
    // Validation messages show like on any node.
    expect(screen.getByText(/needs a value for its input/)).toBeTruthy()
    // A required input without a mapping says so; the optional one is mapped
    // from the start node's input, with a preview.
    expect(screen.getByText("Needs a value.")).toBeTruthy()
    expect(screen.getByLabelText("Preview").textContent).toBe("Preview: ‹start.who›")
    // Outputs of the called workflow, with sensitive ones marked.
    const outputs = screen.getByRole("list", { name: "Sub-workflow outputs" })
    expect(within(outputs).getByText("greeting")).toBeTruthy()
    expect(within(outputs).getByText(/not kept in history/)).toBeTruthy()
    // An input the workflow no longer declares can be removed.
    expect(screen.getByText(/is no longer an input of Greet/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Remove" }))
    expect(changed?.type === "call" && Object.keys(changed.inputs ?? {})).toEqual(["times"])
  })

  it("offers the start node's inputs and a sub-workflow's outputs to later nodes", () => {
    const graph: WorkflowGraph = {
      version: 1,
      nodes: [
        { id: "start", type: "trigger", position: at },
        call,
        { id: "out", type: "output", values: {}, position: at },
      ],
      edges: [
        { from: "start", port: "out", to: "greet" },
        { from: "greet", port: "out", to: "out" },
      ],
    }
    expect(outputOptions(graph, "out", lookup).map((o) => [o.label, o.kind])).toEqual([
      ["Inputs · Who", "text"],
      ["Greet · Greeting", "text"],
      ["Greet · token", "text"],
    ])
  })

  it("sets the workflow's declared outputs in an Output node", () => {
    let changed: GraphNode | undefined
    const output: GraphNode = { id: "out", type: "output", values: {}, position: at }
    inspect(output, (node) => (changed = node))
    expect(screen.getByText(/Result/)).toBeTruthy()
    expect(screen.getByText("Not set by this node.")).toBeTruthy()
    expect(changed).toBeUndefined()
  })
})
