//! Records of the plugin database and the queries on them.
//!
//! [`PluginStore`] is the read side (also usable from a second process
//! while the manager runs). The write helpers take a transaction's
//! connection and are only called by the manager.

use std::collections::BTreeMap;
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
use crate::state::{MigrationPhase, PluginState, TombstoneDecision};

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
    /// Shipped inside the application bundle and verified against the
    /// digests pinned in the agent (`docs/marketplace/module-migration.md`
    /// §4.4). Only first-party (`deckpilot.*`) plugins.
    Bundled,
}

impl InstallSource {
    /// The `plugins.source` column value.
    pub fn as_str(&self) -> &'static str {
        self.column().0
    }

    /// Whether a plugin with `id` may be installed from this source. A
    /// first-party id (publisher `deckpilot`) comes only from the
    /// application bundle or a registry, so a sideloaded or dev
    /// "first-party" plugin can never capture the built-in modules' action
    /// aliases; and a bundled plugin is always first-party.
    pub fn allows(&self, id: &PluginId) -> bool {
        match self {
            Self::Registry { .. } => true,
            Self::Bundled => is_first_party(id),
            Self::Sideload | Self::Dev => !is_first_party(id),
        }
    }

    fn column(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Registry { registry } => ("registry", Some(registry)),
            Self::Sideload => ("sideload", None),
            Self::Dev => ("dev", None),
            Self::Bundled => ("bundled", None),
        }
    }

    fn from_columns(source: &str, registry: Option<String>) -> Result<Self> {
        match (source, registry) {
            ("registry", Some(registry)) => Ok(Self::Registry { registry }),
            ("sideload", None) => Ok(Self::Sideload),
            ("dev", None) => Ok(Self::Dev),
            ("bundled", None) => Ok(Self::Bundled),
            (other, _) => Err(StoreError::record(format!("install source {other:?}"))),
        }
    }
}

/// The publisher of DeckPilot's own plugins.
pub const FIRST_PARTY_PUBLISHER: &str = "deckpilot";

/// Whether `id` is a first-party plugin id (`deckpilot.<name>`).
pub fn is_first_party(id: &PluginId) -> bool {
    id.publisher() == FIRST_PARTY_PUBLISHER
}

/// Refuses a source `id` may not come from ([`InstallSource::allows`]).
pub(crate) fn check_source(id: &PluginId, source: &InstallSource) -> Result<()> {
    if source.allows(id) {
        Ok(())
    } else {
        Err(StoreError::SourceNotAllowed {
            id: id.clone(),
            origin: source.as_str(),
        })
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

/// Automatic retries of a failed module migration: at the next starts, up
/// to this many failed attempts; after that only when the user asks
/// ([`PluginManager::retry_migration`](crate::PluginManager::retry_migration)).
pub const MAX_MIGRATION_ATTEMPTS: u32 = 3;

/// Longest `module_migrations.reason`, in characters.
pub const MAX_MIGRATION_REASON_CHARS: usize = 500;

/// A row of `module_migrations`: where one built-in module's move to its
/// first-party plugin stands (`docs/marketplace/module-migration.md`
/// §5.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleMigration {
    /// The built-in module (`github`).
    pub module_id: String,
    /// Its plugin (`deckpilot.github`).
    pub plugin_id: PluginId,
    pub phase: MigrationPhase,
    /// Why it is not eligible or failed. Never a value or a secret.
    pub reason: Option<String>,
    /// Failed attempts since the last cut-over.
    pub attempts: u32,
    /// sha256 (hex) of the module's settings row and credential ids at the
    /// settings copy, never of a secret; tells a later start about drift.
    pub source_digest: Option<String>,
    /// The app version that last recorded a step.
    pub app_version: String,
    pub updated_at: i64,
}

impl ModuleMigration {
    /// Whether the engine may retry it on its own at the next start: it
    /// failed fewer than [`MAX_MIGRATION_ATTEMPTS`] times.
    pub fn retries_automatically(&self) -> bool {
        self.phase == MigrationPhase::Failed && self.attempts < MAX_MIGRATION_ATTEMPTS
    }
}

/// One migration step to record
/// ([`PluginManager::record_migration`](crate::PluginManager::record_migration)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStep {
    pub module_id: String,
    pub plugin_id: PluginId,
    pub phase: MigrationPhase,
    pub reason: Option<String>,
    /// Replaces the recorded digest when given; `None` keeps it.
    pub source_digest: Option<String>,
    pub app_version: String,
}

