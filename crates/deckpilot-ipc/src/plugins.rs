//! Plugin shapes of protocol 10 (capability [`Capability::Plugins`]).
//!
//! The configuration app manages plugins with these requests:
//!
//! | Request | Answer | Does |
//! | --- | --- | --- |
//! | `getPluginRuntime` | `pluginRuntime` | whether plugins can be used here, and why not |
//! | `listPlugins` | `plugins` | installed plugins |
//! | `getPlugin` | `plugin` | one installed plugin: manifest, permissions with their grant state, actions, pending update, recent audit |
//! | `listPluginsPaged` / `getPluginPaged` | `plugins` / `plugin` | the same without actions and icon (feature [`feature::PAGED_ANSWERS`]) |
//! | `getPluginActions` | `pluginActions` | one page of a plugin's actions with their definitions (feature [`feature::PAGED_ANSWERS`]) |
//! | `getPluginIcon` | `pluginIcon` | one chunk of a plugin's icon (feature [`feature::PAGED_ANSWERS`]) |
//! | `listAvailablePlugins` | `availablePlugins` | the verified registry index with per-version permissions and compatibility (`refresh` re-reads the registry first) |
//! | `installPlugin` | `plugin` | installs from the registry or from a package file (verified against the registry pins), in state `disabled`, granting `consent` |
//! | `enablePlugin` | `plugin` | grants `consent`, then enables; every declared permission must be granted |
//! | `disablePlugin` | `plugin` | stops its host and disables it |
//! | `grantPluginPermissions` / `revokePluginPermission` | `plugin` | changes one plugin's grants; effective on the next capability call |
//! | `updatePlugin` | `pluginUpdate` | installs a newer version; an escalation stays staged unless `consent` covers it |
//! | `approvePluginUpdate` / `declinePluginUpdate` | `pluginUpdate` / `plugin` | decides a staged update |
//! | `rollbackPlugin` | `plugin` | returns to the kept previous version |
//! | `uninstallPlugin` | `ok` | removes files, records, grants and storage; settings and credentials too unless `keep` says otherwise |
//! | `invokePluginAction` | `pluginActionResult` | runs one action out of process (for testing and the UI); parameters are validated against the manifest first |
//! | `listKnownPlugins` | `knownPlugins` | every plugin installed here at some point (`plugin_known`) |
//! | `getPluginSettings` / `setPluginSettings` | `pluginSettings` | a plugin's settings fields and values, validated against the manifest; origin settings with their consent state ([`PluginUserOrigin`]) |
//! | `grantPluginUserOrigin` / `revokePluginUserOrigin` | `pluginSettings` | consents to (or revokes) the exact origin the user entered in an origin setting, a private network only when asked for explicitly (G2) |
//! | `listPluginCredentials` / `setPluginCredential` / `deletePluginCredential` | `pluginCredentials` | a plugin's credential slots and whether each has a secret; a secret goes in, never out ([`PluginSecret`]) |
//! | `listModuleMigrations` | `moduleMigrations` | the built-in modules moving to their first-party plugins: phase, reason, attempts, routing, aliases and what waits for consent ([`ModuleMigrationInfo`]) |
//! | `decideModuleMigration` | `moduleMigration` | approves or declines, item by item, what a migration waits for |
//! | `retryModuleMigration` | `moduleMigration` | runs a failed (or declined) migration again |
//!
//! Progress and state changes arrive as `pluginEvent` messages
//! ([`PluginEvent`]) on connections that sent `subscribePluginEvents`
//! (`AgentClient::subscribe_plugin_events` and
//! `AgentClient::next_plugin_event`); changes of a module migration
//! ([`PluginEvent::MigrationChanged`]) only on connections that asked for
//! them (`subscribePluginEvents` with `migrations: true`), so an app that
//! predates them never receives an event it cannot read. Permissions are named by their grant key
//! ([`PluginPermission::key`]): `network:api.example.com`,
//! `notifications`, `credentials:apiToken`, … The agent answers plugin
//! requests only when it was built with plugins and they are enabled;
//! otherwise every request but `getPluginRuntime` fails with
//! [`ErrorCode::Unsupported`](crate::ErrorCode::Unsupported) and the reason
//! [`reason::PLUGINS_UNAVAILABLE`]. Errors carry a machine-readable
//! [`IpcError::reason`](crate::IpcError::reason) from [`reason`].
//!
//! An answer must fit one IPC message
//! ([`MAX_MESSAGE_BYTES`](crate::framing::MAX_MESSAGE_BYTES)); a plugin's
//! actions (each with its full definition) and its icon do not have to. An
//! agent with [`feature::PAGED_ANSWERS`] in its runtime status answers
//! `listPluginsPaged` and `getPluginPaged` without them, pages the actions
//! with `getPluginActions` and chunks the icon with `getPluginIcon`
//! ([`crate::plugin_answers`] has both sides). An answer that does not fit
//! anyway (a client that does not page, such as an older app) is sent
//! shortened (icon, then definitions, then the manifest's actions left
//! out), and otherwise as an error with the reason
//! [`reason::ANSWER_TOO_LARGE`]; the connection stays open.
//!
//! Every value is checked by the agent: identifiers are bounded and
//! validated there. Only `setPluginCredential` carries a secret, from the
//! app to the agent; no answer or event ever does.
//!
//! [`Capability::Plugins`]: crate::Capability::Plugins

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Most permissions one consent may list.
pub const MAX_CONSENT_KEYS: usize = 64;
/// Most parameters one `invokePluginAction` may pass.
pub const MAX_ACTION_PARAMS: usize = 64;
/// Longest plugin id, action id, version or permission key in a request.
pub const MAX_PLUGIN_TEXT: usize = 256;
/// Longest package file path.
pub const MAX_PACKAGE_PATH: usize = 4096;
/// Largest icon file [`PluginDetails::icon`] carries.
pub const MAX_ICON_BYTES: usize = 256 * 1024;
/// Icon bytes one `getPluginIcon` answer carries at most: a multiple of 3,
/// so the base64 of consecutive chunks joins into the base64 of the file,
/// and small enough that its base64 fits one IPC message.
pub const ICON_CHUNK_BYTES: usize = 30 * 1024;
/// Highest action index a `getPluginActions` may start from (manifests
/// declare at most 64 actions).
pub const MAX_ACTION_INDEX: u32 = 1024;

