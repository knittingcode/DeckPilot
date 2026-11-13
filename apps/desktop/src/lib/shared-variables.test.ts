import { describe, expect, it } from "vitest"

import {
  draftOf,
  formatValue,
  keyProblem,
  parseValue,
  scopeLabel,
  slotsFor,
} from "./shared-variables"
import type { Project, VariableDefinition } from "./types"

const shop: Project = { id: "p-shop", key: "SHOP", name: "Shop", resources: {} }

describe("shared variable keys", () => {
  it("follows the rules of VariableKey", () => {
    expect(keyProblem("paths.root", [])).toBeNull()
    expect(keyProblem("a.b.c.d", [])).toBeNull()
    expect(keyProblem("projects.favorite", [])).toBeNull()
    expect(keyProblem("", [])).toBe("Enter a key.")
    expect(keyProblem("root", [])).toMatch(/2 to 4 parts/)
    expect(keyProblem("a.b.c.d.e", [])).toMatch(/2 to 4 parts/)
    expect(keyProblem("paths.Root", [])).toMatch(/lowercase letter/)
    expect(keyProblem("paths.root-dir", [])).toMatch(/lowercase letter/)
    expect(keyProblem(`paths.${"a".repeat(33)}`, [])).toMatch(/at most 32/)
    for (const reserved of ["steps", "system", "workflow"]) {
      expect(keyProblem(`${reserved}.x`, [])).toBe(`“${reserved}” is reserved.`)
    }
    expect(keyProblem("paths.root", ["paths.root"])).toMatch(/exists/)
  })
})

describe("values", () => {
  it("parses drafts by type and round-trips existing values", () => {
    expect(parseValue({ type: "integer" }, { text: " 42 " })).toEqual({
      value: { type: "integer", value: 42 },
    })
    expect(parseValue({ type: "integer" }, { text: "4.5" })).toEqual({
      error: "Enter a whole number.",
    })
    expect(parseValue({ type: "json" }, { text: "{" })).toEqual({ error: "Enter valid JSON." })
    expect(parseValue({ type: "secretRef" }, { text: "" })).toEqual({
      error: "Choose a stored credential.",
    })
    expect(parseValue({ type: "resource" }, { text: "dev", mode: "projectRole" })).toEqual({
      value: { type: "resource", value: { type: "projectRole", role: "dev" } },
    })
    const value = { type: "json", value: { a: [1] } } as const
    expect(parseValue({ type: "json" }, draftOf(value))).toEqual({ value })
  })

  it("never formats a sensitive value", () => {
    const definition: VariableDefinition = {
      key: "deploy.token",
      valueType: { type: "string" },
      scopes: ["global"],
      sensitive: true,
    }
    expect(formatValue(definition, { type: "string", value: "secret" }, [], [])).toBe("Redacted")
    expect(
      formatValue(
        { ...definition, sensitive: false, valueType: { type: "secretRef" } },
        { type: "secretRef", value: "github" },
        [],
        [],
      ),
    ).toBe("Credential github")
  })

  it("offers the scopes the definition allows for the edited profile", () => {
    const definition: VariableDefinition = {
      key: "paths.root",
      valueType: { type: "string" },
      scopes: ["global", "profile", "project"],
    }
    const slots = slotsFor(definition, "profile-1", [shop])
    expect(
      slots.map((s) => scopeLabel(s, [shop], [{ id: "profile-1", name: "Work", updatedAt: 0 }])),
    ).toEqual(["Global", "Profile Work", "Project SHOP"])
    expect(slotsFor(definition, null, [shop])).toEqual([{ kind: "global" }])
  })
})
