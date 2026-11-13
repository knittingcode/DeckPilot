import { describe, expect, it } from "vitest"

import type { CatalogEntry, SharedVariable } from "./types"
import {
  NO_FILTER,
  catalogRows,
  filterRows,
  insertedReference,
  ownerLabel,
  ownerOf,
  withoutDeprecated,
} from "./variable-catalog"

const entry = (change: Partial<CatalogEntry>): CatalogEntry => ({
  reference: "system.platform",
  label: "Platform",
  group: "system",
  valueType: { type: "string" },
  availability: "available",
  ...change,
})

const canonical = entry({
  reference: "projects.current.key",
  label: "Project key",
  group: "module",
  owner: { kind: "module", module: "projects" },
  declaredScope: "project",
})
const legacy = entry({
  reference: "system.project.current.key",
  label: "Project key",
  owner: { kind: "module", module: "projects" },
  deprecated: { canonical: "projects.current.key", since: "0.6.0" },
})

const shared: SharedVariable = {
  definition: { key: "paths.root", valueType: { type: "string" }, scopes: ["global", "project"] },
  values: [],
  references: [],
}

describe("owners", () => {
  it("come from the agent, or from the reference for older agents", () => {
    expect(ownerOf(canonical)).toEqual({ kind: "module", module: "projects" })
    expect(ownerOf(entry({ reference: "system.project.current.id" }))).toEqual({
      kind: "module",
      module: "projects",
    })
    expect(ownerOf(entry({ reference: "system.work.session.state" }))).toEqual({
      kind: "module",
      module: "work-session",
    })
    expect(ownerOf(entry({}))).toEqual({ kind: "core" })
    expect(ownerOf(entry({ reference: "shared.paths.root" }))).toEqual({ kind: "user" })
    expect(ownerLabel({ kind: "core" })).toBe("DeckPilot")
    expect(ownerLabel({ kind: "module", module: "projects" }, { projects: "Projects" })).toBe(
      "Projects module",
    )
  })
})

describe("the picker", () => {
  it("offers canonical ids and inserts them", () => {
    expect(withoutDeprecated([legacy, canonical]).map((e) => e.reference)).toEqual([
      "projects.current.key",
    ])
    // An older agent lists legacy ids only: they stay.
    const old = entry({ reference: "system.project.current.key" })
    expect(withoutDeprecated([old])).toEqual([old])
    expect(insertedReference(legacy)).toBe("projects.current.key")
    expect(insertedReference(canonical)).toBe("projects.current.key")
  })
})

describe("catalog rows", () => {
  it("fold legacy ids into their canonical row and add shared variables", () => {
    const rows = catalogRows(
      [entry({ owner: { kind: "core" }, declaredScope: "machine" }), legacy, canonical],
      [shared],
    )
    expect(rows.map((r) => [r.reference, r.readOnly, r.aliases])).toEqual([
      ["system.platform", true, []],
      ["projects.current.key", true, ["system.project.current.key"]],
      ["shared.paths.root", false, []],
    ])
    expect(rows[2]?.scopes).toEqual(["global", "project"])
    expect(rows[2]?.sharedKey).toBe("paths.root")
  })

  it("filter by words, owner, type and scope", () => {
    const rows = catalogRows([legacy, canonical, entry({ declaredScope: "machine" })], [shared])
    const refs = (filter: Partial<typeof NO_FILTER>) =>
      filterRows(rows, { ...NO_FILTER, ...filter }).map((r) => r.reference)
    expect(refs({ owner: "user" })).toEqual(["shared.paths.root"])
    expect(refs({ owner: "module:projects" })).toEqual(["projects.current.key"])
    expect(refs({ owner: "core" })).toEqual(["system.platform"])
    expect(refs({ scope: "project" })).toEqual(["projects.current.key", "shared.paths.root"])
    expect(refs({ type: "integer" })).toEqual([])
    expect(refs({ query: "system.project KEY" })).toEqual(["projects.current.key"])
  })
})
