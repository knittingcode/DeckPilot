//! Records of the plugin database and the queries on them.
//!
//! [`PluginStore`] is the read side (also usable from a second process
//! while the manager runs). The write helpers take a transaction's
//! connection and are only called by the manager.

use std::path::Path;
use std::time::Duration;

use deckpilot_plugin_manifest::{Manifest, PackageIntegrity, PluginId};
use rusqlite::{Connection, OptionalExtension as _, params};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::db::{Database, StoreKey};
use crate::error::{Result, StoreError};
use crate::failpoint::Operation;
use crate::grants::{
    GrantKey, GrantScope, GrantSet, GrantSource, GrantStatus, StoredGrant, parse_kind,
};
use crate::state::PluginState;

/// Where an installed plugin came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum InstallSource {
    /// A verified registry (its base URL or name).
    Registry {
        registry: String,
    },
    Sideload,
    Dev,
}

impl InstallSource {
    fn column(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Registry { registry } => ("registry", Some(registry)),
            Self::Sideload => ("sideload", None),
            Self::Dev => ("dev", None),
        }
    }

    fn from_columns(source: &str, registry: Option<String>) -> Result<Self> {
        match (source, registry) {
            ("registry", Some(registry)) => Ok(Self::Registry { registry }),
            ("sideload", None) => Ok(Self::Sideload),
            ("dev", None) => Ok(Self::Dev),
            (other, _) => Err(StoreError::record(format!("install source {other:?}"))),
        }
    }
}

/// The role of an installed version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionRole {
    /// The version that runs.
    Active,
    /// The version a rollback returns to (only one is kept).
    Previous,
    /// An unpacked update waiting for consent.
    Staged,
}

impl VersionRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Previous => "previous",
            Self::Staged => "staged",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        match text {
            "active" => Ok(Self::Active),
            "previous" => Ok(Self::Previous),
            "staged" => Ok(Self::Staged),
            other => Err(StoreError::record(format!("version role {other:?}"))),
        }
    }
}

/// A row of `plugin_versions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRecord {
    pub version: Version,
    pub role: VersionRole,
    /// `<id>/<version>`, relative to `<data dir>/plugins`.
    pub install_dir: String,
    pub integrity: PackageIntegrity,
    pub manifest: Manifest,
    pub installed_at: i64,
    pub activated_at: Option<i64>,
}

impl VersionRecord {
    pub fn grants(&self) -> GrantSet {
        GrantSet::from_manifest(&self.manifest)
    }
}

/// An installed plugin with its versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRecord {
    pub id: PluginId,
    pub state: PluginState,
    pub state_reason: Option<String>,
    pub source: InstallSource,
    pub installed_at: i64,
    pub updated_at: i64,
    pub versions: Vec<VersionRecord>,
}

impl PluginRecord {
    pub fn version(&self, role: VersionRole) -> Option<&VersionRecord> {
        self.versions.iter().find(|v| v.role == role)
    }

    pub fn active(&self) -> Option<&VersionRecord> {
        self.version(VersionRole::Active)
    }

    pub fn previous(&self) -> Option<&VersionRecord> {
        self.version(VersionRole::Previous)
    }

    pub fn staged(&self) -> Option<&VersionRecord> {
        self.version(VersionRole::Staged)
    }
}

/// A row of `plugin_audit`. Targets are grant keys, never values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub id: i64,
    pub at: i64,
    pub plugin_id: String,
    pub version: Option<String>,
    pub op: String,
    pub target: Option<String>,
    pub decision: String,
    pub reason: Option<String>,
}

/// Retention of the audit log (architecture section 5.7): 30 days or
/// 10,000 rows per plugin, whichever is smaller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditRetention {
    pub max_age_seconds: i64,
    pub max_rows_per_plugin: u32,
}

impl Default for AuditRetention {
    fn default() -> Self {
        Self {
            max_age_seconds: 30 * 24 * 60 * 60,
            max_rows_per_plugin: 10_000,
        }
    }
}

/// An unfinished operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub id: i64,
    pub plugin_id: String,
    pub op: Operation,
    pub step: String,
    pub(crate) payload: Payload,
    pub started_at: i64,
}

