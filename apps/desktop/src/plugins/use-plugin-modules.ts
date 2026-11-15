// The installed plugins as modules of the action editors (experimental).
//
// Only builds with `VITE_DECKPILOT_PLUGINS=1` ask the agent anything: in
// every other build the hook returns the same empty list and loads no
// plugin client, so no request is ever sent. The answer of `listPlugins`
// is also what the backend validates plugin actions against
// (`src-tauri/src/plugin_actions.rs`), so the editors and the backend see
// the same plugins.

import { useEffect, useState } from "react"

import { pluginModules } from "@/lib/plugin-actions"
import type { ModuleInfo } from "@/lib/types"

import { type PluginClient, usePluginClientOverride } from "./api"

const NONE: ModuleInfo[] = []

/**
 * One module per installed plugin, read again whenever `agentKey` changes
 * (the agent started, stopped or was replaced) and whenever the agent
 * reports that a plugin changed. While the agent cannot answer plugin
 * requests there are none: instances in profiles then show as unavailable.
 */
export function usePluginModules(agentKey: string): ModuleInfo[] {
  const override = usePluginClientOverride()
  const [modules, setModules] = useState<ModuleInfo[]>(NONE)

  useEffect(() => {
    // Inline, so that Vite folds it to a constant and builds without
    // plugins contain neither the client nor its commands.
    if (import.meta.env.VITE_DECKPILOT_PLUGINS !== "1") return
    let active = true
    let stop: (() => void) | undefined
    let generation = 0
    const load = (client: PluginClient) => {
      const current = ++generation
      // The names of uninstalled plugins only improve messages.
      const known = client.listKnownPlugins().catch(() => [])
      client.listPlugins().then(
        async (plugins) => {
          const names = await known
          if (active && current === generation) setModules(pluginModules(plugins, names))
        },
        () => {
          if (active && current === generation) setModules(NONE)
        },
      )
    }
    const start = async () => {
      const client = override ?? (await import("./client")).createTauriPluginClient()
      if (!active) return
      load(client)
      stop = client.subscribe((event) => {
        if (event.type === "changed") load(client)
      })
    }
    void start()
    return () => {
      active = false
      stop?.()
    }
  }, [override, agentKey])

  return modules
}
