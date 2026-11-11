import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import type { ProjectScope } from "@/lib/types"

/**
 * The per-workflow permission to change DeckPilot's active project. Without
 * it, project selections inside a run only affect the nodes after them.
 */
export function ProjectScopeSetting({
  scope,
  selectsProjects,
  onChange,
}: {
  scope: ProjectScope
  selectsProjects: boolean
  onChange: (scope: ProjectScope) => void
}) {
  const application = scope === "application"
  return (
    <div className="grid gap-1.5 rounded-md border p-2.5">
      <div className="flex items-center gap-2">
        <Label htmlFor="workflow-project-scope" className="flex-1 text-xs">
          May change the active project
        </Label>
        <Switch
          id="workflow-project-scope"
          checked={application}
          onCheckedChange={(checked) => {
            onChange(checked ? "application" : "execution")
          }}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {application
          ? "When a run ends on another project, it becomes DeckPilot’s active project — unless the active project changed during the run."
          : selectsProjects
            ? "Project selections stay in each run: they affect the nodes after them, never DeckPilot’s active project or other runs."
            : "Runs use the active project of the moment they start."}
      </p>
    </div>
  )
}
