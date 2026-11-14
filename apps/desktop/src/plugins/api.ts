// Typed client of the plugin marketplace.
//
// Mirrors the requests the agent is planned to answer (protocol 10,
// architecture §12.6). None of them is implemented in the agent or as a
// Tauri command yet, so the only implementation is the development fixture
// client in `./fixtures`; it is never live marketplace data.

import { createContext, useContext } from "react"

import type { MarketplaceSnapshot, PluginDetail } from "./types"

export interface InstallRequest {
  id: string
  version: string
  /** Keys of every grant the user consented to (`Grant.key`). */
  consent: string[]
}

export interface PluginClient {
  /**
   * `fixtures`: development data that never comes from the registry; the UI
   * labels every screen with it. `agent`: the planned live backend (not
   * implemented).
   */
  readonly source: "fixtures" | "agent"
  /** `listPlugins`: registry state, catalog, installed plugins and updates. */
  listPlugins(): Promise<MarketplaceSnapshot>
  /** `getPluginDetail`: the signed per-plugin detail with every version. */
  getPluginDetail(id: string): Promise<PluginDetail>
  /** `refreshRegistry`: checks the signed timestamp and index again. */
  refreshRegistry(): Promise<MarketplaceSnapshot>
  /** `installPlugin`: refused unless `consent` covers every grant of the version. */
  installPlugin(request: InstallRequest): Promise<MarketplaceSnapshot>
  /** `approvePluginUpdate`: `consent` must cover every escalated grant. */
  approvePluginUpdate(request: InstallRequest): Promise<MarketplaceSnapshot>
  /** `rollbackPlugin`: swaps the active and the previous version. */
  rollbackPlugin(id: string): Promise<MarketplaceSnapshot>
  /** `uninstallPlugin`: removes files, grants and storage; credentials only if asked. */
  uninstallPlugin(id: string, deleteCredentials: boolean): Promise<MarketplaceSnapshot>
  /**
   * Enables or disables the plugin's module (`setModuleEnabled` for a
   * plugin id). `consent` re-grants revoked grants the user chose to allow.
   */
  setPluginEnabled(id: string, enabled: boolean, consent?: string[]): Promise<MarketplaceSnapshot>
  /** `setPluginGrant`: revokes (or allows again) one grant. */
  setPluginGrant(id: string, grant: string, enabled: boolean): Promise<MarketplaceSnapshot>
}

export type PluginErrorCode =
  | "consent-required"
  | "registry-stale"
  | "registry-unavailable"
  | "verification-failed"
  | "incompatible"
  | "revoked"
  | "yanked"
  | "not-found"
  | "failed"

/** A refusal of the marketplace, with what the user can do about it. */
export class PluginError extends Error {
  readonly code: PluginErrorCode
  readonly hint: string | undefined

  constructor(code: PluginErrorCode, message: string, hint?: string) {
    super(message)
    this.name = "PluginError"
    this.code = code
    this.hint = hint
  }
}

export function pluginErrorText(error: unknown): { message: string; hint?: string } {
  if (error instanceof PluginError) return { message: error.message, hint: error.hint }
  if (error instanceof Error) return { message: error.message }
  return { message: String(error) }
}

export const PluginClientContext = createContext<PluginClient | null>(null)

/** The client of the surrounding provider, or `null` for the default. */
export function usePluginClientOverride(): PluginClient | null {
  return useContext(PluginClientContext)
}
