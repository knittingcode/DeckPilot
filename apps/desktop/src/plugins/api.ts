// Typed client of the plugin marketplace: one method per plugin request of
// IPC protocol 10 (docs/marketplace/agent-integration.md). The only
// implementation in the app is `createTauriPluginClient` (./client), which
// goes through the Tauri commands to the agent. Tests provide their own
// through `PluginClientContext`.

import { createContext, useContext } from "react"

import { type TranslationKey, i18n } from "@/i18n"

import type {
  ActionParamValue,
  AvailablePlugins,
  KnownPlugin,
  ModuleMigration,
  NavigationIconTheme,
  PluginActionOutcome,
  PluginCredentialSlot,
  PluginDetails,
  PluginEvent,
  PluginInstallPlan,
  PluginInstallSource,
  PluginNavigationItem,
  PluginRemovalPlan,
  PluginRetention,
  PluginRuntimeStatus,
  PluginSettings,
  PluginSettingValue,
  PluginSummary,
  PluginUpdateResult,
} from "./types"

export interface PluginClient {
  /** `getPluginRuntime`: answered by every protocol 10 agent. */
  getRuntime(): Promise<PluginRuntimeStatus>
  /** `listPlugins`. */
  listPlugins(): Promise<PluginSummary[]>
  /** `getPlugin`. */
  getPlugin(id: string): Promise<PluginDetails>
  /** `listAvailablePlugins`; `refresh` re-reads the registry first. */
  listAvailablePlugins(refresh?: boolean): Promise<AvailablePlugins>
  /** `installPlugin`: installs in state `disabled`, granting exactly `consent`. */
  installPlugin(source: PluginInstallSource, consent: string[]): Promise<PluginDetails>
  /** `enablePlugin`: grants `consent`, then enables. */
  enablePlugin(id: string, consent: string[]): Promise<PluginDetails>
  /** `disablePlugin`. */
  disablePlugin(id: string): Promise<PluginDetails>
  /** `grantPluginPermissions`. */
  grantPermissions(id: string, permissions: string[]): Promise<PluginDetails>
  /** `revokePluginPermission`. */
  revokePermission(id: string, permission: string): Promise<PluginDetails>
  /** `updatePlugin`: an escalation not covered by `consent` is staged. */
  updatePlugin(
    id: string,
    version: string | undefined,
    consent: string[],
  ): Promise<PluginUpdateResult>
  /** `approvePluginUpdate`: `consent` must list every escalated key. */
  approveUpdate(id: string, consent: string[]): Promise<PluginUpdateResult>
  /** `declinePluginUpdate`. */
  declineUpdate(id: string): Promise<PluginDetails>
  /** `rollbackPlugin`. */
  rollbackPlugin(id: string): Promise<PluginDetails>
  /**
   * `uninstallPlugin`: deletes its storage, settings and credentials too,
   * unless `keep` keeps settings and/or credentials for a later install.
   */
  uninstallPlugin(id: string, keep?: PluginRetention): Promise<void>
  /**
   * `restoreBundledPlugin` (protocol 15, agent feature `restoreBundled`):
   * installs again, from DeckPilot's own bundle, a bundled plugin the user
   * removed, or reinstalls one whose installed copy is broken.
   */
  restoreBundledPlugin(id: string): Promise<PluginDetails>
  /**
   * `planPluginInstall` (agent feature `dependencies`): resolves and
   * verifies what installing `id` takes, with the optional integrations
   * in `optional`; installs nothing.
   */
  planInstall(
    id: string,
    version: string | undefined,
    optional: string[],
  ): Promise<PluginInstallPlan>
  /**
   * `installPluginPlan`: carries out the plan the user confirmed, with
   * the permissions granted to each plugin of it (`consent` by plugin id).
   */
  installPlan(
    plan: PluginInstallPlan,
    optional: string[],
    consent: Record<string, string[]>,
    activate: boolean,
  ): Promise<PluginDetails>
  /** `planPluginRemoval`. */
  planRemoval(id: string, uninstall: boolean): Promise<PluginRemovalPlan>
  /** `removePlugin`: with its enabled dependents disabled first, and the chosen orphans. */
  removePlugin(
    id: string,
    uninstall: boolean,
    keep: PluginRetention | undefined,
    withDependents: boolean,
    orphans: string[],
  ): Promise<void>
  /** `invokePluginAction`. */
  invokeAction(
    id: string,
    action: string,
    params: Record<string, ActionParamValue>,
  ): Promise<PluginActionOutcome>
  /** `listKnownPlugins`: every plugin installed here at some point. */
  listKnownPlugins(): Promise<KnownPlugin[]>
  /** `getPluginSettings`. */
  getSettings(id: string): Promise<PluginSettings>
  /** `setPluginSettings`: merged; `null` resets one to its default. */
  setSettings(
    id: string,
    values: Record<string, PluginSettingValue | null>,
  ): Promise<PluginSettings>
  /** `listPluginCredentials`: the slots and whether each is set, never a secret. */
  listCredentials(id: string): Promise<PluginCredentialSlot[]>
  /** `setPluginCredential`: the secret goes to the agent; the answer never has it. */
  setCredential(id: string, key: string, secret: string): Promise<PluginCredentialSlot[]>
  /** `deletePluginCredential`. */
  deleteCredential(id: string, key: string): Promise<PluginCredentialSlot[]>
  /**
   * `grantPluginUserOrigin`: consents to exactly the server `origin` that the
   * origin setting holds; `privateNetwork` is the separate consent to reach
   * it on a private network, never assumed.
   */
  grantUserOrigin(
    id: string,
    setting: string,
    origin: string,
    privateNetwork: boolean,
  ): Promise<PluginSettings>
  /** `revokePluginUserOrigin`. */
  revokeUserOrigin(id: string, setting: string): Promise<PluginSettings>
  /** `listModuleMigrations`: the built-in modules moving to their first-party plugins. */
  listModuleMigrations(): Promise<ModuleMigration[]>
  /**
   * `decideModuleMigration`: the consent dialog, item by item (grant keys).
   * Every item approved: the migration goes on; any declined: the built-in stays.
   */
  decideModuleMigration(
    module: string,
    approve: string[],
    decline: string[],
  ): Promise<ModuleMigration>
  /** `retryModuleMigration`: a failed migration runs again; a declined one asks again. */
  retryModuleMigration(module: string): Promise<ModuleMigration>
  /**
   * `list_plugin_navigation`: the toolbar entries of installed, enabled
   * plugins (plugin API 1.3); empty with an agent that has none.
   */
  listNavigation(): Promise<PluginNavigationItem[]>
  /**
   * `get_plugin_navigation_icon`: an entry's icon as a `data:` URL for
   * `<img>` only; `null` when there is none to show.
   */
  getNavigationIcon(
    plugin: string,
    entry: string,
    theme: NavigationIconTheme,
  ): Promise<string | null>
  /** A `.dpplugin` chosen in a native dialog; `null` when cancelled. */
  pickPackage(): Promise<string | null>
  /** Plugin events while subscribed; returns the unsubscribe function. */
  subscribe(listener: (event: PluginEvent) => void): () => void
}

