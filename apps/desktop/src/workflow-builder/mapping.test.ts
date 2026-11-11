import { describe, expect, it } from "vitest"

import type { ActionInstance, ActionType, WorkflowGraph } from "@/lib/types"

import {
  compatibility,
  mappingReferences,
  outputOptions,
  parseTemplate,
  previewMapping,
  referenceProblem,
  serializeTemplate,
} from "./mapping"

const repository: ActionType = {
  id: "projects.repository",
  name: "Project repository",
  description: "",
  category: "",
  parameters: [],
  requiresProject: false,
  node: {
    version: 1,
    icon: null,
    outputs: [
      { key: "value", label: "Path", kind: "path", always: true },
      { key: "projectKey", label: "Project key", kind: "projectKey", always: true },
    ],
    timeoutMs: 30000,
    retrySafe: true,
    idempotent: true,
    changesContext: false,
  },
}
const actions: ActionInstance[] = [
  { id: "a1", name: "Repo", actionType: "projects.repository", parameters: {} },
]
const lookup = { actions, types: [repository], workflows: [], projects: [] }
const policy = { onError: "stop" as const, retry: { attempts: 0, backoffMs: 0 } }
const at = { x: 0, y: 0 }
const graph: WorkflowGraph = {
  version: 1,
  nodes: [
    { id: "start", type: "trigger", position: at },
    { id: "repo", type: "action", action: "a1", policy, position: at },
    { id: "use", type: "action", action: "a1", policy, position: at },
    { id: "elsewhere", type: "action", action: "a1", policy, position: at },
  ],
  edges: [
    { from: "start", port: "out", to: "repo" },
    { from: "repo", port: "out", to: "use" },
  ],
}

describe("templates", () => {
  it("round-trip between text and parts", () => {
    const template = "Open {{ steps.repo.output.value }} for {{steps.repo.output.projectKey}}!"
    const parts = parseTemplate(template)
    expect(parts).toEqual([
      { type: "text", text: "Open " },
      { type: "output", reference: { node: "repo", output: "value" } },
      { type: "text", text: " for " },
      { type: "output", reference: { node: "repo", output: "projectKey" } },
      { type: "text", text: "!" },
    ])
    expect(serializeTemplate(parts)).toBe(
      "Open {{ steps.repo.output.value }} for {{ steps.repo.output.projectKey }}!",
    )
    expect(parseTemplate("plain")).toEqual([{ type: "text", text: "plain" }])
    expect(mappingReferences({ type: "template", template })).toHaveLength(2)
  })
})

describe("references", () => {
  it("offer only earlier nodes' declared outputs, with their types", () => {
    const options = outputOptions(graph, "use", lookup)
    expect(options.map((o) => `${o.reference.node}.${o.reference.output}:${o.kind}`)).toEqual([
      "repo.value:path",
      "repo.projectKey:projectKey",
    ])
    expect(outputOptions(graph, "repo", lookup)).toEqual([])
  })

  it("report missing and unreachable references", () => {
    expect(referenceProblem(graph, "use", { node: "repo", output: "value" }, lookup)).toBeNull()
    expect(referenceProblem(graph, "use", { node: "repo", output: "nope" }, lookup)).toMatch(
      /no output/,
    )
    expect(referenceProblem(graph, "use", { node: "elsewhere", output: "value" }, lookup)).toMatch(
      /does not run before/,
    )
    expect(referenceProblem(graph, "use", { node: "ghost", output: "value" }, lookup)).toMatch(
      /no node/,
    )
  })
})

describe("compatibility", () => {
  it("follows the engine's conversions", () => {
    expect(compatibility({ type: "text", multiline: false }, "path").level).toBe("ok")
    expect(compatibility({ type: "integer" }, "integer").level).toBe("ok")
    expect(compatibility({ type: "integer" }, "path").level).toBe("warning")
    expect(compatibility({ type: "boolean" }, "text-template").level).toBe("warning")
    expect(compatibility({ type: "project" }, "projectKey").level).toBe("ok")
    expect(compatibility({ type: "project" }, "text").level).toBe("warning")
    expect(compatibility({ type: "resource" }, "text").level).toBe("error")
  })
})

describe("previews", () => {
  it("use known values and name the rest", () => {
    const template = { type: "template" as const, template: "at {{ steps.repo.output.value }}" }
    expect(previewMapping(template)).toBe("at ‹repo.value›")
    expect(previewMapping(template, { repo: { value: "/src/shop" } })).toBe("at /src/shop")
    const single = {
      type: "output" as const,
      reference: { node: "repo", output: "value" },
      default: "/tmp",
    }
    expect(previewMapping(single)).toBe("/tmp")
  })
})