/// What an agent can do beyond the plugin requests every protocol 10 agent
/// has ([`PluginRuntimeStatus::features`]). A client uses a feature only
/// when the agent lists it.
pub mod feature {
    /// `listPluginsPaged`, `getPluginPaged`, `getPluginActions` and
    /// `getPluginIcon`: answers that fit one IPC message whatever the
    /// plugin's number of actions and outputs and its icon's size.
    pub const PAGED_ANSWERS: &str = "pagedAnswers";
    /// Plugin API 1.3 navigation: [`PluginSummary::navigation`](super::PluginSummary::navigation)
    /// and `getPluginIcon` with `navigation`
    /// ([`NavigationIconRef`](super::NavigationIconRef)). An agent without
    /// it ignores `navigation` and answers with the plugin's own icon, so a
    /// client asks for a navigation icon only when the agent lists it.
    pub const NAVIGATION: &str = "navigation";
}

/// One toolbar entry of an installed plugin (`contributes.navigation`,
/// plugin API 1.3). It opens a view the app renders itself; the plugin
/// ships no UI code. Its icon is read with `getPluginIcon` and
/// [`NavigationIconRef`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginNavigationEntry {
    /// Unique within the plugin.
    pub id: String,
    /// At most 24 characters.
    pub label: String,
    pub view: PluginView,
}

/// A host-rendered view (`contributes.views`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginView {
    pub id: String,
    /// `variables` (the plugin's variables, live), `actions` (the Action
    /// Library filtered to the plugin) or `settings` (its configuration
    /// page). A string, so that a client ignores kinds a later agent adds
    /// rather than failing on them.
    pub kind: String,
    /// Which of the app's toolbar selectors the view shows (plugin API
    /// 1.4): `none`, `device`, `profile` or `device+profile`. Left out for
    /// `none` and by older agents; a client treats absent and unknown
    /// values as `none`. Only the app's chrome follows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// The theme a navigation icon is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NavigationIconTheme {
    Light,
    Dark,
}

/// Which navigation icon a `getPluginIcon` asks for (feature
/// [`feature::NAVIGATION`]): the entry's file for the theme, else its file
/// for the other theme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationIconRef {
    /// [`PluginNavigationEntry::id`].
    pub entry: String,
    pub theme: NavigationIconTheme,
}

/// Media types of navigation icons; the plugin's own icon is always
/// `image/png`.
pub const NAVIGATION_ICON_MEDIA_TYPES: [&str; 3] = ["image/png", "image/webp", "image/svg+xml"];

/// Stable, machine-readable reasons of plugin errors
/// ([`IpcError::reason`](crate::IpcError::reason)). The message explains
/// the same in words.
pub mod reason {
    /// Not built with plugins, or plugins are not enabled.
    pub const PLUGINS_UNAVAILABLE: &str = "pluginsUnavailable";
    /// The host sandbox of this platform is not verified: plugins can be
    /// installed but not enabled or run.
    pub const PLATFORM_UNSUPPORTED: &str = "platformUnsupported";
    /// The configuration is locked; so is the plugin database.
    pub const LOCKED: &str = "locked";
    pub const NOT_INSTALLED: &str = "notInstalled";
    /// The registry does not offer the plugin or version.
    pub const UNKNOWN_PLUGIN: &str = "unknownPlugin";
    /// `plugins.db` cannot be opened (e.g. it is under another key).
    pub const STORE_UNAVAILABLE: &str = "storeUnavailable";
    pub const ALREADY_INSTALLED: &str = "alreadyInstalled";
    /// A declared permission was not granted.
    pub const CONSENT_REQUIRED: &str = "consentRequired";
    /// A consent named a permission the plugin does not declare.
    pub const UNKNOWN_PERMISSION: &str = "unknownPermission";
    /// The registry is unreachable, never verified, stale, or a document
    /// failed verification (signature, hash, rollback).
    pub const REGISTRY: &str = "registryUnavailable";
    /// The registry revoked or yanked the version.
    pub const REVOKED: &str = "revoked";
    /// The plugin needs another DeckPilot, plugin API or platform.
    pub const INCOMPATIBLE: &str = "incompatible";
    /// The package does not match its pins or breaks the package rules.
    pub const INVALID_PACKAGE: &str = "invalidPackage";
    /// The package is not an ABI 1 WebAssembly plugin (e.g. `plugin.js`).
    pub const UNSUPPORTED_RUNTIME: &str = "unsupportedRuntime";
    /// The operation is not possible in the plugin's state.
    pub const INVALID_STATE: &str = "invalidState";
    /// The plugin is not enabled (or its action not available).
    pub const NOT_ENABLED: &str = "notEnabled";
    /// The plugin host failed: crashed, timed out, refused the module, or
    /// is backing off after crashes.
    pub const HOST_FAILED: &str = "hostFailed";
    /// The installed files no longer match their record.
    pub const BROKEN: &str = "broken";
    /// Too many invocations of the plugin are already waiting for its host.
    pub const BUSY: &str = "busy";
    /// Settings values or a credential that the plugin's manifest does not
    /// declare, or that do not fit their declaration.
    pub const INVALID_DATA: &str = "invalidData";
    /// No migration of that module is known.
    pub const UNKNOWN_MIGRATION: &str = "unknownMigration";
    /// The answer does not fit one IPC message, even shortened. Sent with
    /// [`ErrorCode::Failed`](crate::ErrorCode::Failed), which every client
    /// knows; the connection stays open.
    pub const ANSWER_TOO_LARGE: &str = "answerTooLarge";
}

/// Most consent items one `decideModuleMigration` may decide.
pub const MAX_MIGRATION_ITEMS: usize = 64;
/// Longest module id in a request.
pub const MAX_MODULE_ID: usize = 64;

/// Longest credential secret a request may carry.
pub const MAX_PLUGIN_SECRET: usize = 16 * 1024;
/// Most settings values one `setPluginSettings` may pass.
pub const MAX_SETTINGS_VALUES: usize = 64;
/// Longest origin one `grantPluginUserOrigin` may name.
pub const MAX_USER_ORIGIN: usize = 2048;

