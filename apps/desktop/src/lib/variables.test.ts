import { describe, expect, it } from "vitest"

import type { CatalogEntry, ParameterKind } from "./types"
import {
  SYSTEM_ENTRIES,
  assignableTo,
  autocompleteAt,
  describeContext,
  describeStatus,
  eventAffects,
  groupEntries,
  insertReference,
  matchesFilter,
  stepOutputEntries,
  templateReferences,
} from "./variables"

const entry = (change: Partial<CatalogEntry>): CatalogEntry => ({
  reference: "shared.paths.root",
  label: "Root folder",
  group: "shared",
  valueType: { type: "string" },
  availability: "available",
  ...change,
})

describe("autocompleteAt", () => {
  it("opens after an unclosed {{ and reads what follows", () => {
    expect(autocompleteAt("cd {{", 5)).toEqual({ start: 3, query: "" })
    expect(autocompleteAt("cd {{ shared.pa", 15)).toEqual({ start: 3, query: "shared.pa" })
  })

  it("stays closed without {{, after a closed reference and for other characters", () => {
    expect(autocompleteAt("cd /src", 7)).toBeNull()
    expect(autocompleteAt("{{ a.b }} x", 11)).toBeNull()
    expect(autocompleteAt("{{ a/b", 6)).toBeNull()
  })

  it("only looks before the cursor", () => {
    expect(autocompleteAt("ab {{ x", 2)).toBeNull()
  })
})

describe("insertReference", () => {
  it("inserts at the cursor and places the cursor after the reference", () => {
    expect(insertReference("hello world", 6, 6, "shared.paths.root")).toEqual({
      text: "hello {{ shared.paths.root }}world",
      cursor: 29,
    })
  })

  it("replaces a selection", () => {
    expect(insertReference("a XX b", 2, 4, "system.platform").text).toBe(
      "a {{ system.platform }} b",
    )
  })

  it("replaces the typed {{ and a closing }} that follows it", () => {
    expect(insertReference("cd {{ ro }}/x", 3, 8, "shared.paths.root").text).toBe(
      "cd {{ shared.paths.root }}/x",
    )
  })
})

describe("templateReferences", () => {
  it("lists each reference once and reports an unclosed one", () => {
    expect(templateReferences("{{ a.b }} {{a.b}} {{ c.d }}")).toEqual({
      references: ["a.b", "c.d"],
      unclosed: false,
    })
    expect(templateReferences("x {{ a.b }} {{ c")).toEqual({ references: ["a.b"], unclosed: true })
  })
})

describe("assignableTo", () => {
  const text: ParameterKind = { type: "text", multiline: false }
  it("mirrors the core rules", () => {
    expect(assignableTo({ type: "integer" }, { type: "integer" })).toBe(true)
    expect(assignableTo({ type: "integer" }, text)).toBe(true)
    expect(assignableTo({ type: "string" }, { type: "integer" })).toBe(false)
    expect(assignableTo({ type: "projectId" }, { type: "project" })).toBe(true)
    expect(assignableTo({ type: "secretRef" }, text)).toBe(false)
    expect(assignableTo({ type: "secretRef" }, { type: "credential" })).toBe(true)
    expect(
      assignableTo({ type: "resource", kind: "url" }, { type: "resource", kind: "directory" }),
    ).toBe(false)
    expect(assignableTo({ type: "resource" }, { type: "resource", kind: "directory" })).toBe(true)
    expect(assignableTo({ type: "json" }, { type: "code", language: "json" })).toBe(true)
    expect(assignableTo({ type: "json" }, { type: "lines" })).toBe(false)
  })
})

