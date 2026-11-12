import { type ReactNode, useMemo } from "react"

import type { CatalogEntry, ResolutionContext } from "@/lib/types"
import type { ContextNames } from "@/lib/variables"

import { VariableScopeContext, useVariableScope } from "./scope"

/**
 * Narrows the scope for the editors inside, e.g. to a workflow whose runs
 * have their own project (`projectSource: "execution"`), and adds the
 * outputs of the graph being edited.
 */
export function VariableScopeProvider({
  context,
  names,
  stepOutputs,
  disabled = false,
  children,
}: {
  context?: Partial<ResolutionContext>
  names?: ContextNames
  stepOutputs?: CatalogEntry[]
  /** No references here: nothing resolves them (e.g. module settings). */
  disabled?: boolean
  children: ReactNode
}) {
  const parent = useVariableScope()
  // Callers build these objects on every render; only their content counts.
  const key = JSON.stringify([context, names, stepOutputs])
  const value = useMemo(() => {
    if (!parent) return null
    const [c, n, s] = JSON.parse(key) as [
      Partial<ResolutionContext> | undefined,
      ContextNames | undefined,
      CatalogEntry[] | undefined,
    ]
    return {
      ...parent,
      live: parent.live && !disabled,
      context: { ...parent.context, ...c },
      names: { ...parent.names, ...n },
      stepOutputs: s ?? parent.stepOutputs,
    }
  }, [parent, key, disabled])
  if (!value) return children
  return <VariableScopeContext value={value}>{children}</VariableScopeContext>
}