/// What recovery needs to complete or undo an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Payload {
    Install {
        package: PendingVersion,
        source: InstallSource,
        consent: crate::grants::Consent,
    },
    Update {
        package: PendingVersion,
        source: InstallSource,
        prior_state: PluginState,
    },
    Uninstall {
        prior_state: PluginState,
    },
}

impl Payload {
    pub(crate) fn op(&self) -> Operation {
        match self {
            Self::Install { .. } => Operation::Install,
            Self::Update { .. } => Operation::Update,
            Self::Uninstall { .. } => Operation::Uninstall,
        }
    }
}

/// A version being written: everything its `plugin_versions` row needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingVersion {
    pub version: Version,
    pub install_dir: String,
    pub manifest_json: String,
    pub integrity: PackageIntegrity,
}

impl PendingVersion {
    pub(crate) fn manifest(&self) -> Result<Manifest> {
        serde_json::from_str(&self.manifest_json).map_err(StoreError::record)
    }
}

/// Read access to the plugin database.
#[derive(Debug)]
pub struct PluginStore {
    db: Database,
}

impl PluginStore {
    /// Opens (creating and migrating) the encrypted plugin database at
    /// `path` with `key`.
    pub fn open(path: &Path, key: &StoreKey, busy_timeout: Duration) -> Result<Self> {
        Ok(Self {
            db: Database::open(path, key, busy_timeout)?,
        })
    }

    pub fn path(&self) -> &Path {
        self.db.path()
    }

    pub fn schema_version(&self) -> Result<u32> {
        self.db.schema_version()
    }

    pub(crate) fn db(&mut self) -> &mut Database {
        &mut self.db
    }

    pub(crate) fn connection(&self) -> &Connection {
        self.db.connection()
    }

    /// The raw connection, for tests that hold locks or inspect rows. Not
    /// part of the API.
    #[doc(hidden)]
    pub fn connection_for_tests(&self) -> &Connection {
        self.db.connection()
    }

    pub fn plugin(&self, id: &PluginId) -> Result<Option<PluginRecord>> {
        load_plugin(self.connection(), id)
    }