/// Whether plugins can be used, and why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts the app shows: built, enabled, runnable here"
)]
pub struct PluginRuntimeStatus {
    /// Built with plugins, enabled, and the configuration is unlocked:
    /// plugin requests are answered.
    pub available: bool,
    /// The agent was built with the `plugins` feature.
    pub built: bool,
    /// Plugins are enabled (`--enable-plugins`).
    pub enabled: bool,
    /// This platform's host sandbox is verified, so plugins may be enabled
    /// and run (macOS, Linux). Elsewhere they can be installed only.
    pub execution_supported: bool,
    /// `macos`, `linux` or `windows`.
    pub platform: String,
    /// Why not available, or why plugins cannot run here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The registry the agent installs from, once one is configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryStatus>,
    /// What this agent can do beyond the original plugin requests
    /// ([`feature`]). Additive: older agents leave it out, older clients
    /// ignore it, and a client ignores names it does not know.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}

impl PluginRuntimeStatus {
    /// Whether the agent lists `feature` ([`feature`]).
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

/// The registry's verified state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryStatus {
    /// Where documents come from: `local:<dir>`, or the registry's https
    /// URL.
    pub source: String,
    pub freshness: RegistryFreshness,
    /// Sequence number of the verified index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// Why the registry is stale (with `freshness: stale`). Additive:
    /// older agents leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<RegistryStaleReason>,
    /// When the verified timestamp expires (RFC 3339): after it, without
    /// a successful refresh, the registry is stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    /// Set when the last refresh could not reach the registry: why. The
    /// verified state above is then the stored one (offline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline: Option<String>,
}

/// Why a verified registry may no longer be used for installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegistryStaleReason {
    /// The verified timestamp expired (no successful refresh in time, or
    /// a frozen registry).
    Expired,
    /// The index expired (only where index expiry is enforced).
    IndexExpired,
    /// The verified index is not cached any more.
    IndexUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegistryFreshness {
    /// Never verified: nothing can be installed.
    NeverVerified,
    /// Installs and updates are possible.
    Fresh,
    /// The verified state expired or its index is not available:
    /// installed plugins run, nothing new is installed.
    Stale,
}

/// The lifecycle state of an installed plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginState {
    Installing,
    Disabled,
    Enabled,
    Updating,
    RollingBack,
    Uninstalling,
    /// The installed files do not match their record.
    Broken,
    /// The registry revoked the installed version.
    Quarantined,
}

/// Where an installed plugin came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginOrigin {
    Registry {
        registry: String,
    },
    Sideload,
    Dev,
    /// Shipped with DeckPilot: a bundled first-party plugin, verified
    /// against the pins compiled into the agent. Agents before the module
    /// migration UI reported these as `sideload` (no release bundles one
    /// yet).
    Bundled,
}

/// Consent tier of a permission (design section 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionTier {
    Low,
    Medium,
    High,
}

/// Whether a declared permission is granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionStatus {
    Granted,
    Revoked,
    /// Declared, never decided.
    NotGranted,
}

/// One declared permission, normalized to a grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPermission {
    /// The grant key used in consents: `kind` or `kind:key`, e.g.
    /// `network:api.open-meteo.com`.
    pub key: String,
    /// The permission kind (`network`, `notifications`, `credentials`, …).
    pub kind: String,
    pub tier: PermissionTier,
    pub status: PermissionStatus,
    /// What the grant allows beyond its key (methods, hosts, quota…).
    pub scope: serde_json::Value,
    /// The manifest's justification, shown for consent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
}

/// One action a plugin contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginActionInfo {
    /// The action's id in the plugin (`greet`).
    pub action: String,
    /// The action type id in DeckPilot: `<plugin id>.<action>`.
    pub action_type: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Runnable now: the plugin is enabled and plugins can run here.
    pub available: bool,
    /// Why not, when not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    /// The action type exactly as the agent's engine registers it: the
    /// manifest's parameters and node contract, converted one to one
    /// (`cancellable` always, never `changesContext`). The configuration
    /// app offers it in its action editors and validates instances
    /// against it. Present whether or not the action is available now;
    /// absent when the manifest action does not convert (the engine does
    /// not offer it either) and from agents that predate the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<deckpilot_core::ActionType>,
}

/// An installed plugin, as listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSummary {
    /// `<publisher>.<name>`.
    pub id: String,
    pub name: String,
    pub publisher: String,
    /// The active version.
    pub version: String,
    pub state: PluginState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    pub origin: PluginOrigin,
    /// The version a rollback returns to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    /// A newer version waiting for consent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_version: Option<String>,
    /// Empty in a paged answer (`listPluginsPaged`, `getPluginPaged`),
    /// which has [`PluginSummary::action_count`] instead.
    pub actions: Vec<PluginActionInfo>,
    /// In a paged answer only: how many actions the plugin has, which
    /// `getPluginActions` pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_count: Option<u32>,
    /// What the agent saw of the plugin's host since it started: absent
    /// until an invocation failed, and from agents before the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<PluginHealth>,
    /// The active version's toolbar entries (`contributes.navigation`,
    /// plugin API 1.3), in manifest order, whatever the plugin's state:
    /// the app shows them for enabled plugins only. Empty (left out) from
    /// agents without [`feature::NAVIGATION`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub navigation: Vec<PluginNavigationEntry>,
}

/// Failures of a plugin's host the agent saw since it started (kept in
/// memory, never stored). The plugin's lifecycle state and its reason are
/// [`PluginSummary::state`] and [`PluginSummary::state_reason`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginHealth {
    /// Times the host stopped during an invocation (crashed or was killed
    /// after its time limit).
    pub crashes: u32,
    /// The newest failure, worded as the invocation reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// When it happened, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_at: Option<i64>,
}

/// The permission changes of an update (design section 13.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionChanges {
    /// New grants the update asks for.
    pub added: Vec<PluginPermission>,
    /// Grants the update widens (more methods, hosts…).
    pub broadened: Vec<PluginPermission>,
    /// Grant keys the update no longer declares (dropped on update).
    pub removed: Vec<String>,
    /// Whether consent is needed (`added` or `broadened` is not empty).
    pub escalation: bool,
}

/// A staged update waiting for consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPluginUpdate {
    pub version: String,
    pub changes: PermissionChanges,
}

/// One audit record (decisions only, never values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAuditEntry {
    /// Seconds since the Unix epoch.
    pub at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One installed plugin in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDetails {
    pub summary: PluginSummary,
    pub description: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Every declared permission of the active version with its state.
    pub permissions: Vec<PluginPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_update: Option<PendingPluginUpdate>,
    /// The active version's `manifest.json` (parameters, outputs, icons…).
    pub manifest: serde_json::Value,
    /// The newest audit records, newest first.
    pub audit: Vec<PluginAuditEntry>,
    /// The active version's 128 px icon as a `data:image/png;base64,` URL,
    /// when the installed file is a PNG of at most [`MAX_ICON_BYTES`] that
    /// matches its integrity record. Absent from agents before the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// What the plugin's storage holds and may hold. Absent from agents
    /// before the field, and when plugins.db cannot be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<PluginStorageUsage>,
}

/// One action of a `getPluginActions` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginActionEntry {
    /// As in a full [`PluginSummary::actions`], definition included.
    pub action: PluginActionInfo,
    /// The action's entry in the manifest (`contributes.actions[i]`), which
    /// a paged [`PluginDetails::manifest`] leaves out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
    /// The action alone does not fit one message: its definition and
    /// manifest entry are left out (the editors then do not offer it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub too_large: bool,
}

