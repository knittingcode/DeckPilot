import { describe, expect, it } from "vitest"

import { pluginModules } from "@/lib/plugin-actions"
import { pluginFeatures } from "@/lib/plugin-features"
import { projectsPlugin, workflowsPlugin } from "@/test/first-party"

import {
  hostDashboardCards,
  hostPage,
  hostPages,
  hostStatusItems,
  isHostPageAvailable,
} from "./host-views"

const withPlugins = (plugins: Parameters<typeof pluginModules>[0]) =>
  pluginFeatures([], pluginModules(plugins), true)

describe("host views", () => {
  it("draws nothing without the plugins", () => {
    const none = withPlugins([])
    expect(hostPages(none)).toEqual([])
    expect(hostStatusItems(none)).toEqual([])
    expect(hostDashboardCards(none)).toEqual([])
    expect(isHostPageAvailable(none, "workflows")).toBe(false)
    expect(isHostPageAvailable(none, "projects/projects")).toBe(false)
  })

  it("draws the screens, the status item and the card of enabled plugins", () => {
    const on = withPlugins([workflowsPlugin(), projectsPlugin()])
    expect(hostPages(on).map((p) => [p.id, p.context])).toEqual([
      ["workflows", "profile"],
      ["projects/projects", "profile"],
    ])
    expect(hostStatusItems(on).map((i) => i.key)).toEqual(["deckpilot.projects:activeProject"])
    expect(hostDashboardCards(on).map((c) => c.key)).toEqual(["deckpilot.projects:activeProject"])
    expect(isHostPageAvailable(on, "projects/projects")).toBe(true)
    expect(isHostPageAvailable(on, "designer")).toBe(false)
    // Only Workflows enabled: no project UI at all.
    const workflows = withPlugins([workflowsPlugin(), projectsPlugin("disabled")])
    expect(hostPages(workflows).map((p) => p.id)).toEqual(["workflows"])
    expect(hostStatusItems(workflows)).toEqual([])
    expect(hostDashboardCards(workflows)).toEqual([])
    expect(hostPage("projects/projects")?.label).toBe("navigation:pages.projects")
  })
})
