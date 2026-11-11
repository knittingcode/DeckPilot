import { listen } from "@tauri-apps/api/event"
import { useEffect } from "react"

import { useExecutions } from "@/hooks/use-executions"
import { useSnapshot, useStatus } from "@/hooks/use-status"
import { EXECUTION_EVENT, api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { ExecutionStore, executionProgress } from "@/lib/executions"
import type { AgentExecutionEvent } from "@/lib/types"

/**
 * Keeps the execution store in line with the agent and shows the runs of
 * the active profile as background operations in the status bar.
 *
 * Runs belong to the agent and continue while no window is open. When the
 * window opens, or the agent connects again, the store is filled from the
 * agent's list, so runs started from the deck or before a restart of the
 * app appear with their real progress.
 */
export function ExecutionSync() {
  const store = useExecutions()
  const status = useStatus()
  const { agent, profiles } = useAppContext()
  // Changes when the agent connects or restarts (a new agent has a new
  // start time), which is when the list is read again.
  const connection = agent?.running ? (agent.status?.startedAt ?? 0) : null
  const profileId = profiles.activeProfileId

  // (Re)connect: the agent's list replaces what this window knew.
  useEffect(() => {
    if (!store || connection === null) return
    let current = true
    api.listExecutions(undefined, 50).then(
      (executions) => {
        if (!current) return
        store.reset(executions)
        for (const run of executions.filter((e) => e.state === "running")) {
          api.getExecution(run.id).then(
            (record) => {
              if (current) store.load(record)
            },
            () => undefined,
          )
        }
      },
      () => undefined,
    )
    return () => {
      current = false
    }
  }, [store, connection, profileId])

  useEffect(() => {
    if (!store) return
    const stop = listen<AgentExecutionEvent>(EXECUTION_EVENT, ({ payload }) => {
      if (!store.apply(payload) && payload.type === "executionProgress") {
        // A run this window has not seen start (e.g. while reconnecting).
        api.getExecution(payload.execution).then(
          (record) => {
            store.load(record)
          },
          () => undefined,
        )
      }
      if (payload.type === "executionFinished") {
        const run = payload.execution
        // The final, complete node reports.
        api.getExecution(run.id).then(
          (record) => {
            store.load(record)
          },
          () => undefined,
        )
        if (run.state === "failed" || run.state === "interrupted") {
          status?.notifications.notify({
            key: `workflow.${run.workflow}.failed`,
            severity: "error",
            title: `Workflow “${run.workflowName}” ${run.state === "failed" ? "failed" : "was interrupted"}`,
            message: run.message || "Open the workflow to see which step failed.",
          })
        }
      }
    })
    return () => {
      void stop.then((unlisten) => {
        unlisten()
      })
    }
  }, [store, status])

  const records = useSnapshot(store ?? EMPTY)
  useEffect(() => {
    if (!status) return
    for (const run of ExecutionStore.ofProfile(records, profileId)) {
      const id = `execution:${run.id}`
      const existing = status.operations.get(id)
      if (run.state === "running") {
        status.operations.start({
          id,
          kind: "workflow",
          title: `Workflow “${run.workflowName}”`,
          startedAt: run.startedAt,
          progress: executionProgress(run),
          completed: run.completedNodes,
          total: run.totalNodes,
          cancel: () => {
            void api.cancelExecution(run.id).catch(() => undefined)
          },
        })
      } else if (existing?.status === "running") {
        status.operations.finish(
          id,
          run.state === "cancelled"
            ? { cancelled: true }
            : run.state === "succeeded"
              ? {}
              : {
                  error:
                    run.message ||
                    `The workflow ${run.state === "failed" ? "failed" : "was interrupted"}`,
                },
        )
      }
    }
  }, [records, status, profileId])

  return null
}

const EMPTY = new ExecutionStore()