/**
 * Reasons of refusals: the agent's (`deckpilot_ipc::plugins::reason`) and
 * the app's own (`commands::plugins::reason`).
 */
export type PluginErrorReason =
  | "agentNotRunning"
  | "agentTooOld"
  | "communication"
  | "noDialog"
  | "pluginsUnavailable"
  | "platformUnsupported"
  | "locked"
  | "notInstalled"
  | "unknownPlugin"
  | "storeUnavailable"
  | "alreadyInstalled"
  | "consentRequired"
  | "unknownPermission"
  | "registryUnavailable"
  | "revoked"
  | "incompatible"
  | "invalidPackage"
  | "unsupportedRuntime"
  | "invalidState"
  | "notEnabled"
  | "hostFailed"
  | "broken"
  | "busy"
  | "invalidParameters"
  | "invalidData"
  | "unknownMigration"
  | "answerTooLarge"
  | "requiredBy"
  | "dependency"
  | "planChanged"
  | "notBundled"
  | "notPinned"
  | "failed"

/** What each reason means, as a headline; the agent's message has the details. */
const TITLES: Partial<Record<string, TranslationKey>> = {
  agentNotRunning: "plugins:errors.title.agentNotRunning",
  agentTooOld: "plugins:errors.title.agentTooOld",
  communication: "plugins:errors.title.communication",
  noDialog: "plugins:errors.title.noDialog",
  pluginsUnavailable: "plugins:errors.title.pluginsUnavailable",
  platformUnsupported: "plugins:errors.title.platformUnsupported",
  locked: "plugins:errors.title.locked",
  notInstalled: "plugins:errors.title.notInstalled",
  unknownPlugin: "plugins:errors.title.unknownPlugin",
  storeUnavailable: "plugins:errors.title.storeUnavailable",
  alreadyInstalled: "plugins:errors.title.alreadyInstalled",
  consentRequired: "plugins:errors.title.consentRequired",
  unknownPermission: "plugins:errors.title.unknownPermission",
  registryUnavailable: "plugins:errors.title.registryUnavailable",
  revoked: "plugins:errors.title.revoked",
  incompatible: "plugins:errors.title.incompatible",
  invalidPackage: "plugins:errors.title.invalidPackage",
  unsupportedRuntime: "plugins:errors.title.unsupportedRuntime",
  invalidState: "plugins:errors.title.invalidState",
  notEnabled: "plugins:errors.title.notEnabled",
  hostFailed: "plugins:errors.title.hostFailed",
  broken: "plugins:errors.title.broken",
  busy: "plugins:errors.title.busy",
  invalidParameters: "plugins:errors.title.invalidParameters",
  invalidData: "plugins:errors.title.invalidData",
  unknownMigration: "plugins:errors.title.unknownMigration",
  answerTooLarge: "plugins:errors.title.answerTooLarge",
  requiredBy: "plugins:errors.title.requiredBy",
  dependency: "plugins:errors.title.dependency",
  planChanged: "plugins:errors.title.planChanged",
  notBundled: "plugins:errors.title.notBundled",
  notPinned: "plugins:errors.title.notPinned",
  failed: "plugins:errors.title.failed",
}

