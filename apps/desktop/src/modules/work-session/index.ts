import { Timer } from "lucide-react"
import { lazy } from "react"

import type { FrontendModule } from "../types"

/**
 * User interface of the `work-session` module: the Work Session page. The
 * page appears once the backend reports the module (its manifest, W5).
 */
export const workSessionModule: FrontendModule = {
  id: "work-session",
  icon: Timer,
  pages: {
    "work-session": {
      view: lazy(() => import("./work-session-page").then((m) => ({ default: m.WorkSessionPage }))),
      icon: Timer,
    },
  },
}
