import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import {
  ArrowDown,
  ArrowUp,
  Copy,
  GripVertical,
  Play,
  Plus,
  Save,
  Timer,
  Trash2,
  X,
} from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { ErrorNotice } from "@/components/page"
import { ParameterField } from "@/components/parameter-field"
import { UsageList } from "@/components/usage-list"
import { Button } from "@/components/ui/button"
import { Panel } from "@/components/settings-layout"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useExecutions } from "@/hooks/use-executions"
import { api, errorMessage } from "@/lib/api"
import { toastSaved } from "@/lib/agent-reload"
import { toastFailure } from "@/lib/failure"
import { useSnapshot } from "@/hooks/use-status"
import { EXECUTION_STATE_LABEL, ExecutionStore } from "@/lib/executions"
import { useTitlebar } from "@/hooks/use-titlebar"
import { useActionModules, useAppContext } from "@/lib/app-context"
import { pluginActionUnavailable } from "@/lib/plugin-actions"
import { ActionIcon } from "@/components/action-icon"
import { initialValue } from "@/lib/parameters"
import type {
  ActionInstance,
  ActionType,
  ProfileView,
  StepCondition,
  Workflow,
  WorkflowStep,
} from "@/lib/types"
import { existingDefault, newAction } from "@/layout-editor/catalog"
import {
  DELAY_TYPE,
  MAX_DELAY_MS,
  MIN_DELAY_MS,
  delayAction,
  delayMilliseconds,
  delayStep,
  withDelay,
} from "@/lib/multi-action"

const NO_EXECUTIONS = new ExecutionStore()
import { cn } from "@/lib/utils"
import {
  conditionAllowed,
  moveStep,
  newStep,
  removeStep,
  setOverride,
  updateStep,
  validateWorkflow,
} from "@/lib/workflow-model"
import { runBlockedReason } from "@/workflow-builder/run-model"

const CONDITIONS = [
  { value: "always", label: "Always" },
  { value: "previousSucceeded", label: "If the previous step succeeded" },
  { value: "previousFailed", label: "If the previous step failed" },
]

function conditionValue(condition: StepCondition | undefined): string {
  if (!condition) return "always"
  return condition.type === "activeProject" ? `project:${condition.project}` : condition.type
}

function parseCondition(value: string): StepCondition {
  if (value.startsWith("project:")) return { type: "activeProject", project: value.slice(8) }
  if (value === "previousSucceeded" || value === "previousFailed") return { type: value }
  return { type: "always" }
}

/**
 * The step list of a workflow without a graph. `embedded`: opened from a
 * key in the layout editor (a Multi Action), with its own Run button
 * instead of the titlebar's, and without duplicate, delete and the usage
 * list, which belong to the Workflows page.
 */
