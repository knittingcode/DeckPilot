import { FolderKanban } from "lucide-react"
import { lazy } from "react"

import type { FrontendModule } from "../types"
import { ActiveProjectStatus } from "./active-project-status"

/** User interface of the `projects` module. */
export const projectsModule: FrontendModule = {
  id: "projects",
  icon: FolderKanban,
  pages: {
    projects: {
      view: lazy(() => import("./projects-page").then((m) => ({ default: m.ProjectsPage }))),
      icon: FolderKanban,
    },
  },
  statusItems: [ActiveProjectStatus],
  dashboardCards: [
    lazy(() => import("./active-project-card").then((m) => ({ default: m.ActiveProjectCard }))),
  ],
}
