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
  | { type: "registry"; registry: string }
  | { type: "sideload" }
  | { type: "dev" }
  /** Shipped with DeckPilot, checked against the pins compiled into the agent. */
  | { type: "bundled" }

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
  /** What its storage holds and may hold; absent from older agents. */
  storage?: PluginStorageUsage
}

/** `PluginDetails.storage`: bytes are keys plus JSON values, as UTF-8. */
export interface PluginStorageUsage {
  usedBytes: number
  keys: number
  quotaBytes: number
  maxKeys: number
}

// ---- Plugin data (settings, credentials, known plugins) ----

/** What an uninstall keeps for a later install (default: nothing). */
export interface PluginRetention {
  settings?: boolean
  credentials?: boolean
}

/** A plugin installed here at some point (`plugin_known`). */
export interface KnownPlugin {
  id: string
  name: string
  publisher: string
  namespace: string
  lastVersion: string
  installed: boolean
  firstSeenAt: number
  lastSeenAt: number
}

/** One declared setting (`contributes.settings.fields`). */
export interface PluginSettingField {
  key: string
  label: string
  description?: string
  /** The parameter kind, as in action parameters (`{ type: "integer", min: 1 }`). */
  kind: { type: string } & Record<string, unknown>
  required: boolean
  default?: PluginSettingValue
  portability: "portable" | "machineSpecific" | "sensitive"
}

export type PluginSettingValue = boolean | number | string

/** `getPluginSettings` and `setPluginSettings`. */
export interface PluginSettings {
  plugin: string
  fields: PluginSettingField[]
  /** What the user saved. */
  values: Record<string, PluginSettingValue>
  /** What the plugin sees: saved values, otherwise defaults. */
  effective: Record<string, PluginSettingValue>
  /** Required settings with neither a value nor a default. */
  missing?: string[]
}

/** One credential slot and whether a secret is stored; never the secret. */
export interface PluginCredentialSlot {
  key: string
  label: string
  description?: string
  /** The only hosts the secret is ever sent to. */
  hosts: string[]
  /** `bearer`, `basic`, `header`, `query`. */
  schemes: string[]
  setupUrl?: string
  set: boolean
  /** Seconds since the Unix epoch. */
  updatedAt?: number
  /** Whether its `credentials:<key>` permission is granted. */
  granted: boolean
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
  /** A module migration moved; `listModuleMigrations` has the details. */
  | { type: "migrationChanged"; module: string; plugin: string; phase: MigrationPhase }

// ---- Module migrations (docs/marketplace/module-migration.md §5.5-5.7) ----

/** Where a built-in module's move to its first-party plugin stands. */
export type MigrationPhase =
  | "pending"
  | "notEligible"
  | "installed"
  | "settingsCopied"
  | "awaitingConsent"
  | "cutOver"
  | "failed"
  /** A phase added by a newer agent. */
  | "other"

/** Where the module's legacy action types run now. */
export type MigrationRoute = "builtin" | "plugin" | "pluginWithdrawn" | "other"

export type ConsentDecision = "pending" | "approved" | "declined"

/** A stored credential the migration copies into a plugin slot: names only, never the secret. */
export interface CredentialImport {
  /** The built-in's setting naming it (`credential`). */
  setting: string
  /** The stored credential's id (`github.token`). */
  credential: string
  /** The plugin's slot (`token`). */
  slot: string
  /** The only hosts the plugin may send it to. */
  hosts: string[]
}

/** One thing a migration waits for the user to allow. */
export interface MigrationConsentItem {
  /** The grant key `decideModuleMigration` names. */
  key: string
  permission: GrantInfo
  import?: CredentialImport
  /** Why the migration asks, in the agent's words. */
  why: string
  decision: ConsentDecision
}

export interface ModuleActionAlias {
  /** `github.pull-requests`. */
  legacy: string
  /** `deckpilot.github.pull-requests`. */
  target: string
}

/** `listModuleMigrations`: one built-in module moving to its first-party plugin. */
export interface ModuleMigration {
  module: string
  moduleName: string
  plugin: string
  pluginName?: string
  phase: MigrationPhase
  /** Why it is not eligible, failed or waits; never a value or a secret. */
  reason?: string
  attempts: number
  maxAttempts: number
  canRetry: boolean
  route: MigrationRoute
  /** Why the legacy actions are unavailable (`route: "pluginWithdrawn"`). */
  routeReason?: string
  aliases: ModuleActionAlias[]
  /** What waits for a decision, or what the user declined. */
  consent?: MigrationConsentItem[]
  updatedAt: number
}

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
  /** Every plugin installed here at some point (`listKnownPlugins`); empty when unknown. */
  known?: KnownPlugin[]
  /**
   * The labels of required settings without a value or default, by plugin
   * id (`getPluginSettings.missing`); only plugins with such settings.
   */
  missingSettings?: Record<string, string[]>
}
