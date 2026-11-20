// The first-party plugins that activate the app's own screens (plugin API
// 1.9, `hostViews`), as the agent lists them, for tests.

import type { PluginState, PluginSummary } from "@/plugins/types"

/** `deckpilot.workflows`: the Workflows screen and its navigation entry. */
export function workflowsPlugin(state: PluginState = "enabled"): PluginSummary {
  return {
    id: "deckpilot.workflows",
    name: "Workflows",
    publisher: "DeckPilot",
    version: "0.1.0",
    state,
    origin: { type: "bundled" },
    actions: [],
    // The agent lists host views only while the plugin is enabled.
    ...(state === "enabled"
      ? {
          navigation: [
            {
              id: "workflows",
              label: "Workflows",
              view: { id: "workflows", kind: "workflows", context: "profile" },
            },
          ],
        }
      : {}),
  }
}

/** `deckpilot.projects`: the Projects screen, its status item and dashboard card. */
export function projectsPlugin(state: PluginState = "enabled"): PluginSummary {
  return {
    id: "deckpilot.projects",
    name: "Projects",
    publisher: "DeckPilot",
    version: "0.1.0",
    state,
    origin: { type: "bundled" },
    actions: [],
    ...(state === "enabled"
      ? {
          navigation: [
            {
              id: "projects",
              label: "Projects",
              view: { id: "projects", kind: "projects", context: "profile" },
            },
          ],
          statusItems: ["activeProject"],
          dashboardCards: ["activeProject"],
        }
      : {}),
  }
}
