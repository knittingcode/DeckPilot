import { createContext, useContext } from "react"

import type { ExecutionStore } from "@/lib/executions"

export const ExecutionsContext = createContext<ExecutionStore | null>(null)

/** Workflow runs reported by the agent; `null` outside the main window. */
export function useExecutions(): ExecutionStore | null {
  return useContext(ExecutionsContext)
}
