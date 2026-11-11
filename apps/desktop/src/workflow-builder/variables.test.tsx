// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { useState } from "react"
import { afterEach, describe, expect, it } from "vitest"

import type { GraphNode, WorkflowGraph, WorkflowVariable } from "@/lib/types"

import { describeNode, type Lookup } from "./describe"
import { nodeId } from "./graph"
import { NodeInspector } from "./inspector"
import { outputOptions, referenceProblem } from "./mapping"
import { VariablesEditor } from "./variables-editor"

const at = { x: 0, y: 0 }

const lookup: Lookup = {
  actions: [],
  types: [],
  workflows: [],
  projects: [],
  variables: [
    { key: "count", label: "Count", kind: "integer", initial: "1" },
    { key: "token", kind: "text", sensitive: true },
  ],
}

const set: GraphNode = {
  id: "set",
  type: "setVariable",
  values: { count: { type: "template", template: "2" } },
  position: at,
}

const graph: WorkflowGraph = {
  version: 1,
  nodes: [{ id: "start", type: "trigger", position: at }, set],
  edges: [{ from: "start", port: "out", to: "set" }],
}

describe("variables", () => {
  afterEach(cleanup)

  it("offers variables in the output picker and checks references to them", () => {
    const options = outputOptions(graph, "set", lookup)
    expect(options.map((o) => [o.reference.node, o.reference.output, o.label, o.kind])).toEqual([
      ["vars", "count", "Variable · Count", "integer"],
      ["vars", "token", "Variable · token", "text"],
    ])
    expect(options[1]?.sensitive).toBe(true)
    expect(referenceProblem(graph, "set", { node: "vars", output: "count" }, lookup)).toBeNull()
    expect(referenceProblem(graph, "set", { node: "vars", output: "nope" }, lookup)).toBe(
      "there is no variable “nope”",
    )
    // New nodes never take the reserved id.
    expect(nodeId("Vars", [])).toBe("vars_2")
    expect(describeNode(set, lookup)).toEqual({ title: "Set variable", detail: "count" })
  })

  it("sets declared variables in the inspector", () => {
    let changed: GraphNode | undefined
    render(
      <NodeInspector
        graph={graph}
        node={set}
        issues={[]}
        lookup={lookup}
        projects={[]}
        loopTargets={[]}
        onChange={(change) => {
          changed = change(set)
        }}
        onDelete={() => undefined}
      />,
    )
    expect(screen.getByRole("heading", { name: "Set variable", level: 2 })).toBeTruthy()
    expect(screen.getByText(/· sensitive/)).toBeTruthy()
    expect(screen.getByText("Not changed by this node.")).toBeTruthy()
    expect(screen.getByLabelText("Preview").textContent).toBe("Preview: 2")
    fireEvent.click(screen.getByRole("button", { name: /Text/ }))
    expect(changed?.type === "setVariable" && changed.values?.count).toEqual({
      type: "template",
      template: "2",
    })
  })

  it("declares variables in the workflow panel", () => {
    let latest: WorkflowVariable[] = []
    function Harness() {
      const [variables, setVariables] = useState<WorkflowVariable[]>([])
      return (
        <VariablesEditor
          variables={variables}
          onChange={(next) => {
            setVariables(next)
            latest = next
          }}
        />
      )
    }
    render(<Harness />)
    fireEvent.click(screen.getByRole("button", { name: /Add variable/ }))
    expect(latest).toEqual([{ key: "var_1", kind: "text" }])
    fireEvent.change(screen.getByLabelText("Variable key"), { target: { value: "branch" } })
    fireEvent.change(screen.getByLabelText("Variable initial value"), {
      target: { value: "main" },
    })
    fireEvent.click(screen.getByRole("checkbox"))
    expect(latest).toEqual([{ key: "branch", kind: "text", initial: "main", sensitive: true }])
    fireEvent.click(screen.getByRole("button", { name: "Remove variable branch" }))
    expect(latest).toEqual([])
  })
})
