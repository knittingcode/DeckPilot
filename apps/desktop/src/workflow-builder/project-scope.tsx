import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useTranslation } from "@/i18n"
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
  const { t } = useTranslation()
  const application = scope === "application"
  return (
    <div className="grid gap-1.5 rounded-md border p-2.5">
      <div className="flex items-center gap-2">
        <Label htmlFor="workflow-project-scope" className="flex-1 text-xs">
          {t("workflows:inspector.projectScope.label")}
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
          ? t("workflows:inspector.projectScope.application")
          : selectsProjects
            ? t("workflows:inspector.projectScope.execution")
            : t("workflows:inspector.projectScope.atStart")}
      </p>
    </div>
  )
}
