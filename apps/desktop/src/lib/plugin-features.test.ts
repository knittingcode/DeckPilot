import { describe, expect, it } from "vitest"

import { pluginModules } from "@/lib/plugin-actions"
import {
  PROJECTS_PLUGIN,
  WORKFLOWS_PLUGIN,
  isExtractedModule,
  pluginFeatures,
} from "@/lib/plugin-features"
import { moduleInfo } from "@/lib/test-fixtures"
import type { ModuleMigration, PluginSummary } from "@/plugins/types"
import { projectsPlugin, workflowsPlugin } from "@/test/first-party"

const legacyProjects = moduleInfo({ id: "projects" })

function features(
  plugins: PluginSummary[],
  modules = [legacyProjects],
  migrations: ModuleMigration[] = [],
) {
  return pluginFeatures(modules, pluginModules(plugins, [], migrations), true)
}

function migration(route: ModuleMigration["route"]): ModuleMigration {
  return {
    module: "projects",
    moduleName: "Projects",
    plugin: PROJECTS_PLUGIN,
    phase: "cutOver",
    attempts: 0,
    maxAttempts: 3,
    canRetry: false,
    route,
    aliases: [],
    updatedAt: 0,
  }
}

describe("plugin features", () => {
  it("shows nothing optional without plugins", () => {
    const none = features([], [])
    expect(none.isPluginActive(WORKFLOWS_PLUGIN)).toBe(false)
    expect(none.isPluginActive(PROJECTS_PLUGIN)).toBe(false)
    expect([...none.hostViews]).toEqual([])
    expect([...none.statusItems]).toEqual([])
    expect([...none.dashboardCards]).toEqual([])
  })

  it("follows the plugins' state: enabled shows their screens, disabled hides them", () => {
    const on = features([workflowsPlugin(), projectsPlugin()], [])
    expect(on.isPluginActive(WORKFLOWS_PLUGIN)).toBe(true)
    expect(on.isPluginActive(PROJECTS_PLUGIN)).toBe(true)
    expect([...on.hostViews].sort()).toEqual(["projects", "workflows"])
    expect([...on.statusItems]).toEqual(["activeProject"])
    expect([...on.dashboardCards]).toEqual(["activeProject"])

    for (const state of ["disabled", "broken", "quarantined"] as const) {
      const off = features([workflowsPlugin(state), projectsPlugin(state)], [])
      expect(off.isPluginActive(WORKFLOWS_PLUGIN)).toBe(false)
      expect(off.isPluginActive(PROJECTS_PLUGIN)).toBe(false)
      expect(off.hostViews.size + off.statusItems.size + off.dashboardCards.size).toBe(0)
    }
    // Enabled, but listing no host view (an older agent): nothing drawn.
    const bare = { ...workflowsPlugin(), navigation: [] }
    expect(features([bare], []).hostViews.size).toBe(0)
    expect(features([bare], []).isPluginActive(WORKFLOWS_PLUGIN)).toBe(true)
  })

  it("draws a host view only for the first-party plugin that owns it", () => {
    const impostor: PluginSummary = {
      ...projectsPlugin(),
      id: "acme.projects",
      navigation: [
        { id: "w", label: "W", view: { id: "w", kind: "workflows" } },
        { id: "p", label: "P", view: { id: "p", kind: "projects" } },
      ],
    }
    const listed = features(
      [impostor, { ...workflowsPlugin(), statusItems: ["activeProject"] }],
      [],
    )
    expect([...listed.hostViews]).toEqual(["workflows"])
    expect(listed.statusItems.size).toBe(0)
    expect(listed.dashboardCards.size).toBe(0)
  })

  it("keeps Projects on the legacy module only until it is migrated", () => {
    // Not migrated (no migration, or routed to the built-in): the module provides it.
    for (const migrations of [[], [migration("builtin")]]) {
      const legacy = features([], [legacyProjects], migrations)
      expect(legacy.isPluginActive(PROJECTS_PLUGIN)).toBe(true)
      expect(legacy.hostViews.has("projects")).toBe(true)
      expect(legacy.statusItems.has("activeProject")).toBe(true)
    }
    // Migrated: the plugin alone decides, even with the module still enabled.
    for (const route of ["plugin", "pluginWithdrawn"] as const) {
      const off = features([projectsPlugin("disabled")], [legacyProjects], [migration(route)])
      expect(off.isPluginActive(PROJECTS_PLUGIN)).toBe(false)
      expect(off.hostViews.has("projects")).toBe(false)
    }
    // A disabled legacy module provides nothing.
    const disabled = features([], [{ ...legacyProjects, enabled: false }])
    expect(disabled.isPluginActive(PROJECTS_PLUGIN)).toBe(false)
  })

  it("keeps the pre-plugin behaviour in builds without plugins", () => {
    const core = pluginFeatures([legacyProjects], [], false)
    expect(core.isPluginActive(WORKFLOWS_PLUGIN)).toBe(true)
    expect(core.isPluginActive(PROJECTS_PLUGIN)).toBe(true)
    expect([...core.hostViews].sort()).toEqual(["projects", "workflows"])
    const noProjects = pluginFeatures([{ ...legacyProjects, enabled: false }], [], false)
    expect(noProjects.isPluginActive(PROJECTS_PLUGIN)).toBe(false)
    expect([...noProjects.hostViews]).toEqual(["workflows"])
    // Plugins listed by a build without plugins count for nothing.
    expect(
      pluginFeatures([], pluginModules([projectsPlugin()]), false).hostViews.has("projects"),
    ).toBe(false)
  })

  it("names the built-in modules whose UI moved to a plugin, in builds with plugins", () => {
    for (const id of ["git", "docker", "workspace", "github", "gitlab", "jira", "projects"]) {
      expect(isExtractedModule(id, true)).toBe(true)
      expect(isExtractedModule(id, false)).toBe(false)
    }
    for (const id of ["general", "system", "http", "work-session", "deck"]) {
      expect(isExtractedModule(id, true)).toBe(false)
    }
  })
})
