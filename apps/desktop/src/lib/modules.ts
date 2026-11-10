import type { ModuleInfo } from "./types"

/** The module that provides an action type, enabled or not. */
export function moduleOf(modules: ModuleInfo[], typeId: string): ModuleInfo | undefined {
  return modules.find((m) => m.actionTypes.some((t) => t.id === typeId))
}
