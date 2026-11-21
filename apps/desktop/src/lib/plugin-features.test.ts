import { describe, expect, it } from "vitest"

import { pluginModules } from "@/lib/plugin-actions"
import { PROJECTS_PLUGIN, WORKFLOWS_PLUGIN, pluginFeatures } from "@/lib/plugin-features"
import type { PluginSummary } from "@/plugins/types"
import { projectsPlugin, workflowsPlugin } from "@/test/first-party"

function features(plugins: PluginSummary[]) {
  return pluginFeatures(pluginModules(plugins), true)
}

describe("plugin features", () => {
  it("shows nothing optional without plugins", () => {
    const none = features([])
    expect(none.isPluginActive(WORKFLOWS_PLUGIN)).toBe(false)
    expect(none.isPluginActive(PROJECTS_PLUGIN)).toBe(false)
    expect([...none.hostViews]).toEqual([])
    expect([...none.statusItems]).toEqual([])
    expect([...none.dashboardCards]).toEqual([])
  })

  it("follows the plugins' state: enabled shows their screens, disabled hides them", () => {
    const on = features([workflowsPlugin(), projectsPlugin()])
    expect(on.isPluginActive(WORKFLOWS_PLUGIN)).toBe(true)
    expect(on.isPluginActive(PROJECTS_PLUGIN)).toBe(true)
    expect([...on.hostViews].sort()).toEqual(["projects", "workflows"])
    expect([...on.statusItems]).toEqual(["activeProject"])
    expect([...on.dashboardCards]).toEqual(["activeProject"])

    for (const state of ["disabled", "broken", "quarantined"] as const) {
      const off = features([workflowsPlugin(state), projectsPlugin(state)])
      expect(off.isPluginActive(WORKFLOWS_PLUGIN)).toBe(false)
      expect(off.isPluginActive(PROJECTS_PLUGIN)).toBe(false)
      expect(off.hostViews.size + off.statusItems.size + off.dashboardCards.size).toBe(0)
    }
    // Enabled, but listing no host view (an older agent): nothing drawn.
    const bare = { ...workflowsPlugin(), navigation: [] }
    expect(features([bare]).hostViews.size).toBe(0)
    expect(features([bare]).isPluginActive(WORKFLOWS_PLUGIN)).toBe(true)
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
    const listed = features([impostor, { ...workflowsPlugin(), statusItems: ["activeProject"] }])
    expect([...listed.hostViews]).toEqual(["workflows"])
    expect(listed.statusItems.size).toBe(0)
    expect(listed.dashboardCards.size).toBe(0)
  })

  it("keeps the pre-plugin Workflows screen in builds without plugins, and no Projects", () => {
    const core = pluginFeatures([], false)
    expect(core.isPluginActive(WORKFLOWS_PLUGIN)).toBe(true)
    // The built-in `projects` module is gone (WP-C6): no provider.
    expect(core.isPluginActive(PROJECTS_PLUGIN)).toBe(false)
    expect([...core.hostViews]).toEqual(["workflows"])
    expect(core.statusItems.size + core.dashboardCards.size).toBe(0)
    // Plugins listed by a build without plugins count for nothing.
    expect(pluginFeatures(pluginModules([projectsPlugin()]), false).hostViews.has("projects")).toBe(
      false,
    )
  })
})
