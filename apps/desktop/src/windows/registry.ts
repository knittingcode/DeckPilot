import { lazy } from "react"

import { moduleWindowView } from "@/modules"

type View = React.LazyExoticComponent<() => React.JSX.Element>

/** Windows of the application itself, by identifier. */
const CORE_WINDOWS: Record<string, View> = {
  "security.setup": lazy(() =>
    import("./security-windows").then((m) => ({ default: m.SetupWindow })),
  ),
  "security.unlock": lazy(() =>
    import("./security-windows").then((m) => ({ default: m.UnlockWindow })),
  ),
  "security.migrate": lazy(() =>
    import("./security-windows").then((m) => ({ default: m.MigrateWindow })),
  ),
  "security.confirm": lazy(() =>
    import("./security-windows").then((m) => ({ default: m.ConfirmWindow })),
  ),
  "workflows.execution-summary": lazy(() =>
    import("./workflow-summary").then((m) => ({ default: m.WorkflowSummaryWindow })),
  ),
}

/** The view for a window identifier: the application's or a module's. */
export function windowView(id: string): View | null {
  return Object.hasOwn(CORE_WINDOWS, id) ? (CORE_WINDOWS[id] ?? null) : moduleWindowView(id)
}