/// A row of `plugin_tombstones`: the user explicitly uninstalled or
/// disabled a first-party plugin, so nothing reinstalls or re-enables it on
/// its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub plugin_id: PluginId,
    pub decision: TombstoneDecision,
    pub decided_at: i64,
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
        /// What the uninstall keeps; absent in journals of older builds
        /// (nothing kept).
        #[serde(default)]
        keep: crate::data::Retention,
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

    /// The origins the user entered and consented to (G2), one per origin
    /// setting. What the broker may use of them is decided by the agent
    /// against the current settings and manifest.
    pub fn user_origins(&self, id: &PluginId) -> Result<Vec<crate::origins::StoredUserOrigin>> {
        crate::origins::load(self.connection(), id)
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

    /// Every plugin ever installed here, installed or not (`plugin_known`).
    pub fn known(&self) -> Result<Vec<crate::data::KnownPlugin>> {
        crate::data::known(self.connection())
    }

    /// Which credential slots of `id` have a secret (never the secrets).
    pub fn credentials(&self, id: &PluginId) -> Result<Vec<crate::data::CredentialStatus>> {
        crate::data::credentials(self.connection(), id)
    }

    /// The settings values the user saved for `id` (without defaults).
    pub fn settings(
        &self,
        id: &PluginId,
    ) -> Result<std::collections::BTreeMap<String, serde_json::Value>> {
        crate::data::settings(self.connection(), id)
    }

    /// What `id`'s storage holds.
    pub fn storage_usage(&self, id: &PluginId) -> Result<crate::data::StorageUsage> {
        crate::data::storage_usage(self.connection(), id)
    }

    /// `id`'s storage keys.
    pub fn storage_keys(&self, id: &PluginId) -> Result<Vec<String>> {
        crate::data::storage_keys(self.connection(), id, None)
    }

    /// The host-held value `key` (schema 6), if one is stored: never a
    /// plugin's, and never handed to one.
    pub fn host_setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let text: Option<String> = self
            .connection()
            .query_row(
                "SELECT value_json FROM host_settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        text.map(|t| serde_json::from_str(&t).map_err(StoreError::record))
            .transpose()
    }

    /// The migration record of built-in module `module_id`, if any.
    pub fn module_migration(&self, module_id: &str) -> Result<Option<ModuleMigration>> {
        load_migration(self.connection(), module_id)
    }

    /// Every module migration record, by module id.
    pub fn module_migrations(&self) -> Result<Vec<ModuleMigration>> {
        let mut statement = self
            .connection()
            .prepare("SELECT module_id FROM module_migrations ORDER BY module_id")?;
        let ids: Vec<String> = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut migrations = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(migration) = load_migration(self.connection(), &id)? {
                migrations.push(migration);
            }
        }
        Ok(migrations)
    }

    /// `id`'s tombstone, if the user removed or disabled it. Seeding and
    /// migration check this first and leave a tombstoned plugin alone.
    pub fn tombstone(&self, id: &PluginId) -> Result<Option<Tombstone>> {
        load_tombstone(self.connection(), id)
    }

    /// The plugins installed only as another plugin's dependency, with
    /// the plugin whose install brought each one in. Every other installed
    /// plugin was installed by the user.
    pub fn dependency_installs(&self) -> Result<BTreeMap<PluginId, String>> {
        let mut statement = self.connection().prepare(
            "SELECT plugin_id, required_by FROM plugin_dependency_installs ORDER BY plugin_id",
        )?;
        let rows: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(id, by)| Ok((parse_id(&id)?, by)))
            .collect()
    }

    /// Every tombstone, by plugin id.
    pub fn tombstones(&self) -> Result<Vec<Tombstone>> {
        let mut statement = self.connection().prepare(
            "SELECT plugin_id, decision, decided_at FROM plugin_tombstones ORDER BY plugin_id",
        )?;
        let rows: Vec<(String, String, i64)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(id, decision, decided_at)| tombstone_of(&id, &decision, decided_at))
            .collect()
    }
}

// --------------------------------------------- module migrations, tombstones

