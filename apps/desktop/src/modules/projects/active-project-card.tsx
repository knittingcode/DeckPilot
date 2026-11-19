import { SettingsRow } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"

import { ProjectSwitcher } from "./project-switcher"

/** Dashboard row: the active project of the active profile and a switcher. */
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
    <SettingsRow
      label="Active project"
      description={
        project
          ? `${project.key} · ${project.name}${view ? ` in ${view.profile.name}` : ""}`
          : activeId
            ? "None"
            : "Activate a profile to switch projects."
      }
    >
      {view && activeId && (
        <ProjectSwitcher
          profileId={activeId}
          projects={view.profile.projects}
          activeProjectId={view.activeProjectId}
        />
      )}
    </SettingsRow>
  )
}
