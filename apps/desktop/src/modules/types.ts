import type { LucideIcon } from "lucide-react"
import type { LazyExoticComponent } from "react"

type View = LazyExoticComponent<() => React.JSX.Element>

/**
 * The user interface a feature module contributes. The backend decides which
 * modules exist and whether they are enabled; this only maps a module's page
 * ids to views.
 */
export interface FrontendModule {
  id: string
  /** Views for the page ids declared in the module's manifest. */
  pages: Record<string, { view: View; icon: LucideIcon }>
  /** Cards shown on the dashboard while the module is enabled. */
  dashboardCards?: View[]
}