/// One chunk of a plugin's icon (`getPluginIcon`): the active version's
/// 128 px PNG, or with `navigation` a navigation entry's PNG, WebP or
/// (sanitized) SVG, checked against its integrity record on every request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginIconChunk {
    /// `image/png`; for a navigation icon one of
    /// [`NAVIGATION_ICON_MEDIA_TYPES`].
    pub media_type: String,
    /// The whole file's size in bytes (of a sanitized SVG: of the bytes
    /// sent).
    pub size: u64,
    /// The whole file's sha256 (lowercase hex; of a sanitized SVG: of the
    /// bytes sent): the same in every chunk of one file, so a client sees
    /// that it changed between chunks.
    pub sha256: String,
    /// Where this chunk starts in the file.
    pub offset: u64,
    /// The chunk's bytes, base64 (standard alphabet, padded).
    pub data: String,
    /// The offset of the next chunk; absent from the last one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u64>,
}

/// A plugin's storage (`storage.*`): what it holds and its quota, the
/// default or the one its `storage` grant raises it to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStorageUsage {
    /// Bytes of keys and JSON values, as UTF-8.
    pub used_bytes: u64,
    pub keys: u64,
    pub quota_bytes: u64,
    pub max_keys: u64,
}

/// One version a registry offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableVersion {
    pub version: String,
    /// The permissions shown for consent (manifest form).
    pub permissions: Vec<serde_json::Value>,
    /// Yanked or revoked: not installable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawn: Option<String>,
    /// Why this DeckPilot cannot run it; empty when it can.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incompatible: Vec<String>,
    /// The DeckPilot versions it needs (a range such as `>=0.5.0 <1.0.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deckpilot: Option<String>,
    /// The platforms it runs on (`macos`, `windows`, `linux`); empty means
    /// every platform.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    /// When the registry published it, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,
}

/// The publisher of a registry plugin, as the registry index lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublisherInfo {
    /// The namespace (the first part of its plugin ids).
    pub namespace: String,
    /// The name the registry shows, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The publisher's GitHub login.
    pub login: String,
    /// The registry verified who the publisher is.
    pub verified: bool,
}

/// One plugin a registry offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailablePlugin {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub publisher: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// Newest first.
    pub versions: Vec<AvailableVersion>,
    /// The installed (active) version, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<String>,
    /// The registry's description (longer than `summary`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The registry's categories.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// The publisher as the registry index lists it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_info: Option<PublisherInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// `owner/repository` on GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repository: Option<String>,
    /// Why the registry deprecated the plugin, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
}

/// What to install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginInstallSource {
    /// A plugin of the registry: `version`, or its latest stable version.
    Registry {
        plugin: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// A `.dpplugin` on this computer. It is installed only if its bytes
    /// equal a version the verified registry pins.
    File { path: String },
}

/// The result of `updatePlugin` and `approvePluginUpdate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginUpdateOutcome {
    /// The new version is active.
    Applied {
        version: String,
        changes: PermissionChanges,
    },
    /// The new version asks for more and is staged until
    /// `approvePluginUpdate` (or `declinePluginUpdate`).
    AwaitingConsent {
        version: String,
        changes: PermissionChanges,
    },
}

/// What one plugin action invocation returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PluginActionOutcome {
    Ok {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        /// The action's outputs (only those its manifest declares).
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        data: BTreeMap<String, serde_json::Value>,
    },
    Skipped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Failed {
        /// A shared code (`not-permitted`, `invalid-parameters`, …) or one
        /// the plugin declares.
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default)]
        retryable: bool,
    },
}

/// Which lifecycle operation an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginOperation {
    Install,
    Enable,
    Disable,
    Grant,
    Revoke,
    Update,
    Rollback,
    Uninstall,
    Recover,
}

/// How far an operation got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginStage {
    Started,
    /// Reading the package (registry or file).
    Fetching,
    /// Checking signatures, pins, manifest and module.
    Verifying,
    /// Unpacking and committing.
    Installing,
    Finished,
    Failed,
}

/// What a `pluginEvent` message carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginEvent {
    /// A plugin operation moved on (`plugin` is the plugin id, or the
    /// package path while a file install has not read it yet).
    Progress {
        plugin: String,
        operation: PluginOperation,
        stage: PluginStage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// A plugin was installed, changed state or grants, or (`state: None`)
    /// was uninstalled.
    Changed {
        plugin: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<PluginState>,
    },
    /// A module migration moved: its phase, reason, attempts, route or what
    /// waits for consent changed; `listModuleMigrations` has the details.
    /// Only on connections that subscribed with `migrations: true`.
    MigrationChanged {
        /// The built-in module (`github`).
        module: String,
        /// Its plugin (`deckpilot.github`).
        plugin: String,
        phase: MigrationPhase,
    },
}

impl PluginEvent {
    /// Whether a connection receives the event: migration events only when
    /// it asked for them (`subscribePluginEvents` with `migrations: true`).
    pub fn delivered(&self, migrations: bool) -> bool {
        migrations || !matches!(self, Self::MigrationChanged { .. })
    }
}

/// Where a built-in module's move to its first-party plugin stands
/// (`module_migrations.phase`, `docs/marketplace/module-migration.md`
/// §5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationPhase {
    /// Recorded, nothing done yet.
    Pending,
    /// The user's data or this computer does not allow it, or the user
    /// declined (see the reason); the built-in stays.
    NotEligible,
    /// The plugin is installed (disabled), with the derived grants.
    Installed,
    /// The built-in's settings were copied.
    SettingsCopied,
    /// Waits for the user's consent ([`ModuleMigrationInfo::consent`]);
    /// the built-in serves meanwhile.
    AwaitingConsent,
    /// The plugin serves the module's actions.
    CutOver,
    /// A step failed (see the reason); the built-in serves. Retried at the
    /// next starts up to [`ModuleMigrationInfo::max_attempts`] times, then
    /// only on Retry.
    Failed,
    /// A phase added by a newer agent.
    #[serde(other)]
    Other,
}

/// Where the module's legacy action types run now (design §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationRoute {
    /// The built-in handles them.
    Builtin,
    /// The plugin handles them (the built-in takes over if the plugin
    /// breaks).
    Plugin,
    /// The user disabled or uninstalled the plugin after the cut-over:
    /// they are unavailable, with no fallback
    /// ([`ModuleMigrationInfo::route_reason`] says why).
    PluginWithdrawn,
    /// A route added by a newer agent.
    #[serde(other)]
    Other,
}

