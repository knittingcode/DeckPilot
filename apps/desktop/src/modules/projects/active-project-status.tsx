import { FolderKanban } from "lucide-react"

import { useStatusItem } from "@/hooks/use-status"
import { useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"

/**
 * The active project of the profile that is active on the device, as the
 * agent reports both together, so the project always belongs to the shown
 * profile. Hidden while there is none.
 */
export function ActiveProjectStatus() {
  const { t } = useTranslation()
  const { agent, navigate } = useAppContext()
  const project = agent?.running ? agent.status?.activeProject : null
  const profile = agent?.status?.activeProfile
  useStatusItem(
    project && profile
      ? {
          id: "projects.active-project",
          owner: "projects",
          placement: "left",
          priority: 70,
          icon: FolderKanban,
          label: project.key,
          tooltip: t("workspace:projects.status.tooltip", {
            profile: profile.name,
            project: project.name,
          }),
          onSelect: () => {
            navigate("projects/projects")
          },
        }
      : null,
  )
  return null
}