describe("matchesFilter", () => {
  const favorite = entry({
    reference: "shared.projects.favorite",
    label: "Favorite",
    description: "The project I use most",
    valueType: { type: "projectId" },
    scope: "profile",
    module: "projects",
  })

  it("finds by reference, label, description, scope, type and module", () => {
    for (const query of ["favorite", "use most", "profile", "projectid", "projects", "FAV pro"]) {
      expect(matchesFilter(favorite, { query })).toBe(true)
    }
    expect(matchesFilter(favorite, { query: "root" })).toBe(false)
  })

  it("filters by group, compatibility and availability", () => {
    expect(matchesFilter(favorite, { groups: ["system"] })).toBe(false)
    expect(matchesFilter(favorite, { compatibleWith: { type: "integer" } })).toBe(false)
    expect(matchesFilter(favorite, { compatibleWith: { type: "project" } })).toBe(true)
    const away = entry({ availability: "notInContext" })
    expect(matchesFilter(away, {})).toBe(false)
    expect(matchesFilter(away, { includeUnavailable: true })).toBe(true)
  })
})

describe("groups and step outputs", () => {
  it("orders groups like the picker and leaves empty ones out", () => {
    const groups = groupEntries([
      ...stepOutputEntries([
        { node: "build", output: "path", label: "Build · path", kind: "path" },
      ]),
      ...SYSTEM_ENTRIES.slice(0, 1),
      entry({}),
    ])
    expect(groups.map((g) => g.label)).toEqual(["Shared", "System", "Step outputs"])
  })

  it("lists step outputs and workflow variables as run-time only", () => {
    const [output, variable] = stepOutputEntries([
      { node: "build", output: "count", label: "Build · count", kind: "integer" },
      { node: "vars", output: "token", label: "Variable · token", kind: "text", sensitive: true },
    ])
    expect(output).toMatchObject({
      reference: "steps.build.output.count",
      group: "stepOutput",
      valueType: { type: "integer" },
      availability: "runtimeOnly",
    })
    expect(variable).toMatchObject({ group: "workflow", sensitive: true })
  })
})

describe("describeContext", () => {
  it("names where the project comes from", () => {
    expect(
      describeContext(
        { profile: "p1", project: "x", projectSource: "application" },
        { profile: "Work", project: "SHOP" },
      ),
    ).toBe("Profile Work · active project SHOP")
    expect(describeContext({ profile: "p1", projectSource: "execution", workflow: "w" })).toBe(
      "Profile p1 · the project of the run (known when it runs) · workflow w",
    )
    expect(describeContext({ profile: "p1", projectSource: "explicit", project: "x" })).toContain(
      "chosen for this preview",
    )
  })
})

describe("eventAffects", () => {
  const app = { profile: "p1", projectSource: "application" } as const
  const run = { profile: "p1", projectSource: "execution", execution: "e1" } as const
  it("keeps a run's context apart from the active project", () => {
    const changed = {
      type: "projectChanged",
      profile: "p1",
      project: "x",
      source: "application",
    } as const
    expect(eventAffects(changed, app)).toBe(true)
    expect(eventAffects(changed, run)).toBe(false)
    expect(eventAffects({ ...changed, profile: "p2" }, app)).toBe(false)
    expect(
      eventAffects({ type: "executionContextChanged", execution: "e1", project: null }, run),
    ).toBe(true)
    expect(
      eventAffects({ type: "executionContextChanged", execution: "e2", project: null }, run),
    ).toBe(false)
    expect(eventAffects({ type: "sharedVariablesChanged", keys: [] }, run)).toBe(true)
  })
})

describe("describeStatus", () => {
  it("never contains a value", () => {
    const redacted = describeStatus({
      status: "redacted",
      valueType: { type: "string" },
      source: { type: "default" },
    })
    expect(redacted.label).toBe("Redacted")
    expect(describeStatus({ status: "missing", reason: "noProject" })).toEqual({
      tone: "error",
      label: "Missing",
      detail: "No project is active.",
    })
    expect(describeStatus({ status: "typeMismatch", found: { type: "boolean" } }).detail).toContain(
      "yes/no",
    )
  })
})