/** The headline of a refusal's reason; unknown reasons get a generic one. */
export function reasonTitle(reason: string): string {
  return i18n.t(TITLES[reason] ?? "plugins:errors.title.failed")
}

const HINTS: Partial<Record<string, TranslationKey>> = {
  agentNotRunning: "plugins:errors.hint.agentNotRunning",
  agentTooOld: "plugins:errors.hint.agentTooOld",
  pluginsUnavailable: "plugins:errors.hint.pluginsUnavailable",
  platformUnsupported: "plugins:errors.hint.platformUnsupported",
  locked: "plugins:errors.hint.locked",
  storeUnavailable: "plugins:errors.hint.storeUnavailable",
  consentRequired: "plugins:errors.hint.consentRequired",
  unknownPermission: "plugins:errors.hint.unknownPermission",
  registryUnavailable: "plugins:errors.hint.registryUnavailable",
  revoked: "plugins:errors.hint.revoked",
  incompatible: "plugins:errors.hint.incompatible",
  invalidPackage: "plugins:errors.hint.invalidPackage",
  unsupportedRuntime: "plugins:errors.hint.unsupportedRuntime",
  invalidState: "plugins:errors.hint.invalidState",
  notEnabled: "plugins:errors.hint.notEnabled",
  hostFailed: "plugins:errors.hint.hostFailed",
  broken: "plugins:errors.hint.broken",
  alreadyInstalled: "plugins:errors.hint.alreadyInstalled",
  busy: "plugins:errors.hint.busy",
  requiredBy: "plugins:errors.hint.requiredBy",
  dependency: "plugins:errors.hint.dependency",
  planChanged: "plugins:errors.hint.planChanged",
  notBundled: "plugins:errors.hint.notBundled",
  notPinned: "plugins:errors.hint.notPinned",
  answerTooLarge: "plugins:errors.hint.answerTooLarge",
}

/** What the user can do about a refusal's reason, in the window's language. */
function reasonHint(reason: string): string | undefined {
  const key = HINTS[reason]
  return key ? i18n.t(key) : undefined
}

/** A refusal of the marketplace, with what the user can do about it. */
export class PluginError extends Error {
  readonly reason: PluginErrorReason | (string & {})
  readonly hint: string | undefined

  constructor(reason: string, message: string, hint: string | undefined = reasonHint(reason)) {
    super(message)
    this.name = "PluginError"
    this.reason = reason
    this.hint = hint
  }
}

/** The error a Tauri plugin command rejected with (`{ reason, message }`). */
export function toPluginError(error: unknown): PluginError {
  if (error instanceof PluginError) return error
  if (typeof error === "object" && error !== null && "reason" in error) {
    const { reason, message } = error as { reason: unknown; message?: unknown }
    if (typeof reason === "string") {
      return new PluginError(reason, typeof message === "string" ? message : reason)
    }
  }
  if (typeof error === "string") return new PluginError("failed", error)
  if (error instanceof Error) return new PluginError("failed", error.message)
  return new PluginError("failed", i18n.t("plugins:errors.unexpected"))
}

export interface PluginErrorText {
  /** What the reason means, in plain words. */
  title: string
  /** The agent's own explanation. */
  message: string
  /** What the user can do about it. */
  hint?: string
  reason: string
}

export function pluginErrorText(error: unknown): PluginErrorText {
  const e = toPluginError(error)
  const title = reasonTitle(e.reason)
  return e.hint
    ? { title, message: e.message, hint: e.hint, reason: e.reason }
    : { title, message: e.message, reason: e.reason }
}

export const PluginClientContext = createContext<PluginClient | null>(null)

/** The client of the surrounding provider, or `null` for the default. */
export function usePluginClientOverride(): PluginClient | null {
  return useContext(PluginClientContext)
}