    pub fn plugins(&self) -> Result<Vec<PluginRecord>> {
        let ids: Vec<String> = {
            let mut statement = self
                .connection()
                .prepare("SELECT plugin_id FROM plugins ORDER BY plugin_id")?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut plugins = Vec::with_capacity(ids.len());
        for id in ids {
            let id = parse_id(&id)?;
            if let Some(plugin) = load_plugin(self.connection(), &id)? {
                plugins.push(plugin);
            }
        }
        Ok(plugins)
    }

    pub fn grants(&self, id: &PluginId) -> Result<Vec<StoredGrant>> {
        load_grants(self.connection(), id)
    }

    /// What the broker may allow right now: nothing unless the plugin is
    /// enabled; otherwise the declared grants of the active version that
    /// are granted with at least the declared scope (the intersection of
    /// manifest and stored consent, section 5.1).
    pub fn effective_grants(&self, id: &PluginId) -> Result<Vec<(GrantKey, GrantScope)>> {
        let Some(plugin) = self.plugin(id)? else {
            return Ok(Vec::new());
        };
        let Some(active) = plugin.active() else {
            return Ok(Vec::new());
        };
        if !plugin.state.is_runnable() {
            return Ok(Vec::new());
        }
        let stored = self.grants(id)?;
        Ok(active
            .grants()
            .iter()
            .filter(|(key, scope)| {
                stored.iter().any(|row| {
                    row.key == **key
                        && row.status == GrantStatus::Granted
                        && row.scope.covers(scope)
                })
            })
            .map(|(key, scope)| (key.clone(), scope.clone()))
            .collect())
    }

    pub fn audit(&self, id: &PluginId) -> Result<Vec<AuditRecord>> {
        let mut statement = self.connection().prepare(
            "SELECT id, at, plugin_id, version, op, target, decision, reason
             FROM plugin_audit WHERE plugin_id = ?1 ORDER BY id",
        )?;
        let rows = statement
            .query_map([id.as_str()], |row| {
                Ok(AuditRecord {
                    id: row.get(0)?,
                    at: row.get(1)?,
                    plugin_id: row.get(2)?,
                    version: row.get(3)?,
                    op: row.get(4)?,
                    target: row.get(5)?,
                    decision: row.get(6)?,
                    reason: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Unfinished operations (empty after a successful recovery).
    pub fn journal(&self) -> Result<Vec<JournalEntry>> {
        load_journal(self.connection())
    }
}

pub(crate) fn parse_id(text: &str) -> Result<PluginId> {
    PluginId::parse(text).map_err(StoreError::record)
}

pub(crate) fn load_plugin(conn: &Connection, id: &PluginId) -> Result<Option<PluginRecord>> {
    let row = conn
        .query_row(
            "SELECT state, state_reason, source, registry, installed_at, updated_at
             FROM plugins WHERE plugin_id = ?1",
            [id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((state, state_reason, source, registry, installed_at, updated_at)) = row else {
        return Ok(None);
    };
    let state = PluginState::parse(&state)
        .ok_or_else(|| StoreError::record(format!("plugin state {state:?}")))?;
    Ok(Some(PluginRecord {
        id: id.clone(),
        state,
        state_reason,
        source: InstallSource::from_columns(&source, registry)?,
        installed_at,
        updated_at,
        versions: load_versions(conn, id)?,
    }))
}

type VersionRow = (String, String, String, String, String, i64, Option<i64>);
type GrantRow = (String, String, String, String, String, String, i64, String);

fn load_versions(conn: &Connection, id: &PluginId) -> Result<Vec<VersionRecord>> {
    let mut statement = conn.prepare(
        "SELECT version, role, install_dir, integrity_json, manifest_json, installed_at,
                activated_at
         FROM plugin_versions WHERE plugin_id = ?1
         ORDER BY CASE role WHEN 'active' THEN 0 WHEN 'staged' THEN 1 ELSE 2 END",
    )?;
    let rows: Vec<VersionRow> = statement
        .query_map([id.as_str()], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(
            |(version, role, install_dir, integrity, manifest, installed_at, activated_at)| {
                Ok(VersionRecord {
                    version: Version::parse(&version).map_err(StoreError::record)?,
                    role: VersionRole::parse(&role)?,
                    install_dir,
                    integrity: serde_json::from_str(&integrity).map_err(StoreError::record)?,
                    manifest: serde_json::from_str(&manifest).map_err(StoreError::record)?,
                    installed_at,
                    activated_at,
                })
            },
        )
        .collect()
}

pub(crate) fn load_grants(conn: &Connection, id: &PluginId) -> Result<Vec<StoredGrant>> {
    let mut statement = conn.prepare(
        "SELECT kind, grant_key, grant_json, status, source, decided_by, decided_at,
                granted_for_version
         FROM plugin_grants WHERE plugin_id = ?1 ORDER BY kind, grant_key",
    )?;
    let rows: Vec<GrantRow> = statement
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
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(
            |(kind, key, scope, status, source, decided_by, decided_at, version)| {
                Ok(StoredGrant {
                    key: GrantKey::new(
                        parse_kind(&kind)
                            .ok_or_else(|| StoreError::record(format!("grant kind {kind:?}")))?,
                        key,
                    ),
                    scope: serde_json::from_str(&scope).map_err(StoreError::record)?,
                    status: GrantStatus::parse(&status)
                        .ok_or_else(|| StoreError::record(format!("grant status {status:?}")))?,
                    source: GrantSource::parse(&source)
                        .ok_or_else(|| StoreError::record(format!("grant source {source:?}")))?,
                    decided_by,
                    decided_at,
                    granted_for_version: version,
                })
            },
        )
        .collect()
}

pub(crate) fn insert_plugin(
    conn: &Connection,
    id: &PluginId,
    state: PluginState,
    source: &InstallSource,
    now: i64,
) -> Result<()> {
    let (source, registry) = source.column();
    conn.execute(
        "INSERT INTO plugins (plugin_id, state, source, registry, installed_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![id.as_str(), state.as_str(), source, registry, now],
    )?;
    Ok(())
}

pub(crate) fn set_source(conn: &Connection, id: &PluginId, source: &InstallSource) -> Result<()> {
    let (source, registry) = source.column();
    conn.execute(
        "UPDATE plugins SET source = ?2, registry = ?3 WHERE plugin_id = ?1",
        params![id.as_str(), source, registry],
    )?;
    Ok(())
}

/// Moves the plugin from `from` (which must be its current state) to `to`,
/// refusing every transition the state machine does not allow.
pub(crate) fn transition(
    conn: &Connection,
    id: &PluginId,
    from: PluginState,
    to: PluginState,
    reason: Option<&str>,
    now: i64,
) -> Result<()> {
    if !from.can_transition_to(to) {
        return Err(StoreError::IllegalTransition {
            id: id.clone(),
            from,
            to,
        });
    }
    let changed = conn.execute(
        "UPDATE plugins SET state = ?3, state_reason = ?4, updated_at = ?5
         WHERE plugin_id = ?1 AND state = ?2",
        params![id.as_str(), from.as_str(), to.as_str(), reason, now],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(StoreError::record(format!(
            "plugin {id} was not in state {from}"
        )))
    }
}

pub(crate) fn delete_plugin(conn: &Connection, id: &PluginId) -> Result<()> {
    conn.execute("DELETE FROM plugins WHERE plugin_id = ?1", [id.as_str()])?;
    Ok(())
}

pub(crate) fn insert_version(
    conn: &Connection,
    id: &PluginId,
    pending: &PendingVersion,
    role: VersionRole,
    now: i64,
) -> Result<()> {
    let integrity = serde_json::to_string(&pending.integrity).map_err(StoreError::record)?;
    let activated = (role == VersionRole::Active).then_some(now);
    conn.execute(
        "INSERT INTO plugin_versions (plugin_id, version, role, install_dir, package_sha256,
             integrity_json, manifest_json, installed_at, activated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id.as_str(),
            pending.version.to_string(),
            role.as_str(),
            pending.install_dir,
            pending.integrity.package.sha256.to_string(),
            integrity,
            pending.manifest_json,
            now,
            activated,
        ],
    )?;
    Ok(())
}

pub(crate) fn set_role(
    conn: &Connection,
    id: &PluginId,
    version: &Version,
    role: VersionRole,
    now: i64,
) -> Result<()> {
    let activated = (role == VersionRole::Active).then_some(now);
    conn.execute(
        "UPDATE plugin_versions SET role = ?3, activated_at = coalesce(?4, activated_at)
         WHERE plugin_id = ?1 AND version = ?2",
        params![id.as_str(), version.to_string(), role.as_str(), activated],
    )?;
    Ok(())
}

pub(crate) fn delete_version(conn: &Connection, id: &PluginId, version: &Version) -> Result<()> {
    conn.execute(
        "DELETE FROM plugin_versions WHERE plugin_id = ?1 AND version = ?2",
        params![id.as_str(), version.to_string()],
    )?;
    Ok(())
}

pub(crate) fn upsert_grant(conn: &Connection, id: &PluginId, grant: &StoredGrant) -> Result<()> {
    let scope = serde_json::to_string(&grant.scope).map_err(StoreError::record)?;
    conn.execute(
        "INSERT INTO plugin_grants (plugin_id, kind, grant_key, grant_json, status, source,
             decided_by, decided_at, granted_for_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (plugin_id, kind, grant_key) DO UPDATE SET
             grant_json = excluded.grant_json, status = excluded.status,
             source = excluded.source, decided_by = excluded.decided_by,
             decided_at = excluded.decided_at,
             granted_for_version = excluded.granted_for_version",
        params![
            id.as_str(),
            grant.key.kind.as_str(),
            grant.key.key,
            scope,
            grant.status.as_str(),
            grant.source.as_str(),
            grant.decided_by,
            grant.decided_at,
            grant.granted_for_version,
        ],
    )?;
    Ok(())
}

pub(crate) fn set_grant_scope(
    conn: &Connection,
    id: &PluginId,
    key: &GrantKey,
    scope: &GrantScope,
) -> Result<()> {
    let scope = serde_json::to_string(scope).map_err(StoreError::record)?;
    conn.execute(
        "UPDATE plugin_grants SET grant_json = ?4
         WHERE plugin_id = ?1 AND kind = ?2 AND grant_key = ?3",
        params![id.as_str(), key.kind.as_str(), key.key, scope],
    )?;
    Ok(())
}

pub(crate) fn delete_grant(conn: &Connection, id: &PluginId, key: &GrantKey) -> Result<()> {
    conn.execute(
        "DELETE FROM plugin_grants WHERE plugin_id = ?1 AND kind = ?2 AND grant_key = ?3",
        params![id.as_str(), key.kind.as_str(), key.key],
    )?;
    Ok(())
}

/// One audit event; `target` is a grant key or nothing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Audit<'a> {
    pub version: Option<&'a str>,
    pub op: &'a str,
    pub target: Option<&'a GrantKey>,
    pub decision: &'a str,
    pub reason: Option<&'a str>,
}

pub(crate) fn audit(
    conn: &Connection,
    id: &PluginId,
    event: Audit<'_>,
    now: i64,
    retention: AuditRetention,
) -> Result<()> {
    conn.execute(
        "INSERT INTO plugin_audit (at, plugin_id, version, op, target, decision, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            now,
            id.as_str(),
            event.version,
            event.op,
            event.target.map(ToString::to_string),
            event.decision,
            event.reason,
        ],
    )?;
    conn.execute(
        "DELETE FROM plugin_audit WHERE plugin_id = ?1 AND at < ?2",
        params![id.as_str(), now - retention.max_age_seconds],
    )?;
    conn.execute(
        "DELETE FROM plugin_audit WHERE plugin_id = ?1 AND id NOT IN (
             SELECT id FROM plugin_audit WHERE plugin_id = ?1 ORDER BY id DESC LIMIT ?2)",
        params![id.as_str(), retention.max_rows_per_plugin],
    )?;
    Ok(())
}

pub(crate) fn insert_journal(
    conn: &Connection,
    id: &PluginId,
    payload: &Payload,
    now: i64,
) -> Result<()> {
    let json = serde_json::to_string(payload).map_err(StoreError::record)?;
    conn.execute(
        "INSERT INTO plugin_journal (plugin_id, op, step, payload_json, started_at, updated_at)
         VALUES (?1, ?2, 'started', ?3, ?4, ?4)",
        params![id.as_str(), payload.op().as_str(), json, now],
    )?;
    Ok(())
}

pub(crate) fn set_journal_step(
    conn: &Connection,
    id: &PluginId,
    step: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE plugin_journal SET step = ?2, updated_at = ?3 WHERE plugin_id = ?1",
        params![id.as_str(), step, now],
    )?;
    Ok(())
}

pub(crate) fn delete_journal(conn: &Connection, id: &PluginId) -> Result<()> {
    conn.execute(
        "DELETE FROM plugin_journal WHERE plugin_id = ?1",
        [id.as_str()],
    )?;
    Ok(())
}

pub(crate) fn journal_for(conn: &Connection, id: &PluginId) -> Result<Option<JournalEntry>> {
    Ok(load_journal(conn)?
        .into_iter()
        .find(|entry| entry.plugin_id == id.as_str()))
}

fn load_journal(conn: &Connection) -> Result<Vec<JournalEntry>> {
    let mut statement = conn.prepare(
        "SELECT id, plugin_id, op, step, payload_json, started_at
         FROM plugin_journal ORDER BY id",
    )?;
    let rows: Vec<(i64, String, String, String, String, i64)> = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, plugin_id, op, step, payload, started_at)| {
            let op = Operation::parse(&op)
                .ok_or_else(|| StoreError::record(format!("journal op {op:?}")))?;
            let payload: Payload = serde_json::from_str(&payload).map_err(StoreError::record)?;
            if payload.op() != op {
                return Err(StoreError::record("journal op and payload differ"));
            }
            Ok(JournalEntry {
                id,
                plugin_id,
                op,
                step,
                payload,
                started_at,
            })
        })
        .collect()
}
