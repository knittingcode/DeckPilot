// The marketplace's client of the agent, through the Tauri commands of
// `src-tauri/src/commands/plugins.rs`. The commands refuse unless the
// agent answers plugin requests; every refusal arrives as a `PluginError`
// with the backend's `reason`.
//
// One IPC message to the agent carries at most 64 KiB, which a plugin
// with many actions or a large icon does not fit. When the agent lists
// the runtime feature `pagedAnswers`, the backend reads `listPlugins`,
// `getPlugin` and every change's details in parts (`listPluginsPaged`,
// `getPluginPaged`, `getPluginActions`, `getPluginIcon`) and returns them
// whole, so the answers here are complete; an older agent answers in one
// message, as it always did.

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

import { toPluginError, type PluginClient } from "./api"
import type {
  AvailablePlugins,
  KnownPlugin,
  ModuleMigration,
  NavigationIconTheme,
  PluginActionOutcome,
  PluginCredentialSlot,
  PluginDetails,
  PluginDialogAnswer,
  PluginEvent,
  PluginInstallPlan,
  PluginNavigationItem,
  PluginRemovalPlan,
  PluginRetention,
  PluginRuntimeStatus,
  PluginSettings,
  PluginSettingValue,
  PluginSummary,
  PluginUpdateResult,
} from "./types"

export type {
  KnownPlugin,
  NavigationIconTheme,
  PluginCredentialSlot,
  PluginNavigationEntry,
  PluginNavigationItem,
  PluginView,
  PluginViewKind,
  PluginRetention,
  PluginSettingField,
  PluginSettings,
  PluginSettingValue,
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

/**
 * Tells the app whether the main window's plugin dialog host is mounted:
 * only then does it announce plugin dialogs to the agent, so a plugin
 * never asks a window that cannot answer (`plugin_dialogs_ready`).
 */
export function setPluginDialogsReady(ready: boolean): Promise<void> {
  return call<null>("plugin_dialogs_ready", { ready }).then(() => undefined)
}

/**
 * Answers a plugin dialog (`answer_plugin_dialog`). The agent checks the
 * answer against the dialog: it refuses one that does not fit with the
 * reason `invalidAnswer` (the dialog stays open), and one for a dialog that
 * is no longer open with `dialogClosed`.
 */
export function answerPluginDialog(
  dialog: string,
  plugin: string,
  answer: PluginDialogAnswer,
): Promise<void> {
  return call<null>("answer_plugin_dialog", { dialog, plugin, answer }).then(() => undefined)
}

/** `listKnownPlugins`. */
export function listKnownPlugins(): Promise<KnownPlugin[]> {
  return call<KnownPlugin[]>("list_known_plugins")
}

/**
 * The toolbar entries of the installed and enabled plugins (plugin API
 * 1.3); empty with an agent that has none to list. Refused like
 * `listPlugins` when the agent answers no plugin request.
 */
export function listPluginNavigation(): Promise<PluginNavigationItem[]> {
  return call<PluginNavigationItem[]>("list_plugin_navigation")
}

/**
 * A navigation entry's icon as a `data:` URL (`image/png`, `image/webp`
 * or a sanitized `image/svg+xml`) to render through `<img>` only; `null`
 * on any failure, when the app shows its fallback icon. Never rejects
 * for a missing or unsafe icon.
 */
export function getPluginNavigationIcon(
  plugin: string,
  entry: string,
  theme: NavigationIconTheme,
): Promise<string | null> {
  return call<string | null>("get_plugin_navigation_icon", { plugin, entry, theme })
}

/** `uninstallPlugin` with `keep`. */
export function uninstallPluginKeeping(plugin: string, keep: PluginRetention): Promise<void> {
  return call<null>("uninstall_plugin", { plugin, keep }).then(() => undefined)
}

export function getPluginSettings(plugin: string): Promise<PluginSettings> {
  return call<PluginSettings>("get_plugin_settings", { plugin })
}

/** Merges `values` into the saved settings; `null` resets one to its default. */
export function setPluginSettings(
  plugin: string,
  values: Record<string, PluginSettingValue | null>,
): Promise<PluginSettings> {
  return call<PluginSettings>("set_plugin_settings", { plugin, values })
}

export function listPluginCredentials(plugin: string): Promise<PluginCredentialSlot[]> {
  return call<PluginCredentialSlot[]>("list_plugin_credentials", { plugin })
}

/** Sends a secret to the agent; the answer never contains it. */
export function setPluginCredential(
  plugin: string,
  key: string,
  secret: string,
): Promise<PluginCredentialSlot[]> {
  return call<PluginCredentialSlot[]>("set_plugin_credential", { plugin, key, secret })
}

export function deletePluginCredential(
  plugin: string,
  key: string,
): Promise<PluginCredentialSlot[]> {
  return call<PluginCredentialSlot[]>("delete_plugin_credential", { plugin, key })
}

export function createTauriPluginClient(): PluginClient {
  return {
    getRuntime: () => call<PluginRuntimeStatus>("get_plugin_runtime"),
    listPlugins: () => call<PluginSummary[]>("list_plugins"),
    listNavigation: listPluginNavigation,
    getNavigationIcon: getPluginNavigationIcon,
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
    // The original request form unless something is kept.
    uninstallPlugin: (plugin, keep) =>
      keep?.settings || keep?.credentials
        ? uninstallPluginKeeping(plugin, {
            ...(keep.settings ? { settings: true } : {}),
            ...(keep.credentials ? { credentials: true } : {}),
          })
        : call<null>("uninstall_plugin", { plugin }).then(() => undefined),
    restoreBundledPlugin: (plugin) => call<PluginDetails>("restore_bundled_plugin", { plugin }),
    listKnownPlugins,
    getSettings: getPluginSettings,
    setSettings: setPluginSettings,
    listCredentials: listPluginCredentials,
    setCredential: setPluginCredential,
    deleteCredential: deletePluginCredential,
    grantUserOrigin: (plugin, setting, origin, privateNetwork) =>
      call<PluginSettings>("grant_plugin_user_origin", {
        plugin,
        setting,
        origin,
        privateNetwork,
      }),
    revokeUserOrigin: (plugin, setting) =>
      call<PluginSettings>("revoke_plugin_user_origin", { plugin, setting }),
    planInstall: (plugin, version, optional) =>
      call<PluginInstallPlan>("plan_plugin_install", {
        plugin,
        version: version ?? null,
        optional: [...optional],
      }),
    installPlan: (plan, optional, consent, activate) =>
      call<PluginDetails>("install_plugin_plan", {
        plugin: plan.plugin,
        version: plan.version,
        optional: [...optional],
        fingerprint: plan.fingerprint,
        consent,
        activate,
      }),
    planRemoval: (plugin, uninstall) =>
      call<PluginRemovalPlan>("plan_plugin_removal", { plugin, uninstall }),
    removePlugin: (plugin, uninstall, keep, withDependents, orphans) =>
      call<null>("remove_plugin", {
        plugin,
        uninstall,
        keep: keep ?? null,
        withDependents,
        orphans: [...orphans],
      }).then(() => undefined),
    invokeAction: (plugin, action, params) =>
      call<PluginActionOutcome>("invoke_plugin_action", { plugin, action, params }),
    listModuleMigrations: () => call<ModuleMigration[]>("list_module_migrations"),
    decideModuleMigration: (module, approve, decline) =>
      call<ModuleMigration>("decide_module_migration", {
        module,
        approve: [...approve],
        decline: [...decline],
      }),
    retryModuleMigration: (module) => call<ModuleMigration>("retry_module_migration", { module }),
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
