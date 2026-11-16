//! User-entered origins (G2, `docs/marketplace/module-migration.md` §7).
//!
//! A manifest declares origin slots in its `network` permission
//! (`userOrigins`): settings the user enters a server into. Two consents
//! cover one:
//!
//! - the slot itself, a declared grant like any other
//!   (`network:user-origin:<setting>`, [`GrantScope::UserOriginSlot`]),
//!   given at install: "may connect to a server you enter";
//! - the exact origin the user entered, given when they save it
//!   ([`UserOriginConsent`], `plugin_user_origins`): "can send data and
//!   your token to `ghe.example.com`". One row per plugin and setting,
//!   recorded with the origin, path prefix, methods, credential slots and,
//!   separately, whether the user allowed it on a private network.
//!
//! The row is deleted when the user revokes it or the setting changes, and
//! with the plugin. The agent passes a row to the broker only while the
//! setting still holds exactly that origin and the active manifest still
//! declares the slot with at least what was consented (methods, path
//! prefix, private network, bound credential slots), so a stale or wider
//! row never grants anything. Rows are never exported.

use deckpilot_plugin_manifest::{
    HttpMethod, Manifest, Permission, PermissionKind, PluginId, UserOriginSlot,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::{Result, StoreError};
use crate::grants::GrantKey;

/// The prefix of a slot's grant key (`network:user-origin:apiUrl`).
pub const USER_ORIGIN_KEY_PREFIX: &str = "user-origin:";

/// Longest origin (with its path prefix) a consent may record.
pub const MAX_USER_ORIGIN_LEN: usize = 1024;

/// The declared grant of the origin slot in `setting`.
pub fn user_origin_key(setting: &str) -> GrantKey {
    GrantKey::new(
        PermissionKind::Network,
        format!("{USER_ORIGIN_KEY_PREFIX}{setting}"),
    )
}

/// The setting of a slot grant key, if `key` is one.
pub fn user_origin_setting(key: &GrantKey) -> Option<&str> {
    (key.kind == PermissionKind::Network)
        .then(|| key.key.strip_prefix(USER_ORIGIN_KEY_PREFIX))
        .flatten()
}

/// The origin slot declared for `setting` in `manifest`, and the methods
/// it allows (its own, or the permission's, or all of them).
pub fn declared_slot<'a>(
    manifest: &'a Manifest,
    setting: &str,
) -> Option<(&'a UserOriginSlot, Vec<HttpMethod>)> {
    manifest.permissions.iter().find_map(|p| match p {
        Permission::Network { methods, .. } => {
            let slot = p.user_origins().iter().find(|s| s.setting == setting)?;
            let methods = slot
                .methods
                .clone()
                .or_else(|| methods.clone())
                .unwrap_or_else(|| crate::grants::ALL_METHODS.to_vec());
            Some((slot, methods))
        }
        _ => None,
    })
}

/// The credential slots bound to the origin slot in `setting`.
pub fn bound_credentials(manifest: &Manifest, setting: &str) -> Vec<String> {
    manifest
        .contributes
        .credentials()
        .iter()
        .filter(|c| c.user_origin.as_deref() == Some(setting))
        .map(|c| c.key.clone())
        .collect()
}

/// What the user consented to for one origin setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserOriginConsent {
    pub setting: String,
    /// Normalized `https://host[:port]` (the broker's `UserOrigin`).
    pub origin: String,
    /// Normalized, starting with `/`, without a trailing `/`.
    pub path_prefix: Option<String>,
    pub methods: Vec<HttpMethod>,
    /// The separate consent to reach it on a private network.
    pub private_network: bool,
    /// Credential slots whose secrets may be sent there.
    pub credentials: Vec<String>,
}

impl UserOriginConsent {
    /// The origin with its path prefix, as shown and audited.
    pub fn display(&self) -> String {
        format!(
            "{}{}",
            self.origin,
            self.path_prefix.as_deref().unwrap_or("")
        )
    }
}

/// A row of `plugin_user_origins`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUserOrigin {
    pub consent: UserOriginConsent,
    pub decided_by: String,
    pub decided_at: i64,
    pub granted_for_version: String,
}

