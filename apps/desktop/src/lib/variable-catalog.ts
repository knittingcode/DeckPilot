// The unified variable catalog (docs/variables.md): one list of every
// variable, whoever owns it, for the Variables page and the picker. Values
// are never computed here; they come from the agent's previews.

import type {
  CatalogEntry,
  DeclaredScope,
  SharedVariable,
  VariableOwner,
  VariableScope,
  VariableType,
} from "./types"

/**
 * Legacy ids and the module-owned ids they resolve as. Mirrors
 * `deckpilot_core::variables::ALIASES` (checked by a Rust test). Used to
 * name owners when the agent cannot (an agent before protocol 9).
 */
export const LEGACY_ALIASES: Record<string, { canonical: string; owner: string }> = {
  "system.project.current.id": { canonical: "projects.current.id", owner: "projects" },
  "system.project.current.key": { canonical: "projects.current.key", owner: "projects" },
  "system.project.current.name": { canonical: "projects.current.name", owner: "projects" },
  "system.work.session.id": { canonical: "workSession.id", owner: "work-session" },
  "system.work.session.state": { canonical: "workSession.state", owner: "work-session" },
  "system.work.session.startedAt": { canonical: "workSession.startedAt", owner: "work-session" },
  "system.work.session.elapsedSeconds": {
    canonical: "workSession.elapsedSeconds",
    owner: "work-session",
  },
  "system.work.session.projectId": { canonical: "workSession.projectId", owner: "work-session" },
  "system.work.session.canFinish": { canonical: "workSession.canFinish", owner: "work-session" },
}

/** Who owns `entry`: as the agent says, else derived from the reference. */
export function ownerOf(entry: CatalogEntry): VariableOwner {
  if (entry.owner) return entry.owner
  const alias = LEGACY_ALIASES[entry.reference]
  if (alias) return { kind: "module", module: alias.owner }
  if (entry.module) return { kind: "module", module: entry.module }
  const root = entry.reference.split(".")[0]
  if (root === "shared") return { kind: "user" }
  if (root === "system" || root === "workflow") return { kind: "core" }
  return { kind: "module", module: root ?? "" }
}

/** The owner in words; `names` maps module ids to their display names. */
export function ownerLabel(owner: VariableOwner, names: Record<string, string> = {}): string {
  switch (owner.kind) {
    case "core":
      return "DeckPilot"
    case "user":
      return "You"
    case "module":
      return `${names[owner.module] ?? owner.module} module`
  }
}

/**
 * The entries the picker offers: a legacy id is hidden when its canonical
 * id is listed too, so the picker inserts canonical ids. With an older
 * agent, which lists only legacy ids, they stay.
 */
export function withoutDeprecated(entries: CatalogEntry[]): CatalogEntry[] {
  const listed = new Set(entries.map((e) => e.reference))
  return entries.filter((e) => !e.deprecated || !listed.has(e.deprecated.canonical))
}

/** The reference to insert for `entry`: the canonical id of a legacy one. */
export function insertedReference(entry: CatalogEntry): string {
  return entry.deprecated?.canonical ?? entry.reference
}

/** A scope as the Variables page filters it: stored scopes and declared ones. */
export type CatalogScope = VariableScope | DeclaredScope

export const CATALOG_SCOPE_LABEL: Record<CatalogScope, string> = {
  global: "Global",
  profile: "Profile",
  project: "Project",
  execution: "Run",
  machine: "This computer",
  device: "Device",
  workflow: "Workflow",
  workSession: "Work session",
}

/** One line of the Variables page. */
export interface VariableRow {
  /** The canonical reference without braces, copied and inserted. */
  reference: string
  name: string
  description?: string
  owner: VariableOwner
  valueType: VariableType
  scopes: CatalogScope[]
  sensitive: boolean
  /** Core and module variables, and shared variables a module declares. */
  readOnly: boolean
  availability: CatalogEntry["availability"]
  /** Deprecated ids that still resolve as this one. */
  aliases: string[]
  /** For user variables: the key to edit. */
  sharedKey?: string
  /** For user variables: how often it is used. */
  usages?: number
}

/**
 * The rows of the Variables page: core and module variables from the
 * agent's catalog (legacy ids folded into their canonical row), then the
 * shared variables from the store.
 */
export function catalogRows(entries: CatalogEntry[], shared: SharedVariable[]): VariableRow[] {
  const rows: VariableRow[] = []
  const byReference = new Map<string, VariableRow>()
  const aliases: [string, string][] = []
  for (const entry of entries) {
    if (entry.reference.startsWith("shared.") || entry.group === "stepOutput") continue
    const canonical = entry.deprecated?.canonical
    if (canonical && entries.some((e) => e.reference === canonical)) {
      aliases.push([canonical, entry.reference])
      continue
    }
    const row: VariableRow = {
      reference: entry.reference,
      name: entry.label,
      description: entry.description,
      owner: ownerOf(entry),
      valueType: entry.valueType,
      scopes: entry.declaredScope ? [entry.declaredScope] : [],
      sensitive: Boolean(entry.sensitive),
      readOnly: true,
      availability: entry.availability,
      aliases: [],
    }
    rows.push(row)
    byReference.set(row.reference, row)
  }
  for (const [canonical, alias] of aliases) byReference.get(canonical)?.aliases.push(alias)
  const states = new Map(entries.map((e) => [e.reference, e.availability]))
  for (const { definition: d, references } of shared) {
    const reference = `shared.${d.key}`
    rows.push({
      reference,
      name: d.label || d.key,
      description: d.description,
      owner: d.module ? { kind: "module", module: d.module } : { kind: "user" },
      valueType: d.valueType,
      scopes: d.scopes,
      sensitive: Boolean(d.sensitive),
      readOnly: Boolean(d.module),
      availability: states.get(reference) ?? "available",
      aliases: [],
      sharedKey: d.key,
      usages: references.filter((r) => r.use.type !== "trigger").length,
    })
  }
  return rows
}

export type OwnerFilter = "all" | "core" | "user" | `module:${string}`

export interface RowFilter {
  query: string
  owner: OwnerFilter
  type: VariableType["type"] | "all"
  scope: CatalogScope | "all"
}

export const NO_FILTER: RowFilter = { query: "", owner: "all", type: "all", scope: "all" }

/** The owner filter value of `owner`. */
export function ownerKey(owner: VariableOwner): OwnerFilter {
  return owner.kind === "module" ? `module:${owner.module}` : owner.kind
}

/** Rows matching every part of `filter`; words match anywhere, ignoring case. */
export function filterRows(rows: VariableRow[], filter: RowFilter): VariableRow[] {
  const words = filter.query.toLowerCase().split(/\s+/).filter(Boolean)
  return rows.filter((row) => {
    if (filter.owner !== "all" && ownerKey(row.owner) !== filter.owner) return false
    if (filter.type !== "all" && row.valueType.type !== filter.type) return false
    if (filter.scope !== "all" && !row.scopes.includes(filter.scope)) return false
    const haystack = [
      row.reference,
      row.name,
      row.description ?? "",
      row.owner.kind === "module" ? row.owner.module : row.owner.kind,
      row.valueType.type,
      ...row.aliases,
    ]
      .join("\n")
      .toLowerCase()
    return words.every((word) => haystack.includes(word))
  })
}
