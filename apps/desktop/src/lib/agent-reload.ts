// What a save says about the agent's reload (`commands/reload.rs`).

import { toast } from "sonner"

import { sentence } from "./failure"
import type { AgentReload } from "./types"

/** Whether the agent has read the saved change. */
export function appliedByAgent(reload: AgentReload | undefined): boolean {
  return reload?.state === "applied"
}

/**
 * Why the agent may not use a saved change yet, or `undefined` when there
 * is nothing to say: it applied the change, or it is not running and reads
 * the change when it starts.
 */
export function reloadProblem(reload: AgentReload | undefined): string | undefined {
  switch (reload?.state) {
    case "timedOut":
      return "The agent did not confirm that it applied the change. It applies it when it can, or restart it in Settings › Background Agent."
    case "failed":
      return `The agent did not apply the change: ${sentence(reload.message)}`
    default:
      return undefined
  }
}

/**
 * The toast after a save: `title` as a success, or as a warning that says
 * why the agent may not use the change yet. The change is saved either way.
 */
export function toastSaved(title: string, reload: AgentReload | undefined): void {
  const problem = reloadProblem(reload)
  if (problem) toast.warning(title, { description: problem })
  else toast.success(title)
}