/// A legacy action type of the module and the plugin action type that
/// replaces it (`ActionAlias`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleActionAlias {
    /// `github.pull-requests`.
    pub legacy: String,
    /// `deckpilot.github.pull-requests`.
    pub target: String,
}

/// The user's decision on one consent item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConsentDecision {
    /// Not decided yet.
    Pending,
    Approved,
    /// The built-in stays until the user asks again (Retry).
    Declined,
}

/// A stored credential the migration copies into the plugin's slot. Names
/// only: the secret never leaves the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialImport {
    /// The built-in's setting that names the credential (`credential`).
    pub setting: String,
    /// The stored credential's id (`github.token`), never its secret.
    pub credential: String,
    /// The plugin's slot it is copied into (`token`).
    pub slot: String,
    /// The only hosts the plugin may send it to.
    pub hosts: Vec<String>,
    /// The plugin's origin setting the slot is bound to: it is also sent
    /// to the server the user entered there and allowed, and nowhere else.
    /// Additive: older agents leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_origin: Option<String>,
}

/// The server a migration asks the user to allow for the plugin: what the
/// built-in talked to (a self-managed GitLab, a Jira site), now in the
/// plugin's origin setting. Never granted without the user's decision on
/// this item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationUserOrigin {
    /// The plugin's origin setting that holds it.
    pub setting: String,
    /// The server, normalized, with its path prefix
    /// (`https://code.example.com/gitlab/api/v4`).
    pub origin: String,
    /// This item is the separate consent to reach that server on a
    /// private network (its key ends in `:private-network`); the server
    /// itself is another item.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private_network: bool,
    /// The methods requests to it may use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    /// The credential slots whose secrets are sent there once allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<String>,
    /// The plugin may also open pages of this server in the browser.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub opens_pages: bool,
}

/// One thing a migration waits for the user to allow: a permission that
/// is never derived (a credential, the private network, a medium-tier
/// permission the built-in did not use here) and, with a credential
/// permission, the import of the stored credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationConsentItem {
    /// The grant key it decides (`credentials:token`,
    /// `network:api.example.com`); what `decideModuleMigration` names.
    pub key: String,
    /// The permission as `getPlugin` shows it: tier, scope, the
    /// manifest's justification.
    pub permission: PluginPermission,
    /// The stored credential copied into the slot on approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<CredentialImport>,
    /// The server the built-in talked to, which approving this item
    /// allows for the plugin (with `network:user-origin:<setting>`), or
    /// allows on a private network (the item whose key ends in
    /// `:private-network`, which is no grant of its own). Additive: older
    /// agents leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<MigrationUserOrigin>,
    /// Why the migration asks, in words.
    pub why: String,
    pub decision: ConsentDecision,
}

/// One built-in module's move to its first-party plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleMigrationInfo {
    /// The built-in module (`github`).
    pub module: String,
    /// Its name (`GitHub`), or the id when the built-in is not part of
    /// this app.
    pub module_name: String,
    /// The plugin (`deckpilot.github`).
    pub plugin: String,
    /// The plugin's name, once it is installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    pub phase: MigrationPhase,
    /// Why it is not eligible, failed or waits, or what a later start did;
    /// never a value or a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Failed attempts since the last cut-over.
    pub attempts: u32,
    /// Attempts the agent makes by itself before only Retry runs it again.
    pub max_attempts: u32,
    /// Whether `retryModuleMigration` is possible now: failed, or not
    /// eligible because the user declined.
    pub can_retry: bool,
    pub route: MigrationRoute,
    /// Why the legacy action types are unavailable
    /// (`route: pluginWithdrawn`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_reason: Option<String>,
    /// The module's legacy action types and their plugin replacements.
    pub aliases: Vec<ModuleActionAlias>,
    /// What waits for the user's decision (`phase: awaitingConsent`), or
    /// what the user declined (`phase: notEligible`). Empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consent: Vec<MigrationConsentItem>,
    /// When the record last changed (seconds since the Unix epoch).
    pub updated_at: i64,
}

/// Checks the plugin data and module migration requests.
pub(crate) fn check_request(request: &crate::Request) -> Result<(), String> {
    match request {
        crate::Request::SetPluginSettings { plugin, values } => {
            check_text("plugin", plugin)?;
            check_settings(values)
        }
        crate::Request::SetPluginCredential {
            plugin,
            key,
            secret,
        } => {
            check_text("plugin", plugin)?;
            check_text("credential", key)?;
            check_secret(secret)
        }
        crate::Request::DeletePluginCredential { plugin, key } => {
            check_text("plugin", plugin)?;
            check_text("credential", key)
        }
        crate::Request::GetPluginActions { plugin, from } => {
            check_text("plugin", plugin)?;
            if *from > MAX_ACTION_INDEX {
                return Err(format!("from must be at most {MAX_ACTION_INDEX}"));
            }
            Ok(())
        }
        crate::Request::GetPluginIcon {
            plugin,
            offset,
            navigation,
        } => {
            check_text("plugin", plugin)?;
            if let Some(navigation) = navigation {
                check_text("navigation entry", &navigation.entry)?;
            }
            if *offset > MAX_ICON_BYTES as u64 {
                return Err(format!("offset must be at most {MAX_ICON_BYTES}"));
            }
            Ok(())
        }
        crate::Request::RetryModuleMigration { module } => check_module(module),
        crate::Request::GrantPluginUserOrigin {
            plugin,
            setting,
            origin,
            ..
        } => {
            check_text("plugin", plugin)?;
            check_text("setting", setting)?;
            if origin.is_empty() || origin.len() > MAX_USER_ORIGIN {
                return Err(format!(
                    "origin must be between 1 and {MAX_USER_ORIGIN} characters"
                ));
            }
            if origin.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err("origin contains whitespace or control characters".into());
            }
            Ok(())
        }
        crate::Request::RevokePluginUserOrigin { plugin, setting } => {
            check_text("plugin", plugin)?;
            check_text("setting", setting)
        }
        crate::Request::DecideModuleMigration {
            module,
            approve,
            decline,
        } => {
            check_module(module)?;
            check_decisions(approve, decline)
        }
        _ => Ok(()),
    }
}

fn check_module(module: &str) -> Result<(), String> {
    if module.is_empty() || module.len() > MAX_MODULE_ID {
        return Err(format!(
            "module must be between 1 and {MAX_MODULE_ID} characters"
        ));
    }
    if !module
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("module must be a built-in module id".into());
    }
    Ok(())
}

