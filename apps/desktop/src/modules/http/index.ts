import { Globe } from "lucide-react"
import { lazy } from "react"

import type { FrontendModule } from "../types"

/** User interface of the `http` module: its settings with stored credentials. */
export const httpModule: FrontendModule = {
  id: "http",
  icon: Globe,
  pages: {},
  settings: lazy(() => import("./http-settings").then((m) => ({ default: m.HttpSettings }))),
}
