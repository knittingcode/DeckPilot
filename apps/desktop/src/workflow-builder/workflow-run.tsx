import { useEffect, useState } from "react"
import { toast } from "sonner"

import { useExecutions } from "@/hooks/use-executions"
import { useSnapshot } from "@/hooks/use-status"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { ExecutionStore } from "@/lib/executions"

import { type BuilderProps, WorkflowBuilder } from "./builder"
import { RunDebugger } from "./run-debugger"
import { nodeRuns, runBlockedReason, runNodeInfo, runSummary } from "./run-model"
import { RunHistory, StateIcon } from "./run-panel"

const NO_EXECUTIONS = new ExecutionStore()
const HISTORY = 20

type Props = Omit<
  BuilderProps,
  "canRun" | "running" | "onRun" | "onCancelRun" | "run" | "workflowPanel"
>

/** The current time, ticking while `active`. */
function useNow(active: boolean) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    const timer = setInterval(() => {
      setNow(Date.now())
    }, 500)
    return () => {
      clearInterval(timer)
    }
  }, [active])
  return now
}

/**
 * The builder with running the workflow in the agent, the run shown on the
 * canvas and the run history. Runs come from the agent, so a run started from
 * the deck or before the window opened shows the same way.
 */
export function WorkflowRun(props: Props) {
  const { agent, profiles } = useAppContext()
  const executions = useExecutions()
  const all = useSnapshot(executions ?? NO_EXECUTIONS)
  const workflowId = props.initial.id
  const runs = ExecutionStore.ofProfile(all, props.profileId)
    .filter((r) => r.workflow === workflowId)
    .slice(0, HISTORY)
  const latest = runs[0]
  const running = latest?.state === "running"
  const [starting, setStarting] = useState(false)
  // `undefined`: follow the newest run; `null`: show none.
  const [chosen, setChosen] = useState<string | null | undefined>(undefined)
  const shownId = chosen === undefined ? (latest?.id ?? null) : chosen
  const shown = runs.find((r) => r.id === shownId)
  const now = useNow(running)
  const connection = agent?.running ? (agent.status?.startedAt ?? 0) : null
  const [historyProblem, setHistoryProblem] = useState<string | null>(null)

  // The history of this workflow, also after the agent reconnects.
  useEffect(() => {
    if (props.isNew || !executions || connection === null) return
    let current = true
    api.listExecutions(workflowId, HISTORY).then(
      (list) => {
        if (!current) return
        setHistoryProblem(null)
        executions.merge(list)
      },
      (e: unknown) => {
        if (current) setHistoryProblem(errorMessage(e))
      },
    )
    return () => {
      current = false
    }
  }, [executions, workflowId, props.isNew, connection])

  // Node reports of a finished run that was only listed.
  const missing = shown && shown.state !== "running" && shown.nodes.length === 0 ? shown.id : null
  useEffect(() => {
    if (!missing || !executions) return
    api.getExecution(missing).then(
      (record) => {
        executions.load(record)
      },
      () => undefined,
    )
  }, [missing, executions])

  const info = runNodeInfo(props.graph, {
    actions: props.view.profile.actions,
    types: props.types,
    workflows: props.view.profile.workflows,
    projects: props.view.profile.projects,
  })

  const cancel = (id: string) => {
    api
      .cancelExecution(id)
      .catch((e: unknown) =>
        toast.error("The run was not cancelled", { description: errorMessage(e) }),
      )
  }
  const runBlocked = runBlockedReason({
    isNew: props.isNew,
    dirty: false, // the builder knows about unsaved changes
    agentRunning: Boolean(agent?.running),
    activeProfileId: profiles.activeProfileId,
    profileId: props.profileId,
    profileName: props.view.profile.name,
  })
  const canRun = runBlocked === null && !starting

  return (
    <WorkflowBuilder
      {...props}
      toolbar={
        <>
          {props.toolbar}
          {shown && (
            <span
              className="flex min-w-0 items-center gap-1.5 truncate text-xs text-muted-foreground"
              aria-live="polite"
            >
              <StateIcon state={shown.state} className="size-3.5 shrink-0" />
              {runSummary(shown, now)}
            </span>
          )}
        </>
      }
      run={nodeRuns(shown)}
      workflowPanel={
        props.isNew ? undefined : (
          <>
            <RunHistory
              runs={runs}
              shown={shownId}
              now={now}
              problem={historyProblem}
              onShow={setChosen}
              onCancel={cancel}
              projectKey={(id) =>
                props.view.profile.projects.find((p) => p.id === id)?.key ?? "no project"
              }
              onClear={() => {
                api
                  .clearExecutionHistory(workflowId)
                  .then(() => {
                    executions?.forget((r) => r.workflow === workflowId)
                    setChosen(undefined)
                  })
                  .catch((e: unknown) =>
                    toast.error("The run history was not cleared", {
                      description: errorMessage(e),
                    }),
                  )
              }}
            />
            {shown && <RunDebugger key={shown.id} run={shown} now={now} info={info} />}
          </>
        )
      }
      canRun={canRun}
      runBlocked={runBlocked}
      running={running || starting}
      onRun={() => {
        setStarting(true)
        setChosen(undefined)
        api
          .startWorkflow(props.profileId, workflowId)
          .then((execution) => executions?.apply({ type: "executionStarted", execution }))
          .catch((e: unknown) =>
            toast.error(`“${props.initial.name}” did not start`, { description: errorMessage(e) }),
          )
          .finally(() => {
            setStarting(false)
          })
      }}
      onCancelRun={
        latest && running
          ? () => {
              cancel(latest.id)
            }
          : undefined
      }
    />
  )
}