fn check_decisions(approve: &[String], decline: &[String]) -> Result<(), String> {
    if approve.is_empty() && decline.is_empty() {
        return Err("a decision approves or declines at least one item".into());
    }
    if approve.len() + decline.len() > MAX_MIGRATION_ITEMS {
        return Err(format!(
            "a decision covers at most {MAX_MIGRATION_ITEMS} items"
        ));
    }
    approve
        .iter()
        .chain(decline)
        .try_for_each(|key| check_text("permission", key))?;
    if approve.iter().any(|key| decline.contains(key)) {
        return Err("an item is either approved or declined".into());
    }
    Ok(())
}

/// A credential secret the user enters (`setPluginCredential`). It goes
/// from the app to the agent only: no answer carries it, `Debug` shows
/// `<redacted>`, and it is zeroized when dropped. Only sent over the
/// authenticated local socket, like the database key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginSecret(zeroize::Zeroizing<String>);

impl PluginSecret {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(zeroize::Zeroizing::new(secret.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for PluginSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginSecret(<redacted>)")
    }
}

/// One credential slot of a plugin (`contributes.credentials`) and whether
/// the user stored a secret for it. Never the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCredentialSlot {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The only hosts the secret is ever sent to (besides the server of
    /// `user_origin`).
    pub hosts: Vec<String>,
    /// The origin setting whose server also gets the secret, once the user
    /// allowed that server (G2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_origin: Option<String>,
    /// `bearer`, `basic`, `header`, `query`.
    pub schemes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    /// Whether a secret is stored.
    pub set: bool,
    /// When it was stored (seconds since the Unix epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// Whether its `credentials:<key>` permission is granted: without it
    /// the secret is stored but never used.
    pub granted: bool,
}

/// One setting of a plugin (`contributes.settings.fields`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingField {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The parameter kind, as in action parameters (`{"type": "integer",
    /// "min": 1}`).
    pub kind: serde_json::Value,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// `portable`, `machineSpecific` or `sensitive`.
    pub portability: String,
}

/// A plugin's settings: the declared fields, the values the user saved,
/// and the values the plugin sees (saved, or the defaults).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettings {
    pub plugin: String,
    pub fields: Vec<PluginSettingField>,
    /// What the user saved.
    pub values: BTreeMap<String, serde_json::Value>,
    /// What `settings.get` gives the plugin: saved values that still fit
    /// their field, otherwise the defaults.
    pub effective: BTreeMap<String, serde_json::Value>,
    /// Required settings with neither a value nor a default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// The origin settings (G2): the origin each holds and whether the user
    /// consented to it. Empty for a plugin without origin settings, and
    /// from an agent that predates them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<PluginUserOrigin>,
}

/// Where the consent of an origin setting stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserOriginStatus {
    /// The setting is empty: the plugin reaches no user origin.
    Unset,
    /// The setting holds something that is not an origin this slot takes
    /// (see `problem`).
    Invalid,
    /// An exact host the plugin declares and was granted at install: no
    /// further consent.
    Declared,
    /// The slot itself (`network:user-origin:<setting>`) is not granted.
    SlotNotGranted,
    /// Waits for the user's consent to this origin.
    NeedsConsent,
    /// The user consented to exactly this origin.
    Granted,
}

/// One origin setting of a plugin (`permissions[].userOrigins`, G2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts the consent dialog shows"
)]
pub struct PluginUserOrigin {
    /// The setting key.
    pub setting: String,
    /// The setting's label.
    pub label: String,
    /// The slot's grant key (`network:user-origin:<setting>`).
    pub permission: String,
    pub status: UserOriginStatus,
    /// The origin the setting holds, normalized, with its path prefix
    /// (`https://ghe.example.com/api/v3`). What `grantPluginUserOrigin`
    /// must name. Absent while unset or invalid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Why the value is not an origin this slot takes (`status: invalid`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// The host itself is on a private network (an IP literal or a local
    /// name): consent needs `privateNetwork`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private_host: bool,
    /// The slot lets the user allow a private network.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private_network_allowed: bool,
    /// The user allowed this origin on a private network.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private_network_granted: bool,
    /// The slot lets the user enter a path prefix.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub path_prefix_allowed: bool,
    /// The methods requests to it may use.
    pub methods: Vec<String>,
    /// The credential slots whose secrets are sent there once consented.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<String>,
    /// When the user consented (seconds since the Unix epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<i64>,
    /// The plugin may also open pages of this server in the browser
    /// (`openUrl.userOrigins`, and its permission is granted): any https
    /// page of the same host and port, whatever the path. Additive: older
    /// agents leave it out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub opens_pages: bool,
}

/// A plugin the agent installed at some point (`plugin_known`), installed
/// now or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownPlugin {
    pub id: String,
    pub name: String,
    pub publisher: String,
    /// Its variable namespace, `plugins.<publisher>.<name>` in camel case.
    pub namespace: String,
    pub last_version: String,
    pub installed: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// What `uninstallPlugin` keeps for a later install of the same plugin.
/// By default nothing: its storage, settings and credentials are deleted
/// (its name stays in `plugin_known`, its audit stays).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRetention {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub settings: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub credentials: bool,
}

