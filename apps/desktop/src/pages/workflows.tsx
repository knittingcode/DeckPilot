import { ListOrdered, Plus, Workflow as WorkflowIcon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAsync } from "@/hooks/use-async"
import { useEditingProfile } from "@/hooks/use-profile"
import { useTitlebar } from "@/hooks/use-titlebar"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import type { ActionType, Conversion, ProfileView, Workflow } from "@/lib/types"
import { type WorkflowSelection, shownWorkflow } from "@/lib/workflow-model"
import { StepEditor } from "@/pages/workflow-steps"
import { emptyGraph } from "@/workflow-builder/graph"
import { WorkflowRun } from "@/workflow-builder/workflow-run"

export function WorkflowsPage() {
  return (
    <Page title="Workflows" description="Connect actions into workflows" profileScoped>
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
  return (
    <WorkflowsView view={state.data} types={types.data} profileId={profileId} reload={reload} />
  )
}

const NEW = "new"
const NEW_STEPS = "new-steps"

function newWorkflow(): Workflow {
  // New workflows keep project changes to their runs.
  return {
    id: crypto.randomUUID(),
    name: "New workflow",
    description: "",
    steps: [],
    projectScope: "execution",
  }
}

/** Switches between the profile's workflows. */
function WorkflowSwitcher({
  workflows,
  value,
  onChange,
}: {
  workflows: Workflow[]
  value: string
  onChange: (id: string) => void
}) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="h-7 w-64 text-sm" aria-label="Workflow">
        <WorkflowIcon className="size-3.5" />
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {workflows.map((w) => (
          <SelectItem key={w.id} value={w.id}>
            {w.name}
            {!w.graph && <span className="text-muted-foreground"> · steps</span>}
          </SelectItem>
        ))}
        {workflows.length > 0 && <SelectSeparator />}
        <SelectItem value={NEW}>New workflow</SelectItem>
        <SelectItem value={NEW_STEPS}>New step workflow</SelectItem>
      </SelectContent>
    </Select>
  )
}

function WorkflowsView({
  view,
  types,
  profileId,
  reload,
}: {
  view: ProfileView
  types: ActionType[]
  profileId: string
  reload: () => unknown
}) {
  const { refresh, modules } = useAppContext()
  const workflows = view.profile.workflows
  const [selection, setSelection] = useState<WorkflowSelection>({ id: workflows[0]?.id ?? NEW })
  // A new workflow gets a fresh id each time "New workflow" is chosen.
  const [draft, setDraft] = useState(newWorkflow)
  const select = (id: string) => {
    if (id === NEW || id === NEW_STEPS) setDraft(newWorkflow())
    setSelection({ id })
  }
  // Step workflows opened in the graph editor; stored only when saved.
  const [conversions, setConversions] = useState<Record<string, Conversion>>({})
  const shown = shownWorkflow(selection, workflows, view, [NEW, NEW_STEPS])
  const current = workflows.find((w) => w.id === shown)
  // After a change the requested workflow may only arrive with the reload;
  // until then the editor keeps what it shows.
  const changed = (id?: string) => {
    setSelection({
      id: id ?? workflows.find((w) => w.id !== shown)?.id ?? NEW,
      waiting: { data: view, shown },
    })
    void reload()
    refresh()
  }
  useTitlebar({
    actions: [
      {
        id: "workflows.new",
        label: "New workflow",
        icon: Plus,
        placement: "secondary",
        order: 10,
        onSelect: () => {
          select(NEW)
        },
      },
      {
        id: "workflows.new-steps",
        label: "New step workflow",
        icon: ListOrdered,
        placement: "overflow",
        order: 11,
        onSelect: () => {
          select(NEW_STEPS)
        },
      },
    ],
  })
  const switcher = (
    <WorkflowSwitcher
      workflows={workflows}
      value={current ? current.id : shown === NEW_STEPS ? NEW_STEPS : NEW}
      onChange={select}
    />
  )

  if (!current && shown === NEW_STEPS) {
    return (
      <div className="grid gap-4">
        <div className="flex flex-wrap items-center gap-3 rounded-lg border bg-muted/40 p-3">
          {switcher}
          <p className="flex-1 text-sm text-muted-foreground">
            A step workflow runs its actions in order. It can be opened in the graph editor later.
          </p>
        </div>
        <StepEditor
          key={draft.id}
          view={view}
          types={types}
          profileId={profileId}
          initial={draft}
          isNew
          onChanged={changed}
        />
      </div>
    )
  }

  if (current && !current.graph && !conversions[current.id]) {
    return (
      <div className="grid gap-4">
        <div className="flex flex-wrap items-center gap-3 rounded-lg border bg-muted/40 p-3">
          {switcher}
          <p className="flex-1 text-sm text-muted-foreground">
            This workflow runs its steps in order. Open it in the graph editor to add branches,
            parallel steps and loops; the steps are kept.
          </p>
          <Button
            size="sm"
            onClick={() => {
              api
                .convertWorkflow(profileId, current)
                .then((conversion) => {
                  setConversions((c) => ({ ...c, [current.id]: conversion }))
                })
                .catch((e: unknown) => toast.error(errorMessage(e)))
            }}
          >
            Open in graph editor
          </Button>
        </div>
        <StepEditor
          key={current.id}
          view={view}
          types={types}
          profileId={profileId}
          initial={current}
          isNew={false}
          onChanged={changed}
        />
      </div>
    )
  }

  const workflow = current ?? draft
  const conversion = current ? conversions[current.id] : undefined
  return (
    <WorkflowRun
      key={workflow.id}
      profileId={profileId}
      view={view}
      types={types}
      modules={modules}
      initial={workflow}
      graph={workflow.graph ?? conversion?.graph ?? emptyGraph()}
      isNew={!current}
      notes={workflow.graph ? [] : (conversion?.notes ?? [])}
      toolbar={switcher}
      // The conversion stays until the reloaded profile has the saved
      // graph, so the editor is not replaced by the step editor meanwhile.
      onSaved={changed}
      onDeleted={() => {
        changed()
      }}
      onActionsChanged={async () => {
        await reload()
      }}
    />
  )
}
