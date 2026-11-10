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
  /** Shown in Settings → Modules. */
  icon?: LucideIcon
  /** Views for the page ids declared in the module's manifest. */
  pages: Record<string, { view: View; icon: LucideIcon }>
  /** Cards shown on the dashboard while the module is enabled. */
  dashboardCards?: View[]
  /**
   * Views of the native windows declared in the module's manifest, by
   * window identifier (`<module>.<name>`). The backend decides whether a
   * window may open.
   */
  windows?: Record<string, View>
  /**
   * A settings page of the module's own, shown under Settings → Extensions
   * while the module is enabled. Without one, a module that declares
   * settings in its manifest gets a page generated from them.
   */
  settings?: View
}
