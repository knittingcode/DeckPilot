import { lazy } from "react"

export { ActiveProjectStatus } from "./active-project-status"

/**
 * The Projects screen, a host view the `deckpilot.projects` plugin
 * activates (plugin API 1.9, `hostViews`; `modules/host-views.ts`). The
 * page id is the one the legacy module's page had, so links from other
 * windows (`projects/projects`) keep working.
 */
export const PROJECTS_PAGE = "projects/projects"

export const ProjectsPageView = lazy(() =>
  import("./projects-page").then((m) => ({ default: m.ProjectsPage })),
)

/** The dashboard card of the active project (`dashboardCards: activeProject`). */
export const ActiveProjectCardView = lazy(() =>
  import("./active-project-card").then((m) => ({ default: m.ActiveProjectCard })),
)
