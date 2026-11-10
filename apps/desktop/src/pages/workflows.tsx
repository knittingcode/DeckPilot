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
import { ArrowDown, ArrowUp, Copy, GripVertical, Play, Plus, Save, Trash2, X } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { ParameterField } from "@/components/parameter-field"
import { RequireProfile } from "@/components/require-profile"
import { UsageList } from "@/components/usage-list"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
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
import { useAsync } from "@/hooks/use-async"
import { useEditingProfile } from "@/hooks/use-profile"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { ActionIcon } from "@/components/action-icon"
import { initialValue } from "@/lib/parameters"
import type {
  ActionType,
  ProfileView,
  StepCondition,
  Workflow,
  WorkflowReport,
  WorkflowStep,
} from "@/lib/types"
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

export function WorkflowsPage() {
  return (
    <Page title="Workflows" description="Run several actions in sequence">
      <RequireProfile>{() => <Workflows />}</RequireProfile>
    </Page>
  )
}

function Workflows() {
  const { state, reload, profileId } = useEditingProfile()
  const types = useAsync(() => api.listActionTypes(), "types").state
  if (state.status === "loading" || types.status === "loading") return <Loading />
  if (state.status === "error") return <ErrorNotice error={state.error} />
  if (types.status === "error") return <ErrorNotice error={types.error} />
  if (!profileId) return null
  return <WorkflowList view={state.data} types={types.data} profileId={profileId} reload={reload} />
}

function emptyWorkflow(): Workflow {
  return { id: crypto.randomUUID(), name: "", description: "", steps: [] }
}

function WorkflowList({
  view,
  types,
  profileId,
  reload,
}: {
  view: ProfileView
  types: ActionType[]
  profileId: string
  reload: () => void
}) {
  const { refresh } = useAppContext()
  const [selected, setSelected] = useState<string>(view.profile.workflows[0]?.id ?? "new")
  const current = view.profile.workflows.find((w) => w.id === selected)
  const changed = (id?: string) => {
    if (id) setSelected(id)
    reload()
    refresh()
  }

  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(16rem,20rem)_1fr]">
      <Card className="h-fit">
        <CardHeader className="flex flex-row items-center gap-2">
          <CardTitle className="flex-1">Workflows</CardTitle>
          <Button
            size="sm"
            onClick={() => {
              setSelected("new")
            }}
          >
            <Plus /> New
          </Button>
        </CardHeader>
        <CardContent className="grid gap-1">
          {view.profile.workflows.length === 0 && (
            <EmptyState title="No workflows yet">Combine actions into a sequence.</EmptyState>
          )}
          {view.profile.workflows.map((workflow) => (
            <button
              key={workflow.id}
              type="button"
              className={cn(
                "flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                selected === workflow.id && "bg-accent",
              )}
              onClick={() => {
                setSelected(workflow.id)
              }}
            >
              <span className="flex-1 truncate">{workflow.name}</span>
              <Badge variant="secondary">{workflow.steps.length} steps</Badge>
            </button>
          ))}
        </CardContent>
      </Card>
      <WorkflowEditor
        key={current?.id ?? "new"}
        view={view}
        types={types}
        profileId={profileId}
        initial={current ?? emptyWorkflow()}
        isNew={!current}
        onChanged={changed}
      />
    </div>
  )
}

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

function WorkflowEditor({
  view,
  types,
  profileId,
  initial,
  isNew,
  onChanged,
}: {
  view: ProfileView
  types: ActionType[]
  profileId: string
  initial: Workflow
  isNew: boolean
  onChanged: (id?: string) => void
}) {
  const { profiles, agent } = useAppContext()
  const [workflow, setWorkflow] = useState<Workflow>(initial)
  const [addAction, setAddAction] = useState("")
  const [errors, setErrors] = useState<string[]>([])
  const [report, setReport] = useState<WorkflowReport | null>(null)
  const actions = view.profile.actions
  const actionIds = new Set(actions.map((a) => a.id))
  const dirty = JSON.stringify(workflow) !== JSON.stringify(initial)
  const canRun =
    !isNew && !dirty && Boolean(agent?.running) && profiles.activeProfileId === profileId
  const usages = view.workflowUsages[workflow.id] ?? []
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
      toast.success("Workflow saved")
      onChanged(saved.id)
    } catch (e) {
      setErrors(errorMessage(e).split("\n"))
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{isNew ? "New workflow" : initial.name}</CardTitle>
        <CardDescription>
          Steps run in order in the agent. Each step uses an action from this profile.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4">
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
          <Label>Steps</Label>
          {workflow.steps.length === 0 && (
            <p className="text-sm text-muted-foreground">Add actions below.</p>
          )}
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
            <SortableContext items={ids} strategy={verticalListSortingStrategy}>
              <ol className="grid gap-2">
                {workflow.steps.map((step, index) => (
                  <SortableStep
                    key={ids[index]}
                    id={ids[index] ?? ""}
                    index={index}
                    count={workflow.steps.length}
                    step={step}
                    view={view}
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
                {actions.map((action) => (
                  <SelectItem
                    key={action.id}
                    value={action.id}
                    disabled={!types.some((t) => t.id === action.actionType)}
                  >
                    {action.name}
                    {!types.some((t) => t.id === action.actionType) && " (unavailable)"}
                  </SelectItem>
                ))}
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
          </div>
        </div>

        <div className="flex flex-wrap gap-2">
          <Button onClick={() => void save()} disabled={!dirty && !isNew}>
            <Save /> Save
          </Button>
          {!isNew && (
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
                    .catch((e: unknown) => toast.error(errorMessage(e)))
                }}
              >
                <Copy /> Duplicate
              </Button>
              <Button
                variant="outline"
                disabled={!canRun}
                title={canRun ? undefined : "Save first; runs in the agent for the active profile"}
                onClick={() => {
                  api
                    .runWorkflow(profileId, workflow.id)
                    .then(setReport)
                    .catch((e: unknown) => toast.error(errorMessage(e)))
                }}
              >
                <Play /> Run
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
                    .catch((e: unknown) => toast.error(errorMessage(e)))
                }}
              />
            </>
          )}
        </div>

        {report && (
          <div className="grid gap-1 rounded-md bg-muted p-3 text-xs" role="status">
            <p className="font-medium">Result: {report.status}</p>
            <ol className="grid gap-0.5 font-mono">
              {report.steps.map((step, index) => (
                <li key={index}>
                  {index + 1}. {actions.find((a) => a.id === step.action)?.name ?? "?"} —{" "}
                  {step.result.status}: {step.result.message}
                </li>
              ))}
            </ol>
          </div>
        )}
        {!isNew && usages.length > 0 && (
          <div className="grid gap-1">
            <h3 className="text-sm font-medium">Used in</h3>
            <UsageList usages={usages} />
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function SortableStep({
  id,
  index,
  count,
  step,
  view,
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
  types: ActionType[]
  onChange: (change: Partial<WorkflowStep>) => void
  onMove: (to: number) => void
  onRemove: () => void
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id,
  })
  const [showParameters, setShowParameters] = useState(Object.keys(step.overrides ?? {}).length > 0)
  const action = view.profile.actions.find((a) => a.id === step.action)
  const type = types.find((t) => t.id === action?.actionType)

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
            {view.profile.actions.map((a) => (
              <SelectItem key={a.id} value={a.id}>
                {a.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
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
            {CONDITIONS.filter((c) => conditionAllowed(parseCondition(c.value), index)).map((c) => (
              <SelectItem key={c.value} value={c.value}>
                {c.label}
              </SelectItem>
            ))}
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
      {type && type.parameters.length > 0 && (
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