fn tombstone_of(id: &str, decision: &str, decided_at: i64) -> Result<Tombstone> {
    Ok(Tombstone {
        plugin_id: parse_id(id)?,
        decision: TombstoneDecision::parse(decision)
            .ok_or_else(|| StoreError::record(format!("tombstone decision {decision:?}")))?,
        decided_at,
    })
}

pub(crate) fn load_tombstone(conn: &Connection, id: &PluginId) -> Result<Option<Tombstone>> {
    let row = conn
        .query_row(
            "SELECT decision, decided_at FROM plugin_tombstones WHERE plugin_id = ?1",
            [id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    row.map(|(decision, at)| tombstone_of(id.as_str(), &decision, at))
        .transpose()
}

/// Refuses to (re)install or enable a tombstoned plugin: only the user
/// clears a tombstone.
pub(crate) fn ensure_not_tombstoned(conn: &Connection, id: &PluginId) -> Result<()> {
    match load_tombstone(conn, id)? {
        Some(tombstone) => Err(StoreError::Tombstoned {
            id: id.clone(),
            decision: tombstone.decision,
        }),
        None => Ok(()),
    }
}

/// Writes (or replaces) `id`'s tombstone. Only first-party plugins have
/// one: nothing installs any other plugin on its own.
pub(crate) fn set_tombstone(
    conn: &Connection,
    id: &PluginId,
    decision: TombstoneDecision,
    now: i64,
) -> Result<()> {
    if !is_first_party(id) {
        return Err(StoreError::InvalidData(format!(
            "only first-party plugins have tombstones, not {id}"
        )));
    }
    conn.execute(
        "INSERT INTO plugin_tombstones (plugin_id, decision, decided_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (plugin_id) DO UPDATE SET
             decision = excluded.decision, decided_at = excluded.decided_at",
        params![id.as_str(), decision.as_str(), now],
    )?;
    Ok(())
}

pub(crate) fn delete_tombstone(conn: &Connection, id: &PluginId) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM plugin_tombstones WHERE plugin_id = ?1",
        [id.as_str()],
    )? == 1)
}

type MigrationRow = (
    String,
    String,
    Option<String>,
    i64,
    Option<String>,
    String,
    i64,
);

