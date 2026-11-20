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

const sessionPage = { id: "work-session", title: "Work Session", icon: "timer", order: 10 }

describe("module navigation", () => {
  it("adds pages of enabled modules that have a view", () => {
    const modules = [
      module("general", true, []),
      module("work-session", true, [sessionPage]),
      // Projects is a host view of its plugin now (`host-views.ts`), not a module page.
      module("projects", true, [{ id: "projects", title: "Projects", icon: "folder", order: 5 }]),
      // Declared by the backend, but no frontend view exists: not shown.
      module("other", true, [{ id: "x", title: "X", icon: "box", order: 0 }]),
    ]
    expect(moduleNavigation(modules).map((item) => [item.id, item.label])).toEqual([
      ["work-session/work-session", "Work Session"],
    ])
    expect(isModulePageAvailable(modules, "work-session/work-session")).toBe(true)
    expect(isModulePageAvailable(modules, "projects/projects")).toBe(false)
    expect(dashboardCards(modules)).toEqual([])
  })

  it("removes pages and cards of disabled modules", () => {
    const modules = [module("work-session", false, [sessionPage])]
    expect(moduleNavigation(modules)).toEqual([])
    expect(isModulePageAvailable(modules, "work-session/work-session")).toBe(false)
    expect(
      isModulePageAvailable([module("work-session", true, [sessionPage])], "work-session/x"),
    ).toBe(false)
    expect(dashboardCards(modules)).toEqual([])
  })
})
