// A stand-in for the agent's variable resolver, behind the mocked
// `api.listVariables` / `api.previewReferences` (protocol 7 is not there
// yet). It answers like the contract in docs/action-architecture.md: the
// catalog carries metadata only, previews redact sensitive values and say
// when a value exists only while running.
//
//   const agent = fakeVariableAgent()
//   agent.install(api)

import type { Mock } from "vitest"

import type {
  CatalogEntry,
  CatalogFilter,
  PreviewItem,
  ReferencePreview,
  ReferenceStatus,
  ResolutionContext,
  VariableValue,
} from "@/lib/types"
import { assignableTo, matchesFilter } from "@/lib/variables"

/** The secret behind `shared.deploy.token`; must never reach the UI. */
export const SECRET = "s3cr3t-deploy-token"

export interface FakeVariable {
  entry: CatalogEntry
  /** The value in `context`, or a status the resolver reports instead. */
  resolve: (context: ResolutionContext) => VariableValue | ReferenceStatus
}

const PROJECT_KEYS: Record<string, string> = { "project-1": "SHOP", "project-2": "BLOG" }

const shared = (
  key: string,
  label: string,
  rest: Partial<CatalogEntry>,
  resolve: FakeVariable["resolve"],
): FakeVariable => ({
  entry: {
    reference: `shared.${key}`,
    label,
    group: "shared",
    valueType: { type: "string" },
    scope: "global",
    availability: "available",
    ...rest,
  },
  resolve,
})

export function defaultVariables(): FakeVariable[] {
  return [
    shared("paths.root", "Root folder", { description: "Where the checkouts live" }, () => ({
      type: "string",
      value: "/src",
    })),
    shared(
      "projects.favorite",
      "Favorite project",
      { valueType: { type: "projectId" }, scope: "profile", group: "profile", module: "projects" },
      () => ({ type: "projectId", value: "project-2" }),
    ),
    shared(
      "limits.retries",
      "Retries",
      { valueType: { type: "integer" }, description: "How often to try again" },
      () => ({ type: "integer", value: 3 }),
    ),
    shared("deploy.token", "Deploy token", { sensitive: true }, () => ({
      type: "string",
      value: SECRET,
    })),
    shared("loop.self", "Self reference", {}, () => ({
      type: "string",
      value: "{{ shared.loop.self }}",
    })),
    shared("team.channel", "Team channel", { scope: "project", group: "project" }, () => ({
      status: "missing",
      reason: "noValue",
    })),
    shared("admin.key", "Admin key", {}, () => ({ status: "permissionDenied" })),
    {
      entry: {
        reference: "system.project.current.key",
        label: "Project key",
        description: "Key of the project in the context",
        group: "system",
        valueType: { type: "string" },
        dynamic: true,
        availability: "available",
      },
      resolve: (context) => {
        if (context.projectSource === "execution" && !context.execution) {
          return { status: "unavailableUntilRuntime" }
        }
        const key = context.project ? PROJECT_KEYS[context.project] : undefined
        return key ? { type: "string", value: key } : { status: "missing", reason: "noProject" }
      },
    },
    {
      entry: {
        reference: "workflow.execution.id",
        label: "Run id",
        group: "workflow",
        valueType: { type: "string" },
        dynamic: true,
        availability: "runtimeOnly",
      },
      resolve: () => ({ status: "unavailableUntilRuntime" }),
    },
  ]
}

type ApiLike = {
  listVariables: Mock<
    (context: ResolutionContext, filter: CatalogFilter) => Promise<CatalogEntry[]>
  >
  previewReferences: Mock<
    (context: ResolutionContext, items: PreviewItem[]) => Promise<ReferencePreview[]>
  >
}

export function fakeVariableAgent(variables: FakeVariable[] = defaultVariables()) {
  const agent = {
    variables,
    /** Sets the value a variable resolves to from now on. */
    set(reference: string, value: VariableValue | ReferenceStatus) {
      const variable = variables.find((v) => v.entry.reference === reference)
      if (!variable) throw new Error(`no variable ${reference}`)
      variable.resolve = () => value
    },
    /** The same catalog in every context: entries are metadata only. */
    list(filter: CatalogFilter): CatalogEntry[] {
      return variables.map((v) => v.entry).filter((entry) => matchesFilter(entry, filter))
    },
    preview(context: ResolutionContext, items: PreviewItem[]): ReferencePreview[] {
      return items.map((item) => {
        const reference = item.reference.trim()
        const variable = variables.find((v) => v.entry.reference === reference)
        if (!variable) {
          return { reference, status: { status: "missing", reason: "unknownVariable" } }
        }
        const result = variable.resolve(context)
        if ("status" in result) return { reference, status: result }
        const found = variable.entry.valueType
        if (item.expected && !assignableTo(found, item.expected)) {
          return { reference, status: { status: "typeMismatch", found } }
        }
        const source = { type: "scope", scope: { kind: "global" } } as const
        return {
          reference,
          status: variable.entry.sensitive
            ? { status: "redacted", valueType: found, source }
            : { status: "resolved", value: result, source },
        }
      })
    },
    install(api: ApiLike) {
      api.listVariables.mockImplementation((_context, filter) =>
        Promise.resolve(agent.list(filter)),
      )
      api.previewReferences.mockImplementation((context, items) =>
        Promise.resolve(agent.preview(context, items)),
      )
      return agent
    },
  }
  return agent
}