export function StepEditor({
  view,
  types,
  profileId,
  initial,
  isNew,
  onChanged,
  embedded = false,
}: {
  view: ProfileView
  types: ActionType[]
  profileId: string
  initial: Workflow
  isNew: boolean
  onChanged: (id?: string) => void
  embedded?: boolean
}) {
  const { profiles, agent, refresh } = useAppContext()
  const [workflow, setWorkflow] = useState<Workflow>(initial)
  const [addAction, setAddAction] = useState("")
  const [errors, setErrors] = useState<string[]>([])
  // The delay action "Add delay" created, until the reloaded profile has it.
  const [created, setCreated] = useState<ActionInstance[]>([])
  const executions = useExecutions()
  const runs = useSnapshot(executions ?? NO_EXECUTIONS)
  // The newest run of this workflow, wherever it was started.
  const lastRun = ExecutionStore.ofProfile(runs, profileId).find((r) => r.workflow === workflow.id)
  const running = lastRun?.state === "running"
  const [starting, setStarting] = useState(false)
  const actions = [
    ...view.profile.actions,
    ...created.filter((c) => !view.profile.actions.some((a) => a.id === c.id)),
  ]
  const delayType = types.find((t) => t.id === DELAY_TYPE)
  const modules = useActionModules()
  const actionIds = new Set(actions.map((a) => a.id))
  const dirty = JSON.stringify(workflow) !== JSON.stringify(initial)
  const runBlocked = runBlockedReason({
    isNew,
    dirty,
    agentRunning: Boolean(agent?.running),
    activeProfileId: profiles.activeProfileId,
    profileId,
    profileName: view.profile.name,
  })
  const canRun = runBlocked === null
  const usages = view.workflowUsages[workflow.id] ?? []
  // Runs in the agent in the background; progress and the result arrive as
  // execution events (status bar, below).
  const run = () => {
    setStarting(true)
    api
      .startWorkflow(profileId, workflow.id)
      .then((execution) => executions?.apply({ type: "executionStarted", execution }))
      .catch((e: unknown) => {
        toast.error(errorMessage(e))
      })
      .finally(() => {
        setStarting(false)
      })
  }
  // Runs the saved workflow of the editor; shown in the titlebar.
  useTitlebar({
    actions:
      isNew || embedded
        ? []
        : [
            {
              id: "workflows.run",
              label: "Run workflow",
              icon: Play,
              placement: "primary",
              disabled: !canRun || running || starting,
              busy: running || starting,
              hint: runBlocked ?? `Run “${initial.name}”`,
              onSelect: run,
            },
          ],
  })

  /** Adds a delay row, with the profile's default delay action or a new one. */
  const addDelay = async () => {
    if (!delayType) return
    try {
      let action = existingDefault(actions, delayType)
      if (!action) {
        action = await api.createAction(profileId, newAction(actions, delayType))
        const made = action
        setCreated((current) => [...current, made])
        refresh()
      }
      const id = action.id
      setWorkflow((w) => ({ ...w, steps: [...w.steps, delayStep(id)] }))
    } catch (e) {
      toastFailure("The delay was not added", e)
    }
  }
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )
  const ids = workflow.steps.map((_, index) => `step-${index}`)

  const onDragEnd = (event: DragEndEvent) => {
    const from = ids.indexOf(String(event.active.id))
    const to = event.over ? ids.indexOf(String(event.over.id)) : -1
    if (from >= 0 && to >= 0) setWorkflow((w) => moveStep(w, from, to))
  }

  const save = async () => {
    const local = validateWorkflow(workflow, actionIds)
    setErrors(local)
    if (local.length > 0) return
    try {
      const saved = await api.saveWorkflow(profileId, workflow)
      toastSaved("Workflow saved", saved.agentReload)
      onChanged(saved.id)
    } catch (e) {
      setErrors(errorMessage(e).split("\n"))
    }
  }

  return (
    <Panel
      title={isNew ? "New workflow" : initial.name}
      description="Steps run in order in the agent. Each step uses an action from this profile."
    >
      {errors.length > 0 && <ErrorNotice error={errors.join(" ")} />}
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="grid gap-2">
          <Label htmlFor="workflow-name">Name</Label>
          <Input
            id="workflow-name"
            value={workflow.name}
            placeholder="Start Work"
            onChange={(e) => {
              setWorkflow({ ...workflow, name: e.target.value })
            }}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="workflow-description">Description</Label>
          <Input
            id="workflow-description"
            value={workflow.description ?? ""}
            placeholder="Optional"
            onChange={(e) => {
              setWorkflow({ ...workflow, description: e.target.value })
            }}
          />
        </div>
      </div>

      <div className="grid gap-2">
        <Label id="workflow-steps-label">Steps</Label>
        {workflow.steps.length === 0 && (
          <p className="text-sm text-muted-foreground">Add actions below.</p>
        )}
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
          <SortableContext items={ids} strategy={verticalListSortingStrategy}>
            <ol className="grid gap-2" aria-labelledby="workflow-steps-label">
              {workflow.steps.map((step, index) => (
                <SortableStep
                  key={ids[index]}
                  id={ids[index] ?? ""}
                  index={index}
                  count={workflow.steps.length}
                  step={step}
                  view={view}
                  actions={actions}
                  types={types}
                  onChange={(change) => {
                    setWorkflow(updateStep(workflow, index, change))
                  }}
                  onMove={(to) => {
                    setWorkflow(moveStep(workflow, index, to))
                  }}
                  onRemove={() => {
                    setWorkflow(removeStep(workflow, index))
                  }}
                />
              ))}
            </ol>
          </SortableContext>
        </DndContext>
        <div className="flex flex-wrap items-center gap-2">
          <Select value={addAction} onValueChange={setAddAction}>
            <SelectTrigger className="w-72" aria-label="Action to add">
              <SelectValue placeholder="Choose an action" />
            </SelectTrigger>
            <SelectContent>
              {actions.map((action) => {
                const why = pluginActionUnavailable(modules, action.actionType)
                const usable = !why && types.some((t) => t.id === action.actionType)
                return (
                  <SelectItem
                    key={action.id}
                    value={action.id}
                    disabled={!usable}
                    title={why ?? undefined}
                  >
                    {action.name}
                    {!usable && (why ? ` (unavailable: ${why})` : " (unavailable)")}
                  </SelectItem>
                )
              })}
            </SelectContent>
          </Select>
          <Button
            variant="outline"
            disabled={!addAction}
            onClick={() => {
              setWorkflow({ ...workflow, steps: [...workflow.steps, newStep(addAction)] })
              setAddAction("")
            }}
          >
            <Plus /> Add step
          </Button>
          {delayType && (
            <Button variant="outline" onClick={() => void addDelay()}>
              <Timer /> Add delay
            </Button>
          )}
        </div>
      </div>

      <div className="flex flex-wrap gap-2">
        <Button onClick={() => void save()} disabled={!dirty && !isNew}>
          <Save /> Save
        </Button>
        {embedded && !isNew && (
          <Button
            variant="outline"
            disabled={!canRun || running || starting}
            title={runBlocked ?? `Run “${initial.name}”`}
            onClick={run}
          >
            <Play /> Run
          </Button>
        )}
        {!isNew && !embedded && (
          <>
            <Button
              variant="outline"
              onClick={() => {
                api
                  .duplicateWorkflow(profileId, workflow.id, `${initial.name} (copy)`)
                  .then((copy) => {
                    toast.success("Workflow duplicated")
                    onChanged(copy.id)
                  })
                  .catch((e: unknown) => {
                    toastFailure("The workflow was not duplicated", e)
                  })
              }}
            >
              <Copy /> Duplicate
            </Button>
            <ConfirmDialog
              trigger={
                <Button variant="ghost" disabled={usages.length > 0}>
                  <Trash2 /> Delete
                </Button>
              }
              title={`Delete “${initial.name}”?`}
              description="The workflow is removed from this profile."
              onConfirm={() => {
                api
                  .deleteWorkflow(profileId, workflow.id)
                  .then(() => {
                    toast.success("Workflow deleted")
                    onChanged()
                  })
                  .catch((e: unknown) => {
                    toastFailure("The workflow was not deleted", e)
                  })
              }}
            />
          </>
        )}
      </div>

      {lastRun && (
        <div className="grid gap-1 rounded-md bg-muted p-3 text-xs" role="status">
          <p className="font-medium">
            Last run: {EXECUTION_STATE_LABEL[lastRun.state]}
            {lastRun.state === "running" && ` (${lastRun.completedNodes}/${lastRun.totalNodes})`}
            {lastRun.message ? ` — ${lastRun.message}` : ""}
          </p>
          <ol className="grid gap-0.5 font-mono">
            {lastRun.nodes.map((node) => (
              <li key={node.node}>
                {actions.find((a) => a.id === node.action)?.name ?? node.node} — {node.status}
                {node.message ? `: ${node.message}` : ""}
              </li>
            ))}
          </ol>
        </div>
      )}
      {!isNew && !embedded && usages.length > 0 && (
        <div className="grid gap-1">
          <h3 className="text-sm font-medium">Used in</h3>
          <UsageList usages={usages} />
        </div>
      )}
    </Panel>
  )
}

