import type { ModuleInfo } from "./types"

/** A module as the backend reports it, for tests. */
export function moduleInfo(overrides: Partial<ModuleInfo> & { id: string }): ModuleInfo {
  return {
    version: "1.0.0",
    name: overrides.id.charAt(0).toUpperCase() + overrides.id.slice(1),
    description: `The ${overrides.id} module`,
    platforms: [],
    pages: [],
    capabilities: [],
    config: [],
    dependencies: [],
    enabledByDefault: true,
    enabled: true,
    supported: true,
    configValues: {},
    actionTypes: [],
    ...overrides,
  }
}
