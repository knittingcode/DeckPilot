import { describe, expect, it } from "vitest"

import type { ModuleInfo } from "@/lib/types"

import { dashboardCards, isModulePageAvailable, moduleNavigation } from "."

function module(id: string, enabled: boolean, pages: ModuleInfo["pages"]): ModuleInfo {
  return {
    id,
    name: id,
    version: "0.1.0",
    description: "",
    platforms: [],
    pages,
    capabilities: [],
    config: [],
    dependencies: [],
    enabledByDefault: true,
    enabled,
    supported: true,
    configValues: {},
    actionTypes: [],
  }
}

const projectsPage = { id: "projects", title: "Projects", icon: "folder-kanban", order: 10 }

describe("module navigation", () => {
  it("adds pages of enabled modules that have a view", () => {
    const modules = [
      module("general", true, []),
      module("projects", true, [projectsPage]),
      // Declared by the backend, but no frontend view exists: not shown.
      module("other", true, [{ id: "x", title: "X", icon: "box", order: 0 }]),
    ]
    expect(moduleNavigation(modules).map((item) => [item.id, item.label])).toEqual([
      ["projects/projects", "Projects"],
    ])
    expect(isModulePageAvailable(modules, "projects/projects")).toBe(true)
    expect(dashboardCards(modules)).toHaveLength(1)
  })

  it("removes pages and cards of disabled modules", () => {
    const modules = [module("projects", false, [projectsPage])]
    expect(moduleNavigation(modules)).toEqual([])
    expect(isModulePageAvailable(modules, "projects/projects")).toBe(false)
    expect(isModulePageAvailable([module("projects", true, [projectsPage])], "projects/x")).toBe(
      false,
    )
    expect(dashboardCards(modules)).toEqual([])
  })
})
