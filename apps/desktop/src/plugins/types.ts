// Types of the experimental plugin marketplace.
//
// The registry and manifest shapes are hand-written from the design schemas
// (docs/marketplace/schemas/plugin-manifest.schema.json and
// registry-plugin.schema.json / registry-index.schema.json, #191). Field
// names follow those documents exactly, including their snake_case. The
// view types further down describe what the planned agent requests
// (`listPlugins`, `getPluginDetail`, … protocol 10, architecture §12.6)
// would return; none of them exists in the agent yet.

// ---- Manifest (plugin-manifest.schema.json) ----

export type PluginPlatform = "macos" | "windows" | "linux"

export type PluginCategory =
  | "developer-tools"
  | "productivity"
  | "communication"
  | "monitoring"
  | "media"
  | "smart-home"
  | "other"

export type HttpMethod = "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"

/** `engines` of a manifest and of every registry version. */
export interface PluginEngines {
  /** One or two comparators, e.g. ">=0.8.0 <1.0.0". */
  deckpilot: string
  /** Plugin API level, e.g. "^1.0". */
  pluginApi: string
  sdk?: string
}

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
export type PluginPermission =
  | NetworkPermission
  | CredentialsPermission
  | NotificationsPermission
  | ProfilePermission
  | VariablesPermission
  | StoragePermission
  | OpenUrlPermission
  | BackgroundPermission
  | DevicePermission

export type PermissionKind = PluginPermission["kind"]

// ---- Registry (registry-index / registry-plugin schemas) ----

export interface RegistryPublisher {
  namespace: string
  github_id: number
  login: string
  display_name?: string
  verified: boolean
}

export interface RegistryIndexEntry {
  id: string
  publisher: string
  name: string
  summary: string
  categories?: PluginCategory[]
  latest: { stable?: string; prerelease?: string }
  deprecated?: { at: string; reason: string; replacement?: string }
  detail: { path: string; sha256: string; length: number }
}

export interface RegistryRevocation {
  id: string
  versions: "*" | string[]
  action: "yank" | "disable"
  reason: string
  advisory?: string
  at: string
}

/** `$defs/version` of registry-plugin.schema.json. */
export interface RegistryPluginVersion {
  version: string
  published_at: string
  engines: PluginEngines
  /** Omitted means all platforms. */
  platforms?: PluginPlatform[]
  permissions: PluginPermission[]
  manifest_sha256: string
  source: {
    repository: string
    tag: string
    commit: string
    path?: string
    archive?: string
  }
  package: { url: string; sha256: string; size: number }
  builder: string
  attestation?: string
  yanked?: { at: string; reason: string }
}

/** registry-plugin.schema.json: `plugins/<id>/<sha256>.json`. */
export interface RegistryPlugin {
  _type: "deckpilot-registry-plugin"
  spec_version: 1
  id: string
  publisher: string
  name: string
  description: string
  license: string
  homepage?: string
  source_repository?: string
  versions: RegistryPluginVersion[]
}

// ---- What the planned agent requests return ----

/** This DeckPilot, as the agent reports it. */
export interface PluginEnvironment {
  appVersion: string
  platform: PluginPlatform
  /** Plugin API level this build provides, e.g. "1.0". */
  pluginApi: string
  /** False where no OS sandbox layer is proven (architecture §16.9). */
  pluginsSupported: boolean
}

/**
 * Trust state of the local registry copy (architecture §9.3, §14).
 *
 * - `fresh`: the signed timestamp is valid; browsing, installs and updates work.
 * - `offline`: the last check failed for lack of network; cached data is shown.
 * - `stale`: the timestamp expired; nothing can be installed or updated.
 * - `verification-failed`: signatures or hashes did not verify; nothing from
 *   that fetch is used.
 * - `never-reached`: no verified copy exists on this machine.
 */
export type RegistryState = "fresh" | "offline" | "stale" | "verification-failed" | "never-reached"

export interface RegistryStatus {
  state: RegistryState
  /** ISO time of the last attempt to reach the registry. */
  checkedAt?: string
  /** ISO time of the last verified timestamp. */
  verifiedAt?: string
  /** Why the last check failed, in plain words. */
  message?: string
}

export interface CatalogEntry {
  entry: RegistryIndexEntry
  publisher: RegistryPublisher | null
}

/** One normalized grant (architecture §5.4) as stored with its consent. */
export interface StoredGrant {
  key: string
  /** The permission entry the grant came from, for display and comparison. */
  permission: PluginPermission
  /** False once the user revoked it. */
  enabled: boolean
  grantedAt: string
  grantedForVersion: string
  source: "install" | "update" | "user"
}

/** Problems of an installed plugin, with what the user can do about them. */
export type InstalledProblem =
  | { kind: "broken"; code: string; message: string; hint: string }
  | { kind: "revoked"; reason: string; advisory?: string }
  | { kind: "yanked"; reason: string }
  | { kind: "deprecated"; reason: string; replacement?: string }
  | { kind: "incompatible"; reason: string }

export interface InstalledPlugin {
  id: string
  name: string
  publisher: string
  version: string
  /** `dev`: an unpacked development folder (architecture §15.4). */
  source: "registry" | "dev"
  enabled: boolean
  /** Permissions in the installed version's manifest. */
  permissions: PluginPermission[]
  grants: StoredGrant[]
  installedAt: string
  /** The version kept for rollback, if any. */
  previousVersion?: string
  problems: InstalledProblem[]
}

/** A newer version the agent would offer, with that version's permissions. */
export interface PluginUpdate {
  id: string
  from: string
  to: string
  publishedAt: string
  permissions: PluginPermission[]
}

export interface MarketplaceSnapshot {
  environment: PluginEnvironment
  registry: RegistryStatus
  catalog: CatalogEntry[]
  installed: InstalledPlugin[]
  updates: PluginUpdate[]
}

export interface PluginDetail {
  plugin: RegistryPlugin
  entry: RegistryIndexEntry
  publisher: RegistryPublisher | null
  /** Revocations of the newest verified index that name this plugin. */
  revocations: RegistryRevocation[]
}
