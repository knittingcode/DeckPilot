// The app's own screens, status items and dashboard cards that first-party
// plugins activate (plugin API 1.9, `hostViews`; docs/marketplace/
// module-migration.md §11, D6). Plugins ship no UI code: the views are
// this app's, shown only while `lib/plugin-features.ts` says their plugin
// activates them.

import { FolderKanban, ListOrdered, type LucideIcon } from "lucide-react"
import { lazy } from "react"

import type { TranslationKey } from "@/i18n"
import type { PageId } from "@/lib/app-context"
import type { HostViewKind, PluginFeatures } from "@/lib/plugin-features"
import type { ToolbarContext } from "@/lib/toolbar-context"

import {
  ActiveProjectCardView,
  ActiveProjectStatus,
  PROJECTS_PAGE,
  ProjectsPageView,
} from "./projects"

type View = React.LazyExoticComponent<() => React.JSX.Element>

export interface HostPage {
  /** The page in the main window's navigation. */
  id: PageId
  kind: HostViewKind
  label: TranslationKey
  /** Before the page registers its own title (English, like the core pages'). */
  fallbackTitle: string
  icon: LucideIcon
  view: View
  /** Which toolbar selectors it shows. */
  context: ToolbarContext
}

/** In the rail's "Configure" group, in this order, after the Action Library. */
export const HOST_PAGES: readonly HostPage[] = [
  {
    id: "workflows",
    kind: "workflows",
    label: "navigation:pages.workflows",
    fallbackTitle: "Workflows",
    icon: ListOrdered,
    view: lazy(() => import("@/pages/workflows").then((m) => ({ default: m.WorkflowsPage }))),
    context: "profile",
  },
  {
    id: PROJECTS_PAGE,
    kind: "projects",
    label: "navigation:pages.projects",
    fallbackTitle: "Projects",
    icon: FolderKanban,
    view: ProjectsPageView,
    // A profile's projects.
    context: "profile",
  },
]

/** The host page of `id`, whether or not it is shown now. */
export function hostPage(id: string): HostPage | undefined {
  return HOST_PAGES.find((page) => page.id === id)
}

/** The host pages their plugins activate now. */
export function hostPages(features: Pick<PluginFeatures, "hostViews">): HostPage[] {
  return HOST_PAGES.filter((page) => features.hostViews.has(page.kind))
}

/** Whether `id` is a host page shown now. */
export function isHostPageAvailable(
  features: Pick<PluginFeatures, "hostViews">,
  id: string,
): boolean {
  const page = hostPage(id)
  return page !== undefined && features.hostViews.has(page.kind)
}

/** Status bar components of the active plugins; each registers its items itself. */
export function hostStatusItems(features: Pick<PluginFeatures, "statusItems">) {
  return features.statusItems.has("activeProject")
    ? [{ key: "deckpilot.projects:activeProject", component: ActiveProjectStatus }]
    : []
}

/** Dashboard cards of the active plugins. */
export function hostDashboardCards(features: Pick<PluginFeatures, "dashboardCards">) {
  return features.dashboardCards.has("activeProject")
    ? [{ key: "deckpilot.projects:activeProject", card: ActiveProjectCardView }]
    : []
}