function SortableStep({
  id,
  index,
  count,
  step,
  view,
  actions,
  types,
  onChange,
  onMove,
  onRemove,
}: {
  id: string
  index: number
  count: number
  step: WorkflowStep
  view: ProfileView
  /** The profile's actions, with those created since it was read. */
  actions: ActionInstance[]
  types: ActionType[]
  onChange: (change: Partial<WorkflowStep>) => void
  onMove: (to: number) => void
  onRemove: () => void
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id,
  })
  const [showParameters, setShowParameters] = useState(Object.keys(step.overrides ?? {}).length > 0)
  const action = actions.find((a) => a.id === step.action)
  const type = types.find((t) => t.id === action?.actionType)
  const modules = useActionModules()
  const unavailable = action ? pluginActionUnavailable(modules, action.actionType) : null
  // A delay row: only its duration is edited; the step keeps whatever else
  // it has.
  const delay = delayAction(step, actions)

  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn(
        "grid gap-3 rounded-lg border bg-card p-3",
        isDragging && "opacity-70 shadow-lg",
      )}
    >
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          className="cursor-grab rounded p-1 text-muted-foreground hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-label={`Drag step ${index + 1}`}
          {...attributes}
          {...listeners}
        >
          <GripVertical className="size-4" />
        </button>
        <span className="w-5 text-sm text-muted-foreground tabular-nums">{index + 1}.</span>
        {delay ? (
          <DelayField
            index={index}
            ms={delayMilliseconds(step, delay)}
            onChange={(ms) => {
              onChange({ overrides: withDelay(step, ms).overrides })
            }}
          />
        ) : (
          <>
            <ActionIcon name={action?.icon} className="size-4" />
            <Select
              value={step.action}
              onValueChange={(value) => {
                onChange({ action: value, overrides: {} })
              }}
            >
              <SelectTrigger className="w-64" aria-label={`Action of step ${index + 1}`}>
                <SelectValue placeholder="Missing action" />
              </SelectTrigger>
              <SelectContent>
                {actions.map((a) => (
                  <SelectItem key={a.id} value={a.id}>
                    {a.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </>
        )}
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="icon"
          aria-label="Move up"
          disabled={index === 0}
          onClick={() => {
            onMove(index - 1)
          }}
        >
          <ArrowUp />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Move down"
          disabled={index === count - 1}
          onClick={() => {
            onMove(index + 1)
          }}
        >
          <ArrowDown />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label={`Remove step ${index + 1}`}
          onClick={onRemove}
        >
          <X />
        </Button>
      </div>
      {unavailable && (
        <p className="text-xs text-destructive" role="note">
          Unavailable: {unavailable}. The step stays in the workflow and runs again once the plugin
          can.
        </p>
      )}
      {!delay && (
        <div className="grid gap-2 sm:grid-cols-2">
          <Select
            value={conditionValue(step.condition)}
            onValueChange={(value) => {
              onChange({ condition: parseCondition(value) })
            }}
          >
            <SelectTrigger aria-label={`Condition of step ${index + 1}`} className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {CONDITIONS.filter((c) => conditionAllowed(parseCondition(c.value), index)).map(
                (c) => (
                  <SelectItem key={c.value} value={c.value}>
                    {c.label}
                  </SelectItem>
                ),
              )}
              {view.profile.projects.map((project) => (
                <SelectItem key={project.id} value={`project:${project.id}`}>
                  While {project.key} is active
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={step.onFailure}
            onValueChange={(value) => {
              onChange({ onFailure: value as WorkflowStep["onFailure"] })
            }}
          >
            <SelectTrigger aria-label={`On failure of step ${index + 1}`} className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="stop">On failure: stop the workflow</SelectItem>
              <SelectItem value="continue">On failure: continue</SelectItem>
            </SelectContent>
          </Select>
        </div>
      )}
      {!delay && type && type.parameters.length > 0 && (
        <div className="grid gap-2">
          <Button
            variant="link"
            size="sm"
            className="h-auto justify-start p-0"
            onClick={() => {
              setShowParameters(!showParameters)
            }}
          >
            {showParameters ? "Hide parameters" : "Override parameters for this step"}
          </Button>
          {showParameters &&
            action &&
            type.parameters.map((spec) => {
              const override = step.overrides?.[spec.key]
              return (
                <div key={spec.key} className="grid gap-2 rounded-md border border-dashed p-2">
                  <label className="flex items-center gap-2 text-sm">
                    <Checkbox
                      checked={override !== undefined}
                      onCheckedChange={(checked) => {
                        onChange({
                          overrides: setOverride(
                            step.overrides,
                            spec.key,
                            checked === true
                              ? (action.parameters[spec.key] ?? initialValue(spec))
                              : undefined,
                          ),
                        })
                      }}
                    />
                    Override “{spec.label}”
                  </label>
                  {override !== undefined && (
                    <ParameterField
                      spec={spec}
                      value={override}
                      projects={view.profile.projects}
                      onChange={(value) => {
                        onChange({ overrides: setOverride(step.overrides, spec.key, value) })
                      }}
                    />
                  )}
                </div>
              )
            })}
        </div>
      )}
    </li>
  )
}

/** The duration of a delay row, in milliseconds. */
function DelayField({
  index,
  ms,
  onChange,
}: {
  index: number
  ms: number
  onChange: (ms: number) => void
}) {
  // The typed text, so that the field can be cleared to type a new number;
  // the step takes it once it is a number.
  const [text, setText] = useState(String(ms))
  return (
    <label className="flex items-center gap-2 text-sm">
      <Timer className="size-4" aria-hidden />
      <span>Delay</span>
      <Input
        type="number"
        inputMode="numeric"
        className="h-8 w-28"
        min={MIN_DELAY_MS}
        max={MAX_DELAY_MS}
        step={50}
        aria-label={`Delay of step ${index + 1} in milliseconds`}
        value={text}
        onChange={(e) => {
          setText(e.target.value)
          const value = Number.parseInt(e.target.value, 10)
          if (Number.isFinite(value)) onChange(value)
        }}
        onBlur={() => {
          setText(String(ms))
        }}
      />
      <span className="text-muted-foreground">ms</span>
    </label>
  )
}
