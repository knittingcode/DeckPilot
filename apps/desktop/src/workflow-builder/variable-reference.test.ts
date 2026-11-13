import { describe, expect, it } from "vitest"

import type { GraphMapping, ParameterKind, VariableDefinition } from "@/lib/types"

import {
  mappingReferences,
  mappingVariables,
  parseTemplate,
  previewMapping,
  serializeTemplate,
  variableProblems,
} from "./mapping"
import { assignable, referenceSyntaxProblem } from "./variable-reference"

const text: ParameterKind = { type: "text", multiline: false }
const integer: ParameterKind = { type: "integer" }
const project: ParameterKind = { type: "project" }
const definitions: VariableDefinition[] = [
  { key: "paths.root", valueType: { type: "string" }, scopes: ["global"] },
  { key: "projects.favorite", valueType: { type: "projectId" }, scopes: ["profile"] },
  { key: "creds.token", valueType: { type: "secretRef" }, scopes: ["global"] },
]

describe("variable references", () => {
  it("accepts the roots of the grammar and nothing else", () => {
    for (const ok of [
      "shared.paths.root",
      " shared.a.b.c.d ",
      "system.project.current.key",
      "workflow.execution.id",
      "steps.git.output.branch",
    ]) {
      expect(referenceSyntaxProblem(ok), ok).toBeNull()
    }
    for (const bad of [
      "",
      "shared.paths",
      "shared.Paths.root",
      "shared.system.x",
      "system.nothing",
      "env.HOME",
      "steps.git.branch",
    ]) {
      expect(referenceSyntaxProblem(bad), bad).not.toBeNull()
    }
  })

  it("keeps native types like the engine", () => {
    expect(assignable({ type: "projectId" }, project)).toBe(true)
    expect(assignable({ type: "projectId" }, integer)).toBe(false)
    expect(assignable({ type: "integer" }, integer)).toBe(true)
    expect(assignable({ type: "string" }, text)).toBe(true)
    expect(assignable({ type: "secretRef" }, text)).toBe(false)
    expect(assignable({ type: "secretRef" }, { type: "credential" })).toBe(true)
    expect(assignable({ type: "resource" }, { type: "resource", kind: "directory" })).toBe(true)
  })
})

describe("templates with variables", () => {
  it("parse into parts and serialize back unchanged", () => {
    const template =
      "cd {{ shared.paths.root }}/{{ steps.repo.output.value }} on {{system.platform}}"
    const parts = parseTemplate(template)
    expect(parts).toEqual([
      { type: "text", text: "cd " },
      { type: "variable", reference: "shared.paths.root" },
      { type: "text", text: "/" },
      { type: "output", reference: { node: "repo", output: "value" } },
      { type: "text", text: " on " },
      { type: "variable", reference: "system.platform" },
    ])
    expect(serializeTemplate(parts)).toBe(
      "cd {{ shared.paths.root }}/{{ steps.repo.output.value }} on {{ system.platform }}",
    )
  })

  it("leave anything else in braces as text", () => {
    expect(parseTemplate("{{ env.HOME }} and {{ shared.x }}")).toEqual([
      { type: "text", text: "{{ env.HOME }} and {{ shared.x }}" },
    ])
  })

  it("previews variables as placeholders, never values", () => {
    const mapping: GraphMapping = {
      type: "template",
      template: "cd {{ shared.paths.root }} {{ steps.a.output.b }}",
    }
    expect(previewMapping(mapping, { a: { b: "x" } })).toBe("cd ‹shared.paths.root› x")
    expect(mappingVariables(mapping)).toEqual(["shared.paths.root"])
    expect(mappingReferences(mapping)).toEqual([{ node: "a", output: "b" }])
  })
})

describe("variable mappings", () => {
  const variable = (reference: string, extra: Partial<GraphMapping> = {}): GraphMapping =>
    ({ type: "variable", reference, ...extra }) as GraphMapping

  it("report types that cannot fill the parameter", () => {
    expect(variableProblems(variable("shared.projects.favorite"), project, definitions)).toEqual([])
    expect(variableProblems(variable("system.project.current.id"), integer)).toEqual([
      "{{ system.project.current.id }} is a projectId value, the parameter expects a whole number",
    ])
    expect(variableProblems(variable("shared.paths.nothing"), text, definitions)).toEqual([
      "there is no shared variable “paths.nothing”",
    ])
    // Without definitions shared variables are checked when the node runs.
    expect(variableProblems(variable("shared.paths.nothing"), text)).toEqual([])
    expect(
      variableProblems(
        variable("shared.paths.root", { default: { type: "boolean", value: true } }),
        integer,
        definitions,
      ),
    ).toEqual([
      "{{ shared.paths.root }} is a string value, the parameter expects a whole number",
      "the default is a boolean value, the parameter expects a whole number",
    ])
  })

  it("refuse credential names in text", () => {
    expect(
      variableProblems(
        { type: "template", template: "token {{ shared.creds.token }}" },
        text,
        definitions,
      ),
    ).toEqual(["{{ shared.creds.token }} is a secretRef value and cannot be part of text"])
  })

  it("treat a steps reference like an output", () => {
    const mapping = variable("steps.repo.output.value")
    expect(mappingReferences(mapping)).toEqual([{ node: "repo", output: "value" }])
    expect(mappingVariables(mapping)).toEqual([])
    expect(previewMapping(mapping, { repo: { value: "/src" } })).toBe("/src")
    expect(previewMapping(variable("shared.paths.root"))).toBe("‹shared.paths.root›")
  })
})
