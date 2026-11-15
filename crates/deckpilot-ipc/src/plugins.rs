//! Plugin shapes of protocol 10 (capability [`Capability::Plugins`]).
//!
//! The configuration app manages plugins with these requests:
//!
//! | Request | Answer | Does |
//! | --- | --- | --- |
//! | `getPluginRuntime` | `pluginRuntime` | whether plugins can be used here, and why not |
//! | `listPlugins` | `plugins` | installed plugins |
//! | `getPlugin` | `plugin` | one installed plugin: manifest, permissions with their grant state, actions, pending update, recent audit |
//! | `listAvailablePlugins` | `availablePlugins` | the verified registry index with per-version permissions and compatibility (`refresh` re-reads the registry first) |
//! | `installPlugin` | `plugin` | installs from the registry or from a package file (verified against the registry pins), in state `disabled`, granting `consent` |
//! | `enablePlugin` | `plugin` | grants `consent`, then enables; every declared permission must be granted |
//! | `disablePlugin` | `plugin` | stops its host and disables it |
//! | `grantPluginPermissions` / `revokePluginPermission` | `plugin` | changes one plugin's grants; effective on the next capability call |
//! | `updatePlugin` | `pluginUpdate` | installs a newer version; an escalation stays staged unless `consent` covers it |
//! | `approvePluginUpdate` / `declinePluginUpdate` | `pluginUpdate` / `plugin` | decides a staged update |
//! | `rollbackPlugin` | `plugin` | returns to the kept previous version |
//! | `uninstallPlugin` | `ok` | removes files, records and grants |
//! | `invokePluginAction` | `pluginActionResult` | runs one action out of process (for testing and the UI) |
//!
//! Progress and state changes arrive as `pluginEvent` messages
//! ([`PluginEvent`]) on connections that sent `subscribePluginEvents`
//! (`AgentClient::subscribe_plugin_events` and
//! `AgentClient::next_plugin_event`). Permissions are named by their grant key
//! ([`PluginPermission::key`]): `network:api.example.com`,
//! `notifications`, `credentials:apiToken`, … The agent answers plugin
//! requests only when it was built with plugins and they are enabled;
//! otherwise every request but `getPluginRuntime` fails with
//! [`ErrorCode::Unsupported`](crate::ErrorCode::Unsupported) and the reason
//! [`reason::PLUGINS_UNAVAILABLE`]. Errors carry a machine-readable
//! [`IpcError::reason`](crate::IpcError::reason) from [`reason`].
//!
//! Every value is checked by the agent: identifiers are bounded and
//! validated there, and nothing here carries a secret.
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
}

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
    Registry { registry: String },
    Sideload,
    Dev,
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
    pub actions: Vec<PluginActionInfo>,
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
