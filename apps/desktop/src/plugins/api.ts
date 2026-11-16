// Typed client of the plugin marketplace: one method per plugin request of
// IPC protocol 10 (docs/marketplace/agent-integration.md). The only
// implementation in the app is `createTauriPluginClient` (./client), which
// goes through the Tauri commands to the agent. Tests provide their own
// through `PluginClientContext`.

import { createContext, useContext } from "react"

import type {
  ActionParamValue,
  AvailablePlugins,
  KnownPlugin,
  ModuleMigration,
  PluginActionOutcome,
  PluginCredentialSlot,
  PluginDetails,
  PluginEvent,
  PluginInstallSource,
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
  | "failed"

/** What each reason means, as a headline; the agent's message has the details. */
const TITLES: Partial<Record<string, string>> = {
  agentNotRunning: "The agent is not running",
  agentTooOld: "The running agent is too old",
  communication: "DeckPilot could not talk to the agent",
  noDialog: "No file dialog is available",
  pluginsUnavailable: "Plugins are not available in the agent",
  platformUnsupported: "Plugins cannot run on this platform",
  locked: "DeckPilot is locked",
  notInstalled: "The plugin is not installed",
  unknownPlugin: "The registry does not offer this plugin or version",
  storeUnavailable: "The plugin database cannot be opened",
  alreadyInstalled: "The plugin is already installed",
  consentRequired: "Consent is missing",
  unknownPermission: "The plugin's permissions changed",
  registryUnavailable: "The registry is not available",
  revoked: "The version was withdrawn",
  incompatible: "Not compatible with this DeckPilot",
  invalidPackage: "The package was refused",
  unsupportedRuntime: "Unsupported kind of plugin",
  invalidState: "Not possible in the plugin's current state",
  notEnabled: "The plugin is not enabled",
  hostFailed: "The plugin host failed",
  broken: "The plugin's files were modified",
  busy: "The plugin is busy",
  invalidParameters: "Invalid parameters",
  invalidData: "The values do not fit what the plugin declares",
  unknownMigration: "No such module migration",
  answerTooLarge: "The agent's answer is too large",
  failed: "The request failed",
}

/** The headline of a refusal's reason; unknown reasons get a generic one. */
export function reasonTitle(reason: string): string {
  return TITLES[reason] ?? "The request failed"
}

const HINTS: Partial<Record<string, string>> = {
  agentNotRunning: "Start the agent in Settings › Background Agent, then try again.",
  agentTooOld:
    "Restart the agent in Settings › Background Agent so that the agent of this release runs.",
  pluginsUnavailable:
    "Plugins need an agent built with the `plugins` feature and started with --enable-plugins.",
  platformUnsupported:
    "Plugins can be installed and inspected here, but only run where the plugin sandbox is verified: macOS, Linux and Windows.",
  locked: "Unlock DeckPilot, then try again.",
  storeUnavailable:
    "The plugin database cannot be opened with the current key (e.g. after a password change while the agent was locked).",
  consentRequired: "Consent to every permission the plugin declares.",
  unknownPermission: "Reload the plugin; its permissions changed.",
  registryUnavailable: "Check the registry the agent uses, then try again.",
  revoked: "The registry withdrew this version; choose another one or uninstall it.",
  incompatible: "This DeckPilot cannot run it; update DeckPilot or choose another version.",
  invalidPackage: "Only package files whose bytes the verified registry pins can be installed.",
  unsupportedRuntime: "Only WebAssembly plugins (ABI 1) can be installed.",
  invalidState: "Reload the list; the plugin changed in the meantime.",
  notEnabled: "Enable the plugin first.",
  hostFailed: "The plugin host failed or is backing off after crashes; try again later.",
  broken: "Its installed files no longer match their record; roll back or uninstall it.",
  alreadyInstalled: "Update it instead.",
  busy: "Wait for its running actions to finish, then try again.",
  answerTooLarge:
    "Part of it does not fit one message to the app, even in parts (for instance a plugin's very large settings); ask the plugin's publisher to shorten its manifest.",
}

/** A refusal of the marketplace, with what the user can do about it. */
export class PluginError extends Error {
  readonly reason: PluginErrorReason | (string & {})
  readonly hint: string | undefined

  constructor(reason: string, message: string, hint: string | undefined = HINTS[reason]) {
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
  return new PluginError("failed", "Unexpected error")
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
