// Types of the experimental plugin marketplace.
//
// The agent is the source of truth: the shapes below mirror IPC protocol 10
// (`crates/deckpilot-ipc/src/plugins.rs`, serialized in camelCase) as the
// Tauri commands in `src-tauri/src/commands/plugins.rs` return them. The
// manifest permission shapes (plugin-manifest.schema.json) are what a
// registry version lists for consent before it is installed.

import type { ActionType } from "@/lib/types"

// ---- Manifest permissions (plugin-manifest.schema.json) ----

export type HttpMethod = "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"

export interface NetworkPermission {
  kind: "network"
  hosts: string[]
  methods?: HttpMethod[]
  privateNetwork?: boolean
  justification: string
}

export interface CredentialsPermission {
  kind: "credentials"
  refs: string[]
  justification: string
}

export interface NotificationsPermission {
  kind: "notifications"
  justification: string
}

export interface ProfilePermission {
  kind: "profile"
  read: ("profile" | "project")[]
  justification: string
}

export interface VariablesPermission {
  kind: "variables"
  read: string[]
  justification: string
}

export interface StoragePermission {
  kind: "storage"
  quotaKb: number
  justification: string
}

export interface OpenUrlPermission {
  kind: "openUrl"
  hosts: string[]
  justification: string
}

export interface BackgroundPermission {
  kind: "background"
  minIntervalSeconds: number
  justification: string
}

export interface DevicePermission {
  kind: "device"
  state?: boolean
  icons?: boolean
  justification: string
}

/** `$defs/permission` of the manifest schema. */
export type ManifestPermission =
  | NetworkPermission
  | CredentialsPermission
  | NotificationsPermission
  | ProfilePermission
  | VariablesPermission
  | StoragePermission
  | OpenUrlPermission
  | BackgroundPermission
  | DevicePermission

export type PermissionKind = ManifestPermission["kind"]

// ---- Agent shapes (protocol 10) ----

export type RegistryFreshness = "neverVerified" | "fresh" | "stale"

export type RegistryStaleReason = "expired" | "indexExpired" | "indexUnavailable"

export interface RegistryStatus {
  /** Where documents come from: `local:<dir>` or the registry's https URL. */
  source: string
  freshness: RegistryFreshness
  seq?: number
  /** Why the registry is stale (with `freshness: "stale"`). */
  staleReason?: RegistryStaleReason
  /** When the verified timestamp expires (RFC 3339). */
  expires?: string
  /** Why the last refresh could not reach the registry (working offline). */
  offline?: string
}

/** `getPluginRuntime`: whether plugins can be used, and why not. */
export interface PluginRuntimeStatus {
  /** Built, enabled and unlocked: plugin requests are answered. */
  available: boolean
  /** The agent was built with the `plugins` feature. */
  built: boolean
  /** The agent was started with `--enable-plugins`. */
  enabled: boolean
  /** The host sandbox is verified here (macOS, Linux, Windows): plugins may run. */
  executionSupported: boolean
  platform: string
  reason?: string
  registry?: RegistryStatus
}

export type PluginState =
  | "installing"
  | "disabled"
  | "enabled"
  | "updating"
  | "rollingBack"
  | "uninstalling"
  | "broken"
  | "quarantined"

export type PluginOrigin =
  { type: "registry"; registry: string } | { type: "sideload" } | { type: "dev" }

export type PermissionTier = "low" | "medium" | "high"

export type PermissionStatus = "granted" | "revoked" | "notGranted"

/** One declared permission, normalized to a grant by the agent. */
export interface GrantInfo {
  /** The grant key consents name: `network:api.example.com`, `notifications`, … */
  key: string
  kind: string
  tier: PermissionTier
  status: PermissionStatus
  scope: unknown
  justification?: string
}

export interface PluginActionInfo {
  action: string
  /** `<plugin id>.<action>`. */
  actionType: string
  name: string
  description?: string
  available: boolean
  unavailableReason?: string
  /**
   * The action type exactly as the agent's engine registers it (the
   * manifest's parameters and node contract), whether or not it is
   * available now. Absent when the action cannot be offered at all.
   */
  definition?: ActionType
}

export interface PluginSummary {
  id: string
  name: string
  publisher: string
  version: string
  state: PluginState
  stateReason?: string
  origin: PluginOrigin
  previousVersion?: string
  stagedVersion?: string
  actions: PluginActionInfo[]
  /** Host failures the agent saw since it started; absent when none. */
  health?: PluginHealth
}

/** `PluginHealth`: what the agent saw of the plugin's host since it started. */
export interface PluginHealth {
  /** Times the host stopped during an invocation (crash or time limit). */
  crashes: number
  /** The newest failure, as the invocation reported it. */
  lastError?: string
  /** Seconds since the Unix epoch. */
  lastErrorAt?: number
}