/// Checks a consent against the active manifest: a declared slot, an
/// origin in the normalized form, methods, a private network and credential
/// slots it allows. The broker parses the origin strictly again before it
/// grants anything.
pub(crate) fn check(id: &PluginId, manifest: &Manifest, consent: &UserOriginConsent) -> Result<()> {
    let invalid = |why: String| Err(StoreError::InvalidData(format!("{id}: {why}")));
    let Some((slot, methods)) = declared_slot(manifest, &consent.setting) else {
        return invalid(format!("`{}` is not an origin setting", consent.setting));
    };
    let host = consent.origin.strip_prefix("https://").unwrap_or_default();
    if host.is_empty()
        || host.contains(['/', '?', '#', '@', '\\'])
        || host.chars().any(|c| c.is_control() || c.is_whitespace())
        || consent.display().len() > MAX_USER_ORIGIN_LEN
    {
        return invalid(format!(
            "`{}` is not a normalized https origin",
            consent.origin
        ));
    }
    if let Some(prefix) = &consent.path_prefix {
        if !slot.path_prefix() {
            return invalid(format!("`{}` takes no path", consent.setting));
        }
        let ok = prefix.starts_with('/')
            && !prefix.ends_with('/')
            && !prefix.contains(['?', '#', '\\'])
            && !prefix.chars().any(|c| c.is_control() || c.is_whitespace());
        if !ok {
            return invalid(format!("`{prefix}` is not a normalized path prefix"));
        }
    }
    if consent.methods.is_empty() || consent.methods.iter().any(|m| !methods.contains(m)) {
        return invalid(format!(
            "the methods are not those of the origin setting `{}`",
            consent.setting
        ));
    }
    if consent.private_network && !slot.private_network() {
        return invalid(format!(
            "`{}` does not allow a server on a private network",
            consent.setting
        ));
    }
    let bound = bound_credentials(manifest, &consent.setting);
    if let Some(key) = consent.credentials.iter().find(|k| !bound.contains(k)) {
        return invalid(format!(
            "the credential `{key}` is not bound to `{}`",
            consent.setting
        ));
    }
    Ok(())
}

type Row = (
    String,
    String,
    Option<String>,
    String,
    bool,
    String,
    String,
    i64,
    String,
);

pub(crate) fn load(conn: &Connection, id: &PluginId) -> Result<Vec<StoredUserOrigin>> {
    let mut statement = conn.prepare(
        "SELECT setting_key, origin, path_prefix, methods_json, private_network,
                credentials_json, decided_by, decided_at, granted_for_version
         FROM plugin_user_origins WHERE plugin_id = ?1 ORDER BY setting_key",
    )?;
    let rows: Vec<Row> = statement
        .query_map([id.as_str()], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(
            |(setting, origin, path_prefix, methods, private_network, credentials, by, at, v)| {
                Ok(StoredUserOrigin {
                    consent: UserOriginConsent {
                        setting,
                        origin,
                        path_prefix,
                        methods: serde_json::from_str(&methods).map_err(StoreError::record)?,
                        private_network,
                        credentials: serde_json::from_str(&credentials)
                            .map_err(StoreError::record)?,
                    },
                    decided_by: by,
                    decided_at: at,
                    granted_for_version: v,
                })
            },
        )
        .collect()
}

pub(crate) fn upsert(conn: &Connection, id: &PluginId, row: &StoredUserOrigin) -> Result<()> {
    let c = &row.consent;
    conn.execute(
        "INSERT INTO plugin_user_origins (plugin_id, setting_key, origin, path_prefix,
             methods_json, private_network, credentials_json, decided_by, decided_at,
             granted_for_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (plugin_id, setting_key) DO UPDATE SET
             origin = excluded.origin, path_prefix = excluded.path_prefix,
             methods_json = excluded.methods_json,
             private_network = excluded.private_network,
             credentials_json = excluded.credentials_json,
             decided_by = excluded.decided_by, decided_at = excluded.decided_at,
             granted_for_version = excluded.granted_for_version",
        params![
            id.as_str(),
            c.setting,
            c.origin,
            c.path_prefix,
            serde_json::to_string(&c.methods).map_err(StoreError::record)?,
            c.private_network,
            serde_json::to_string(&c.credentials).map_err(StoreError::record)?,
            row.decided_by,
            row.decided_at,
            row.granted_for_version,
        ],
    )?;
    Ok(())
}

/// Deletes the consent of one origin setting; whether there was one.
pub(crate) fn delete(conn: &Connection, id: &PluginId, setting: &str) -> Result<Option<String>> {
    let shown: Option<String> = conn
        .query_row(
            "SELECT origin || coalesce(path_prefix, '') FROM plugin_user_origins
             WHERE plugin_id = ?1 AND setting_key = ?2",
            params![id.as_str(), setting],
            |row| row.get(0),
        )
        .ok();
    conn.execute(
        "DELETE FROM plugin_user_origins WHERE plugin_id = ?1 AND setting_key = ?2",
        params![id.as_str(), setting],
    )?;
    Ok(shown)
}
