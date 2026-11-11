import { useState } from "react"
import { toast } from "sonner"

import { useExecutions } from "@/hooks/use-executions"
import { useSnapshot } from "@/hooks/use-status"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { ExecutionStore } from "@/lib/executions"

import { type BuilderProps, WorkflowBuilder } from "./builder"

const NO_EXECUTIONS = new ExecutionStore()

type Props = Omit<BuilderProps, "canRun" | "running" | "onRun" | "onCancelRun">

/** The builder with running the workflow in the agent. */
export function WorkflowRun(props: Props) {
  const { agent, profiles } = useAppContext()
  const executions = useExecutions()
  const runs = useSnapshot(executions ?? NO_EXECUTIONS)
  const latest = ExecutionStore.ofProfile(runs, props.profileId).find(
    (r) => r.workflow === props.initial.id,
  )
  const running = latest?.state === "running"
  const [starting, setStarting] = useState(false)
  const canRun =
    !props.isNew &&
    !starting &&
    Boolean(agent?.running) &&
    profiles.activeProfileId === props.profileId

  return (
    <WorkflowBuilder
      {...props}
      canRun={canRun}
      running={running || starting}
      onRun={() => {
        setStarting(true)
        api
          .startWorkflow(props.profileId, props.initial.id)
          .then((execution) => executions?.apply({ type: "executionStarted", execution }))
          .catch((e: unknown) => toast.error(errorMessage(e)))
          .finally(() => {
            setStarting(false)
          })
      }}
      onCancelRun={
        latest && running
          ? () => {
              api.cancelExecution(latest.id).catch((e: unknown) => toast.error(errorMessage(e)))
            }
          : undefined
      }
    />
  )
}