export interface PermissionChanges {
  added: GrantInfo[]
  broadened: GrantInfo[]
  /** Grant keys the update no longer declares. */
  removed: string[]
  escalation: boolean
}

export interface PendingPluginUpdate {
  version: string
  changes: PermissionChanges
}

export interface PluginAuditEntry {
  /** Seconds since the Unix epoch. */
  at: number
  version?: string
  op: string
  target?: string
  decision: string
  reason?: string
}

/** `getPlugin` and the answer to every plugin change. */
export interface PluginDetails {
  summary: PluginSummary
  description: string
  license: string
  homepage?: string
  permissions: GrantInfo[]
  pendingUpdate?: PendingPluginUpdate
  /** The active version's manifest.json. */
  manifest: unknown
  audit: PluginAuditEntry[]
  /**
   * The active version's 128 px icon (`data:image/png;base64,…`), checked
   * by the agent against the installed files' integrity record.
   */
  icon?: string
}

export interface AvailableVersion {
  version: string
  /** Manifest form, shown for consent. */
  permissions: ManifestPermission[]
  /** Yanked or revoked: why it cannot be installed. */
  withdrawn?: string
  /** Why this DeckPilot cannot run it; absent when it can. */
  incompatible?: string[]
  /** The DeckPilot versions it needs, e.g. `>=0.5.0 <1.0.0`. */
  deckpilot?: string
  /** `macos`, `windows`, `linux`; absent or empty means every platform. */
  platforms?: string[]
  /** When the registry published it, in seconds since the Unix epoch. */
  publishedAt?: number
}

/** The publisher of a registry plugin, as the registry index lists it. */
export interface PublisherInfo {
  namespace: string
  displayName?: string
  /** GitHub login. */
  login: string
  /** The registry verified who the publisher is. */
  verified: boolean
}

export interface AvailablePlugin {
  id: string
  name: string
  summary: string
  publisher: string
  latest?: string
  /** Newest first. */
  versions: AvailableVersion[]
  /** The installed (active) version. */
  installed?: string
  /** The registry's description, longer than `summary`. */
  description?: string
  categories?: string[]
  publisherInfo?: PublisherInfo
  license?: string
  homepage?: string
  /** `owner/repository` on GitHub. */
  sourceRepository?: string
  /** Why the registry deprecated the plugin. */
  deprecated?: string
}

export interface AvailablePlugins {
  registry: RegistryStatus
  plugins: AvailablePlugin[]
}

export type PluginInstallSource =
  { type: "registry"; plugin: string; version?: string } | { type: "file"; path: string }

export type PluginUpdateOutcome =
  | { type: "applied"; version: string; changes: PermissionChanges }
  | { type: "awaitingConsent"; version: string; changes: PermissionChanges }

export interface PluginUpdateResult {
  plugin: string
  outcome: PluginUpdateOutcome
}

export type ActionParamValue = boolean | number | string | string[]

export type PluginActionOutcome =
  | { status: "ok"; message?: string; data?: Record<string, unknown> }
  | { status: "skipped"; message?: string }
  | { status: "failed"; code: string; message?: string; retryable: boolean }

export type PluginOperation =
  | "install"
  | "enable"
  | "disable"
  | "grant"
  | "revoke"
  | "update"
  | "rollback"
  | "uninstall"
  | "recover"

export type PluginStage =
  "started" | "fetching" | "verifying" | "installing" | "finished" | "failed"

/** A `pluginEvent` forwarded by the app (`plugin-event`). */
export type PluginEvent =
  | {
      type: "progress"
      /** The plugin id, or the package path while a file install has not read it. */
      plugin: string
      operation: PluginOperation
      stage: PluginStage
      message?: string
    }
  | { type: "changed"; plugin: string; state?: PluginState }

// ---- What the screens show ----

/** Whether the marketplace can be used, from `getPluginRuntime` or its refusal. */
export type RuntimeAvailability =
  | { kind: "ready"; status: PluginRuntimeStatus }
  /** Installs and inspection work; enabling and running do not (Windows). */
  | { kind: "platformUnsupported"; status: PluginRuntimeStatus }
  | { kind: "notBuilt"; status: PluginRuntimeStatus }
  | { kind: "notEnabled"; status: PluginRuntimeStatus }
  /** Built and enabled but not answering (e.g. the configuration is locked). */
  | { kind: "unavailable"; status: PluginRuntimeStatus }
  | { kind: "agentNotRunning"; message: string }
  | { kind: "agentTooOld"; message: string }

export interface MarketplaceSnapshot {
  runtime: PluginRuntimeStatus
  /** The registry's verified state; `null` when none is configured. */
  registry: RegistryStatus | null
  /** Why the registry index could not be listed, if it could not. */
  registryError?: { reason: string; message: string }
  available: AvailablePlugin[]
  installed: PluginDetails[]
}
