import { type TranslationKey, i18n } from "@/i18n"
import { type LiveExecution, executionDuration } from "@/lib/executions"
import type { ExecutionState, ExecutionSummary, ProjectOutcome } from "@/lib/types"

import type { WorkflowGraph } from "@/lib/types"

import { type Lookup, actionOf, describeNode, formatMs } from "./describe"
import type { NodeRun } from "./node-view"
import type { DeclaredOutput } from "./run-debug"

/**
 * Title and declared outputs of a node, for the run details. Nodes that no
 * longer exist in the graph keep their id as title.
 */
export function runNodeInfo(graph: WorkflowGraph, lookup: Lookup) {
  return (id: string): { title: string; declared?: DeclaredOutput[] } => {
    const node = graph.nodes.find((n) => n.id === id)
    if (!node) return { title: id }
    const { title } = describeNode(node, lookup)
    if (node.type !== "action") return { title }
    const { type } = actionOf(node, lookup)
    return {
      title,
      declared: (type?.node?.outputs ?? []).map(({ key, label, sensitive }) => ({
        key,
        label,
        sensitive,
      })),
    }
  }
}

export { triggerText } from "./trigger-model"

/** The run state of every node of `run`, for the canvas. */
export function nodeRuns(run: LiveExecution | undefined): Record<string, NodeRun> | undefined {
  if (!run) return undefined
  const result: Record<string, NodeRun> = {}
  for (const report of run.nodes) result[report.node] = { state: "done", report }
  for (const node of run.runningNodes ?? []) {
    result[node] = {
      state: "running",
      attempt: run.attempts?.[node],
      progress: run.progress?.[node],
    }
  }
  return result
}

const STATE_LABEL: Record<ExecutionState, TranslationKey> = {
  running: "workflows:run.state.running",
  succeeded: "workflows:run.state.succeeded",
  failed: "workflows:run.state.failed",
  cancelled: "workflows:run.state.cancelled",
  interrupted: "workflows:run.state.interrupted",
}

/** One line about a run: state, progress or duration. */
export function runSummary(run: ExecutionSummary, now: number) {
  if (run.state === "running") {
    return i18n.t("workflows:run.summary.running", {
      completed: run.completedNodes,
      count: run.totalNodes,
    })
  }
  return i18n.t("workflows:run.summary.finished", {
    state: i18n.t(STATE_LABEL[run.state]),
    duration: formatMs(executionDuration(run, now)),
  })
}

/**
 * What a run did to DeckPilot's active project, in words; `null` when it
 * did nothing worth saying. `key` names a project by its id.
 */
export function projectOutcomeText(
  outcome: ProjectOutcome | undefined,
  key: (id: string | null | undefined) => string,
): string | null {
  switch (outcome?.type) {
    case "applied":
      return i18n.t("workflows:run.project.applied", { project: key(outcome.project) })
    case "keptInRun":
      return i18n.t("workflows:run.project.keptInRun", { project: key(outcome.project) })
    case "notApplied":
      return outcome.project
        ? i18n.t("workflows:run.project.notAppliedTo", {
            project: key(outcome.project),
            reason: outcome.reason,
          })
        : i18n.t("workflows:run.project.notApplied", { reason: outcome.reason })
    default:
      return null
  }
}

/**
 * Why a workflow cannot be run from the app right now, with what to do;
 * `null` when it can. Runs happen in the agent, for the active profile.
 */
export function runBlockedReason(state: {
  isNew: boolean
  dirty: boolean
  agentRunning: boolean
  activeProfileId: string | null
  profileId: string
  profileName: string
}): string | null {
  if (state.isNew) return i18n.t("workflows:run.blocked.new")
  if (state.dirty) return i18n.t("workflows:run.blocked.dirty")
  if (!state.agentRunning) return i18n.t("workflows:run.blocked.agent")
  if (state.activeProfileId !== state.profileId) {
    return i18n.t("workflows:run.blocked.profile", { profile: state.profileName })
  }
  return null
}
