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

/** An origin setting of the `network` permission (G2). */
export interface UserOriginSlot {
  setting: string
  methods?: HttpMethod[]
  pathPrefix?: boolean
  privateNetwork?: boolean
}

export interface NetworkPermission {
  kind: "network"
  hosts: string[]
  methods?: HttpMethod[]
  privateNetwork?: boolean
  /** Servers the user enters in a setting; consented when saved. */
  userOrigins?: UserOriginSlot[]
  justification: string
}

export interface CredentialsPermission {
  kind: "credentials"
  refs: string[]
  justification: string
}

export interface NotificationsPermission {
  kind: "notifications"
  /**
   * Where the notifications go (G9): each member names one of the plugin's
   * settings, which the agent reads when it shows one. Without it: the
   * status bar only.
   */
  routing?: { desktop?: string; statusBar?: string; minimumSeverity?: string }
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
  /** Origin settings whose server's pages may be opened too. */
  userOrigins?: string[]
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
  /** Plugins are on (the default with the `plugins` feature; off with `--disable-plugins`). */
  enabled: boolean
  /** The host sandbox is verified here (macOS, Linux, Windows): plugins may run. */
  executionSupported: boolean
  platform: string
  reason?: string
  registry?: RegistryStatus
  /**
   * What the agent can do beyond the original plugin requests, e.g.
   * `pagedAnswers` (the app's backend then reads a plugin's actions and
   * icon in parts, so no plugin is too large to list). Absent from older
   * agents.
   */
  features?: string[]
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
  /**
   * Every action, each with its definition. The app's backend reads them
   * page by page from agents that page plugin answers, so the list is
   * complete whatever its size.
   */
  actions: PluginActionInfo[]
  /** Host failures the agent saw since it started; absent when none. */
  health?: PluginHealth
  /**
   * Toolbar entries of the active version (`contributes.navigation`,
   * plugin API 1.3), whatever the plugin's state; absent when none or
   * from older agents.
   */
  navigation?: PluginNavigationEntry[]
}

/**
 * What a plugin view shows; rendered by the app, never by the plugin.
 * Ignore kinds this app does not know (a later agent may add some).
 */
export type PluginViewKind = "variables" | "actions" | "settings"

/** A host-rendered view (`contributes.views`). */
export interface PluginView {
  id: string
  kind: PluginViewKind | (string & {})
  /**
   * Which of the app's toolbar selectors the view shows (plugin API 1.4;
   * `lib/toolbar-context.ts`). Absent from older agents and manifests:
   * `none`. Only the app's chrome follows it; the plugin gets no data.
   */
  context?: string
}

/** One toolbar entry of a plugin (`PluginNavigationEntry`). */
export interface PluginNavigationEntry {
  /** Unique within the plugin. */
  id: string
  /** At most 24 characters. */
  label: string
  view: PluginView
}

/**
 * `list_plugin_navigation`: an entry of an installed, enabled plugin, in
 * the agent's plugin order and the manifest's entry order.
 */
export interface PluginNavigationItem extends PluginNavigationEntry {
  plugin: string
  pluginName: string
}

/** The theme a navigation icon is shown in. */
export type NavigationIconTheme = "light" | "dark"

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
  /** Origin settings and their consent (absent from older agents). */
  origins?: PluginUserOrigin[]
}

/** Where the consent of an origin setting stands. */
export type UserOriginStatus =
  "unset" | "invalid" | "declared" | "slotNotGranted" | "needsConsent" | "granted"

/** One origin setting (G2): the server the user entered, and its consent. */
export interface PluginUserOrigin {
  setting: string
  label: string
  /** The slot's grant key, `network:user-origin:<setting>`. */
  permission: string
  status: UserOriginStatus
  /** Normalized, with its path prefix; what a consent must name. */
  origin?: string
  problem?: string
  /** The host itself is on a private network: consent needs `privateNetwork`. */
  privateHost?: boolean
  privateNetworkAllowed?: boolean
  privateNetworkGranted?: boolean
  pathPrefixAllowed?: boolean
  methods: string[]
  /** Credential slots whose secrets go there once allowed. */
  credentials?: string[]
  /** Seconds since the Unix epoch. */
  grantedAt?: number
  /** The plugin may also open https pages of this server in the browser. */
  opensPages?: boolean
}

/** One credential slot and whether a secret is stored; never the secret. */
export interface PluginCredentialSlot {
  key: string
  label: string
  description?: string
  /** The only hosts the secret is ever sent to. */
  hosts: string[]
  /** The origin setting whose server (once allowed) also gets it. */
  userOrigin?: string
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
  /** The plugin's origin setting it is bound to: also sent to the server allowed there. */
  userOrigin?: string
}

/**
 * The server a migration asks to allow for the plugin: what the built-in
 * talked to, now in the plugin's origin setting.
 */
export interface MigrationUserOrigin {
  /** The plugin's origin setting that holds it. */
  setting: string
  /** Normalized, with its path prefix. */
  origin: string
  /** This item is the separate consent to reach it on a private network. */
  privateNetwork?: boolean
  methods?: string[]
  /** Credential slots whose secrets go there once allowed. */
  credentials?: string[]
  /** The plugin may also open pages of this server in the browser. */
  opensPages?: boolean
}

/** One thing a migration waits for the user to allow. */
export interface MigrationConsentItem {
  /** What `decideModuleMigration` names: a grant key, or a server's private-network item. */
  key: string
  permission: GrantInfo
  import?: CredentialImport
  /** The server this item allows (or allows on a private network). */
  origin?: MigrationUserOrigin
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
