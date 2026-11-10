import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"

import { ProjectSwitcher } from "./project-switcher"

/** Dashboard card: the active project of the active profile and a switcher. */
export function ActiveProjectCard() {
  const { profiles, revision } = useAppContext()
  const activeId = profiles.activeProfileId
  const { state } = useAsync(
    () => (activeId ? api.getProfile(activeId) : Promise.resolve(null)),
    `${activeId ?? ""}:${revision}`,
  )
  const view = state.status === "ready" ? state.data : null
  const project = view?.profile.projects.find((p) => p.id === view.activeProjectId)
  return (
    <Card>
      <CardHeader>
        <CardDescription>Active project{view ? ` of ${view.profile.name}` : ""}</CardDescription>
        <CardTitle className="text-xl">
          {project ? `${project.key} · ${project.name}` : "None"}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {view && activeId && (
          <ProjectSwitcher
            profileId={activeId}
            projects={view.profile.projects}
            activeProjectId={view.activeProjectId}
          />
        )}
        {!activeId && (
          <p className="text-sm text-muted-foreground">Activate a profile to switch projects.</p>
        )}
      </CardContent>
    </Card>
  )
}
