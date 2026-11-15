// Typed client of the plugin marketplace: one method per plugin request of
// IPC protocol 10 (docs/marketplace/agent-integration.md). The only
// implementation in the app is `createTauriPluginClient` (./client), which
// goes through the Tauri commands to the agent. Tests provide their own
// through `PluginClientContext`.

import { createContext, useContext } from "react"

import type {
  ActionParamValue,
  AvailablePlugins,
  PluginActionOutcome,
  PluginDetails,
  PluginEvent,
  PluginInstallSource,
  PluginRuntimeStatus,
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
  /** `uninstallPlugin`. */
  uninstallPlugin(id: string): Promise<void>
  /** `invokePluginAction`. */
  invokeAction(
    id: string,
    action: string,
    params: Record<string, ActionParamValue>,
  ): Promise<PluginActionOutcome>
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
  | "invalidParameters"
  | "failed"

const HINTS: Partial<Record<string, string>> = {
  agentNotRunning: "Start the agent in Settings › Background Agent, then try again.",
  agentTooOld:
    "Restart the agent in Settings › Background Agent so that the agent of this release runs.",
  pluginsUnavailable:
    "Plugins need an agent built with the `plugins` feature and started with --enable-plugins.",
  platformUnsupported:
    "Plugins can be installed and inspected here, but only run on macOS and Linux.",
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

export function pluginErrorText(error: unknown): {
  message: string
  hint?: string
  reason: string
} {
  const e = toPluginError(error)
  return e.hint
    ? { message: e.message, hint: e.hint, reason: e.reason }
    : { message: e.message, reason: e.reason }
}

export const PluginClientContext = createContext<PluginClient | null>(null)

/** The client of the surrounding provider, or `null` for the default. */
export function usePluginClientOverride(): PluginClient | null {
  return useContext(PluginClientContext)
}
