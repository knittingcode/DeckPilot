// The marketplace's client of the agent, through the Tauri commands of
// `src-tauri/src/commands/plugins.rs`. The commands refuse unless the
// agent answers plugin requests; every refusal arrives as a `PluginError`
// with the backend's `reason`.

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

import { toPluginError, type PluginClient } from "./api"
import type {
  AvailablePlugins,
  PluginActionOutcome,
  PluginDetails,
  PluginEvent,
  PluginRuntimeStatus,
  PluginSummary,
  PluginUpdateResult,
} from "./types"

/** Backend event with a {@link PluginEvent} (`agent::PLUGIN_EVENT`). */
export const PLUGIN_EVENT = "plugin-event"

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args)
  } catch (error) {
    throw toPluginError(error)
  }
}

export function createTauriPluginClient(): PluginClient {
  return {
    getRuntime: () => call<PluginRuntimeStatus>("get_plugin_runtime"),
    listPlugins: () => call<PluginSummary[]>("list_plugins"),
    getPlugin: (plugin) => call<PluginDetails>("get_plugin", { plugin }),
    listAvailablePlugins: (refresh = false) =>
      call<AvailablePlugins>("list_available_plugins", { refresh }),
    installPlugin: (source, consent) =>
      call<PluginDetails>("install_plugin", { source, consent: [...consent] }),
    enablePlugin: (plugin, consent) =>
      call<PluginDetails>("enable_plugin", { plugin, consent: [...consent] }),
    disablePlugin: (plugin) => call<PluginDetails>("disable_plugin", { plugin }),
    grantPermissions: (plugin, permissions) =>
      call<PluginDetails>("grant_plugin_permissions", { plugin, permissions: [...permissions] }),
    revokePermission: (plugin, permission) =>
      call<PluginDetails>("revoke_plugin_permission", { plugin, permission }),
    updatePlugin: (plugin, version, consent) =>
      call<PluginUpdateResult>("update_plugin", {
        plugin,
        version: version ?? null,
        consent: [...consent],
      }),
    approveUpdate: (plugin, consent) =>
      call<PluginUpdateResult>("approve_plugin_update", { plugin, consent: [...consent] }),
    declineUpdate: (plugin) => call<PluginDetails>("decline_plugin_update", { plugin }),
    rollbackPlugin: (plugin) => call<PluginDetails>("rollback_plugin", { plugin }),
    uninstallPlugin: (plugin) => call<null>("uninstall_plugin", { plugin }).then(() => undefined),
    invokeAction: (plugin, action, params) =>
      call<PluginActionOutcome>("invoke_plugin_action", { plugin, action, params }),
    pickPackage: () => call<string | null>("pick_plugin_package"),
    subscribe: (listener) => {
      let active = true
      let stop: (() => void) | undefined
      void listen<PluginEvent>(PLUGIN_EVENT, (event) => {
        if (active) listener(event.payload)
      }).then((unlisten) => {
        if (active) stop = unlisten
        else unlisten()
      })
      // Asks the app to forward the agent's plugin events; idempotent, and
      // a no-op against agents without plugins.
      void invoke("watch_plugin_events").catch(() => undefined)
      return () => {
        active = false
        stop?.()
      }
    },
  }
}
