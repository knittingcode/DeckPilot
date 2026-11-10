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
  "portability.export": lazy(() =>
    import("./portability-windows").then((m) => ({ default: m.ExportWindow })),
  ),
  "portability.import": lazy(() =>
    import("./portability-windows").then((m) => ({ default: m.ImportWindow })),
  ),
  "updates.progress": lazy(() =>
    import("./update-window").then((m) => ({ default: m.UpdateWindow })),
  ),
  "app.logs": lazy(() => import("./logs-window").then((m) => ({ default: m.LogsWindow }))),
}

/** The view for a window identifier: the application's or a module's. */
export function windowView(id: string): View | null {
  return Object.hasOwn(CORE_WINDOWS, id) ? (CORE_WINDOWS[id] ?? null) : moduleWindowView(id)
}