pub(crate) fn load_migration(
    conn: &Connection,
    module_id: &str,
) -> Result<Option<ModuleMigration>> {
    let row: Option<MigrationRow> = conn
        .query_row(
            "SELECT plugin_id, phase, reason, attempts, source_digest, app_version, updated_at
             FROM module_migrations WHERE module_id = ?1",
            [module_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((plugin_id, phase, reason, attempts, source_digest, app_version, updated_at)) = row
    else {
        return Ok(None);
    };
    Ok(Some(ModuleMigration {
        module_id: module_id.to_owned(),
        plugin_id: parse_id(&plugin_id)?,
        phase: MigrationPhase::parse(&phase)
            .ok_or_else(|| StoreError::record(format!("migration phase {phase:?}")))?,
        reason,
        attempts: u32::try_from(attempts).map_err(StoreError::record)?,
        source_digest,
        app_version,
        updated_at,
    }))
}

/// A built-in module id: lowercase letters, digits and hyphens (`github`,
/// `work-session`).
fn is_module_id(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn check_step(step: &MigrationStep) -> Result<()> {
    let invalid = |message: String| Err(StoreError::InvalidData(message));
    if !is_module_id(&step.module_id) {
        return invalid(format!("{:?} is not a module id", step.module_id));
    }
    if !is_first_party(&step.plugin_id) {
        return invalid(format!(
            "module {} can only migrate to a first-party plugin, not {}",
            step.module_id, step.plugin_id
        ));
    }
    if let Some(reason) = &step.reason
        && (reason.chars().count() > MAX_MIGRATION_REASON_CHARS
            || reason.chars().any(char::is_control))
    {
        return invalid(format!(
            "a migration reason is one line of at most {MAX_MIGRATION_REASON_CHARS} characters"
        ));
    }
    if let Some(digest) = &step.source_digest
        && (digest.len() != 64
            || !digest
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)))
    {
        return invalid("a source digest is a lowercase hex sha256".into());
    }
    if step.app_version.is_empty()
        || step.app_version.len() > 64
        || step.app_version.chars().any(char::is_control)
    {
        return invalid("an app version is 1 to 64 printable characters".into());
    }
    Ok(())
}

/// Records `step`, idempotently: recording the phase a module is already
/// in, with the same reason, digest and app version, changes nothing (not
/// even `updated_at`). A `Failed` step counts one more attempt; a cut-over
/// resets the count. A module keeps the plugin it was first recorded with.
pub(crate) fn record_migration(
    conn: &Connection,
    step: &MigrationStep,
    now: i64,
) -> Result<ModuleMigration> {
    check_step(step)?;
    let existing = load_migration(conn, &step.module_id)?;
    if let Some(existing) = &existing
        && existing.plugin_id != step.plugin_id
    {
        return Err(StoreError::InvalidData(format!(
            "module {} migrates to {}, not {}",
            step.module_id, existing.plugin_id, step.plugin_id
        )));
    }
    let source_digest = step
        .source_digest
        .clone()
        .or_else(|| existing.as_ref().and_then(|e| e.source_digest.clone()));
    let attempts = match (step.phase, &existing) {
        (MigrationPhase::Failed, Some(existing)) => existing.attempts.saturating_add(1),
        (MigrationPhase::Failed, None) => 1,
        (MigrationPhase::CutOver, _) | (_, None) => 0,
        (_, Some(existing)) => existing.attempts,
    };
    let next = ModuleMigration {
        module_id: step.module_id.clone(),
        plugin_id: step.plugin_id.clone(),
        phase: step.phase,
        reason: step.reason.clone(),
        attempts,
        source_digest,
        app_version: step.app_version.clone(),
        updated_at: now,
    };
    let unchanged = |existing: &ModuleMigration| {
        let same = ModuleMigration {
            updated_at: existing.updated_at,
            ..next.clone()
        };
        same == *existing
    };
    if let Some(existing) = existing
        && step.phase != MigrationPhase::Failed
        && unchanged(&existing)
    {
        return Ok(existing);
    }
    write_migration(conn, &next)?;
    Ok(next)
}

fn write_migration(conn: &Connection, migration: &ModuleMigration) -> Result<()> {
    conn.execute(
        "INSERT INTO module_migrations (module_id, plugin_id, phase, reason, attempts,
             source_digest, app_version, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (module_id) DO UPDATE SET
             phase = excluded.phase, reason = excluded.reason, attempts = excluded.attempts,
             source_digest = excluded.source_digest, app_version = excluded.app_version,
             updated_at = excluded.updated_at",
        params![
            migration.module_id,
            migration.plugin_id.as_str(),
            migration.phase.as_str(),
            migration.reason,
            migration.attempts,
            migration.source_digest,
            migration.app_version,
            migration.updated_at,
        ],
    )?;
    Ok(())
}

/// The user's Retry: back to `Pending` with no failed attempts, keeping the
/// digest. `None` when the module has no record.
pub(crate) fn retry_migration(
    conn: &Connection,
    module_id: &str,
    now: i64,
) -> Result<Option<ModuleMigration>> {
    let Some(existing) = load_migration(conn, module_id)? else {
        return Ok(None);
    };
    let next = ModuleMigration {
        phase: MigrationPhase::Pending,
        reason: None,
        attempts: 0,
        updated_at: now,
        ..existing
    };
    write_migration(conn, &next)?;
    Ok(Some(next))
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
    check_source(id, source)?;
    let (source, registry) = source.column();
    conn.execute(
        "INSERT INTO plugins (plugin_id, state, source, registry, installed_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![id.as_str(), state.as_str(), source, registry, now],
    )?;
    Ok(())
}

pub(crate) fn set_source(conn: &Connection, id: &PluginId, source: &InstallSource) -> Result<()> {
    check_source(id, source)?;
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
    // Every plugin that was ever installed keeps its name (design §2.3
    // item 4), in the same transaction as its version.
    crate::data::upsert_known(conn, &pending.manifest()?, now)
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

// ------------------------------------------------------ dependency installs

pub(crate) fn set_dependency_install(
    conn: &Connection,
    id: &PluginId,
    required_by: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO plugin_dependency_installs (plugin_id, required_by, installed_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (plugin_id) DO NOTHING",
        params![id.as_str(), required_by, now],
    )?;
    Ok(())
}

pub(crate) fn delete_dependency_install(conn: &Connection, id: &PluginId) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM plugin_dependency_installs WHERE plugin_id = ?1",
        params![id.as_str()],
    )? > 0)
}
