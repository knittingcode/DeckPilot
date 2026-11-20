import { SettingsRow } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"

import { ProjectSwitcher } from "./project-switcher"

/** Dashboard row: the active project of the active profile and a switcher. */
export function ActiveProjectCard() {
  const { t } = useTranslation()
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
      label={t("workspace:projects.card.label")}
      description={
        project
          ? view
            ? t("workspace:projects.card.projectInProfile", {
                key: project.key,
                project: project.name,
                profile: view.profile.name,
              })
            : t("workspace:projects.card.project", { key: project.key, project: project.name })
          : activeId
            ? t("workspace:projects.card.none")
            : t("workspace:projects.card.noActiveProfile")
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
