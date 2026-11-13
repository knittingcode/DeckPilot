import { createContext, useContext, useMemo } from "react"

import { AppContext, type AppContextValue } from "@/lib/app-context"
import type { CatalogEntry, ResolutionContext } from "@/lib/types"
import type { ContextNames } from "@/lib/variables"

/**
 * Where the fields below an editor are resolved: the explicit context
 * previews are asked for, the names shown for it, and the outputs of the
 * graph being edited.
 */
export interface VariableScope {
  context: ResolutionContext
  names: ContextNames
  /**
   * The agent can resolve references (`variables` capability). Without it the
   * picker is hidden and no reference can be saved (an older agent could
   * not run it).
   */
  live: boolean
  /** Outputs of earlier steps, listed from the graph (metadata only). */
  stepOutputs: CatalogEntry[]
}

export const VariableScopeContext = createContext<VariableScope | null>(null)

/**
 * The scope of the surrounding editor; outside any, the profile being
 * edited with its active project, as when a key is pressed. `null` without
 * a profile.
 */
export function useVariableScope(): VariableScope | null {
  const scope = useContext(VariableScopeContext)
  const app = useContext(AppContext)
  const fallback = useMemo(() => (scope ? null : applicationScope(app)), [scope, app])
  return scope ?? fallback
}

/** Whether the connected agent can list and preview variables. */
export function agentResolvesVariables(app: AppContextValue | null): boolean {
  return Boolean(app?.agent?.running && app.agent.capabilities?.includes("variables"))
}

function applicationScope(app: AppContextValue | null): VariableScope | null {
  const profile = app?.editingProfileId
  if (!app || !profile) return null
  return {
    context: { profile, projectSource: "application", phase: "preview" },
    names: { profile: app.profiles.profiles.find((p) => p.id === profile)?.name },
    live: agentResolvesVariables(app),
    stepOutputs: [],
  }
}