impl PluginRetention {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

pub(crate) fn check_secret(secret: &PluginSecret) -> Result<(), String> {
    if secret.expose().is_empty() || secret.expose().len() > MAX_PLUGIN_SECRET {
        Err(format!(
            "a secret must be between 1 and {MAX_PLUGIN_SECRET} bytes"
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn check_settings(values: &BTreeMap<String, serde_json::Value>) -> Result<(), String> {
    if values.len() > MAX_SETTINGS_VALUES {
        return Err(format!("at most {MAX_SETTINGS_VALUES} settings at once"));
    }
    values.keys().try_for_each(|key| check_text("setting", key))
}

/// Checks a plugin id, action id, version or permission key from a request
/// (bounded, printable, no whitespace). The agent parses it afterwards.
pub(crate) fn check_text(field: &str, text: &str) -> Result<(), String> {
    if text.is_empty() || text.len() > MAX_PLUGIN_TEXT {
        return Err(format!(
            "{field} must be between 1 and {MAX_PLUGIN_TEXT} characters"
        ));
    }
    if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(format!("{field} contains whitespace or control characters"));
    }
    Ok(())
}

pub(crate) fn check_consent(keys: &[String]) -> Result<(), String> {
    if keys.len() > MAX_CONSENT_KEYS {
        return Err(format!(
            "a consent lists at most {MAX_CONSENT_KEYS} permissions"
        ));
    }
    keys.iter()
        .try_for_each(|key| check_text("permission", key))
}

impl PluginInstallSource {
    pub(crate) fn check(&self) -> Result<(), String> {
        match self {
            Self::Registry { plugin, version } => {
                check_text("plugin", plugin)?;
                version
                    .as_deref()
                    .map_or(Ok(()), |v| check_text("version", v))
            }
            Self::File { path } => {
                if path.is_empty() || path.len() > MAX_PACKAGE_PATH || path.contains('\0') {
                    Err(format!(
                        "path must be between 1 and {MAX_PACKAGE_PATH} characters"
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn greet(definition: Option<deckpilot_core::ActionType>) -> PluginActionInfo {
        PluginActionInfo {
            action: "greet".into(),
            action_type: "acme.hello-weather.greet".into(),
            name: "Greet".into(),
            description: None,
            available: true,
            unavailable_reason: None,
            definition,
        }
    }

    #[test]
    fn an_action_without_a_definition_keeps_its_old_shape() {
        let json = serde_json::to_value(greet(None)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "action": "greet",
                "actionType": "acme.hello-weather.greet",
                "name": "Greet",
                "available": true,
            })
        );
        // What an agent before the field sends still parses.
        let parsed: PluginActionInfo = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.definition, None);
    }

    #[test]
    fn secrets_are_redacted_and_retention_defaults_to_nothing() {
        let secret = PluginSecret::new("s3cret");
        assert_eq!(format!("{secret:?}"), "PluginSecret(<redacted>)");
        assert_eq!(serde_json::to_string(&secret).unwrap(), r#""s3cret""#);
        assert!(check_secret(&PluginSecret::new("")).is_err());
        assert!(check_secret(&PluginSecret::new("x".repeat(MAX_PLUGIN_SECRET + 1))).is_err());
        let keep: PluginRetention = serde_json::from_str("{}").unwrap();
        assert!(keep.is_default());
        assert_eq!(serde_json::to_string(&keep).unwrap(), "{}");
    }

    #[test]
    fn storage_usage_is_additive() {
        let usage = PluginStorageUsage {
            used_bytes: 12,
            keys: 1,
            quota_bytes: 262_144,
            max_keys: 1_000,
        };
        assert_eq!(
            serde_json::to_value(usage).unwrap(),
            serde_json::json!({"usedBytes": 12, "keys": 1, "quotaBytes": 262_144, "maxKeys": 1_000})
        );
    }

    #[test]
    fn registry_metadata_is_additive() {
        // What an agent before the fields sends still parses, with nothing.
        let old: AvailablePlugin = serde_json::from_value(serde_json::json!({
            "id": "acme.hello-weather",
            "name": "Hello Weather",
            "summary": "Greets.",
            "publisher": "acme",
            "versions": [{"version": "1.0.0", "permissions": []}],
        }))
        .unwrap();
        assert_eq!(old.categories, Vec::<String>::new());
        assert_eq!(old.publisher_info, None);
        assert_eq!(old.versions[0].deckpilot, None);
        assert_eq!(old.versions[0].platforms, Vec::<String>::new());
        // Without values nothing new is sent.
        let json = serde_json::to_value(&old).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["id", "name", "publisher", "summary", "versions"]);
        let mut keys: Vec<_> = json["versions"][0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(keys, ["permissions", "version"]);

        let full = AvailablePlugin {
            description: Some("Long text.".into()),
            categories: vec!["weather".into()],
            publisher_info: Some(PublisherInfo {
                namespace: "acme".into(),
                display_name: Some("Acme Tools".into()),
                login: "acme".into(),
                verified: true,
            }),
            license: Some("Apache-2.0".into()),
            homepage: Some("https://acme.example".into()),
            source_repository: Some("acme/plugins".into()),
            deprecated: None,
            versions: vec![AvailableVersion {
                version: "1.0.0".into(),
                permissions: vec![],
                withdrawn: None,
                incompatible: vec![],
                deckpilot: Some(">=0.5.0 <1.0.0".into()),
                platforms: vec!["macos".into(), "linux".into()],
                published_at: Some(1_700_000_000),
            }],
            ..old
        };
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(json["publisherInfo"]["displayName"], "Acme Tools");
        assert_eq!(json["publisherInfo"]["verified"], true);
        assert_eq!(json["sourceRepository"], "acme/plugins");
        assert_eq!(json["versions"][0]["deckpilot"], ">=0.5.0 <1.0.0");
        assert_eq!(json["versions"][0]["platforms"][1], "linux");
        assert_eq!(json["versions"][0]["publishedAt"], 1_700_000_000);
        let back: AvailablePlugin = serde_json::from_value(json).unwrap();
        assert_eq!(back, full);
    }

    /// Navigation (feature `navigation`): left out when empty, so the
    /// summary of an older agent and of a plugin without entries is the
    /// same; `getPluginIcon` without `navigation` is the original request.
    fn navigation_is_additive(summary: &PluginSummary) {
        assert_eq!(summary.navigation, Vec::new());
        assert!(
            serde_json::to_value(summary)
                .unwrap()
                .get("navigation")
                .is_none()
        );
        let navigating = PluginSummary {
            navigation: vec![PluginNavigationEntry {
                id: "dashboard".into(),
                label: "GitHub".into(),
                view: PluginView {
                    id: "dashboard".into(),
                    kind: "variables".into(),
                    context: None,
                },
            }],
            ..summary.clone()
        };
        let json = serde_json::to_value(&navigating).unwrap();
        assert_eq!(
            json["navigation"],
            serde_json::json!([{"id": "dashboard", "label": "GitHub",
                "view": {"id": "dashboard", "kind": "variables"}}])
        );
        assert_eq!(
            serde_json::from_value::<PluginSummary>(json).unwrap(),
            navigating
        );
        // A view's context (plugin API 1.4) is additive too: left out for
        // none, read as none from an older agent, kept as sent otherwise.
        let older: PluginView =
            serde_json::from_value(serde_json::json!({"id": "v", "kind": "actions"})).unwrap();
        assert_eq!(older.context, None);
        let with_context = PluginView {
            id: "v".into(),
            kind: "actions".into(),
            context: Some("device+profile".into()),
        };
        assert_eq!(
            serde_json::to_value(&with_context).unwrap(),
            serde_json::json!({"id": "v", "kind": "actions", "context": "device+profile"})
        );
        let request = crate::Request::GetPluginIcon {
            plugin: "a.b".into(),
            offset: 0,
            navigation: None,
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"type": "getPluginIcon", "plugin": "a.b", "offset": 0})
        );
        let request = crate::Request::GetPluginIcon {
            plugin: "a.b".into(),
            offset: 0,
            navigation: Some(NavigationIconRef {
                entry: "dashboard".into(),
                theme: NavigationIconTheme::Dark,
            }),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap()["navigation"],
            serde_json::json!({"entry": "dashboard", "theme": "dark"})
        );
    }

    #[test]
    fn health_and_icon_are_additive() {
        let summary: PluginSummary = serde_json::from_value(serde_json::json!({
            "id": "acme.hello-weather",
            "name": "Hello Weather",
            "publisher": "Acme",
            "version": "1.0.0",
            "state": "enabled",
            "origin": {"type": "sideload"},
            "actions": [],
        }))
        .unwrap();
        assert_eq!(summary.health, None);
        assert!(
            serde_json::to_value(&summary)
                .unwrap()
                .get("health")
                .is_none()
        );
        assert_eq!(summary.action_count, None);
        let sick = PluginSummary {
            health: Some(PluginHealth {
                crashes: 2,
                last_error: Some("the plugin host stopped".into()),
                last_error_at: Some(5),
            }),
            ..summary.clone()
        };
        let json = serde_json::to_value(&sick).unwrap();
        assert_eq!(
            json["health"],
            serde_json::json!({
                "crashes": 2,
                "lastError": "the plugin host stopped",
                "lastErrorAt": 5,
            })
        );
        assert_eq!(serde_json::from_value::<PluginSummary>(json).unwrap(), sick);
        navigation_is_additive(&summary);

        let details: PluginDetails = serde_json::from_value(serde_json::json!({
            "summary": serde_json::to_value(&summary).unwrap(),
            "description": "",
            "license": "MIT",
            "permissions": [],
            "manifest": null,
            "audit": [],
        }))
        .unwrap();
        assert_eq!(details.icon, None);
        assert!(
            serde_json::to_value(&details)
                .unwrap()
                .get("icon")
                .is_none()
        );
    }

    #[test]
    fn bundled_is_an_origin() {
        assert_eq!(
            serde_json::to_value(PluginOrigin::Bundled).unwrap(),
            serde_json::json!({"type": "bundled"})
        );
        let parsed: PluginOrigin =
            serde_json::from_value(serde_json::json!({"type": "bundled"})).unwrap();
        assert_eq!(parsed, PluginOrigin::Bundled);
    }

    fn waiting() -> ModuleMigrationInfo {
        ModuleMigrationInfo {
            module: "github".into(),
            module_name: "GitHub".into(),
            plugin: "deckpilot.github".into(),
            plugin_name: Some("GitHub".into()),
            phase: MigrationPhase::AwaitingConsent,
            reason: Some("waiting for your consent: credentials:token".into()),
            attempts: 0,
            max_attempts: 3,
            can_retry: false,
            route: MigrationRoute::Builtin,
            route_reason: None,
            aliases: vec![ModuleActionAlias {
                legacy: "github.pull-requests".into(),
                target: "deckpilot.github.pull-requests".into(),
            }],
            consent: vec![MigrationConsentItem {
                key: "credentials:token".into(),
                permission: PluginPermission {
                    key: "credentials:token".into(),
                    kind: "credentials".into(),
                    tier: PermissionTier::High,
                    status: PermissionStatus::NotGranted,
                    scope: serde_json::json!({"type": "credential"}),
                    justification: Some("Reads your pull requests.".into()),
                },
                import: Some(CredentialImport {
                    setting: "credential".into(),
                    credential: "github.token".into(),
                    slot: "token".into(),
                    hosts: vec!["api.github.com".into()],
                    user_origin: None,
                }),
                origin: None,
                why: "the built-in used the stored token".into(),
                decision: ConsentDecision::Pending,
            }],
            updated_at: 1_767_225_600,
        }
    }

    #[test]
    fn a_migration_names_what_it_imports_never_a_value() {
        let info = waiting();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["phase"], "awaitingConsent");
        assert_eq!(json["route"], "builtin");
        assert_eq!(json["moduleName"], "GitHub");
        assert_eq!(json["maxAttempts"], 3);
        assert_eq!(json["canRetry"], false);
        assert_eq!(json["aliases"][0]["legacy"], "github.pull-requests");
        let item = &json["consent"][0];
        assert_eq!(item["key"], "credentials:token");
        assert_eq!(item["decision"], "pending");
        assert_eq!(item["permission"]["tier"], "high");
        assert_eq!(
            item["import"],
            serde_json::json!({"setting": "credential", "credential": "github.token",
                "slot": "token", "hosts": ["api.github.com"]})
        );
        assert!(json.get("routeReason").is_none());
        assert_eq!(
            serde_json::from_value::<ModuleMigrationInfo>(json).unwrap(),
            info
        );
        // Nothing waits: no `consent` key at all.
        let done = ModuleMigrationInfo {
            phase: MigrationPhase::CutOver,
            route: MigrationRoute::Plugin,
            consent: vec![],
            reason: None,
            ..info
        };
        let json = serde_json::to_value(&done).unwrap();
        assert!(json.get("consent").is_none());
        assert!(json.get("reason").is_none());
    }

    #[test]
    fn phases_and_routes_of_a_newer_agent_still_parse() {
        let mut json = serde_json::to_value(waiting()).unwrap();
        json["phase"] = "rollingBack".into();
        json["route"] = "somewhereElse".into();
        let info: ModuleMigrationInfo = serde_json::from_value(json).unwrap();
        assert_eq!(info.phase, MigrationPhase::Other);
        assert_eq!(info.route, MigrationRoute::Other);
        for (phase, text) in [
            (MigrationPhase::Pending, "pending"),
            (MigrationPhase::NotEligible, "notEligible"),
            (MigrationPhase::Installed, "installed"),
            (MigrationPhase::SettingsCopied, "settingsCopied"),
            (MigrationPhase::AwaitingConsent, "awaitingConsent"),
            (MigrationPhase::CutOver, "cutOver"),
            (MigrationPhase::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap(), text);
        }
        assert_eq!(
            serde_json::to_value(MigrationRoute::PluginWithdrawn).unwrap(),
            "pluginWithdrawn"
        );
    }

    #[test]
    fn an_action_carries_its_definition() {
        let definition: deckpilot_core::ActionType = serde_json::from_value(serde_json::json!({
            "id": "acme.hello-weather.greet",
            "name": "Greet",
            "description": "Says hello.",
            "category": "Fun",
            "parameters": [
                {"key": "name", "label": "Name", "kind": {"type": "text"}, "required": true}
            ],
            "requiresProject": false,
        }))
        .unwrap();
        let info = greet(Some(definition.clone()));
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["definition"]["id"], "acme.hello-weather.greet");
        assert_eq!(json["definition"]["parameters"][0]["key"], "name");
        let back: PluginActionInfo = serde_json::from_value(json).unwrap();
        assert_eq!(back.definition, Some(definition));
    }
}
