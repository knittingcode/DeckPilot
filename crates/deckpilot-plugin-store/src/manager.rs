//! Lifecycle operations (architecture sections 12 and 13).
//!
//! Each multi-step operation (install, update, uninstall) first commits a
//! journal entry together with its transient state, then performs its file
//! system steps (staging + rename, or rename into `.trash`), then commits
//! one transaction that writes the final records **and deletes the journal
//! entry**. An entry that survives therefore means "not finished", and
//! [`PluginManager::recover`] completes or undoes it. Single-transaction
//! operations (enable, disable, grants, approve, decline, rollback,
//! quarantine) need no journal; their file clean-up after the commit is
//! repeated by recovery's orphan sweep.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_package::{VerifiedPackage, verify_installed};
use deckpilot_plugin_registry::{InstalledVerdict, RegistryClient, TrustStore};
use rusqlite::Connection;
use semver::Version;

use crate::data::{self, Retention};
use crate::db::{DEFAULT_BUSY_TIMEOUT, StoreKey};
use crate::error::{Result, StoreError};
use crate::failpoint::{Failpoint, Failpoints, NoFailpoints, Operation};
use crate::grants::{
    Consent, GrantKey, GrantSet, GrantSource, GrantStatus, PermissionDiff, StoredGrant,
};
use crate::layout::Layout;
use crate::state::{PluginState, TombstoneDecision};
use crate::store::{
    self, Audit, AuditRetention, InstallSource, JournalEntry, MigrationStep, ModuleMigration,
    Payload, PendingVersion, PluginRecord, PluginStore, VersionRecord, VersionRole,
};

/// Clean-up the store cannot do itself.
pub trait PluginHooks: Send + Sync + fmt::Debug {
    /// Deletes the plugin's secret values (kept outside this database, by
    /// handle). Called during uninstall, possibly more than once after a
    /// crash, so it must be idempotent.
    fn delete_secrets(&self, id: &PluginId) -> std::result::Result<(), String>;
}

/// No secrets to delete.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHooks;

impl PluginHooks for NoHooks {
    fn delete_secrets(&self, _id: &PluginId) -> std::result::Result<(), String> {
        Ok(())
    }
}

/// Where revocation verdicts come from: the verified registry state
/// ([`RegistryClient`]), which also holds offline.
pub trait VerdictSource {
    fn verdict(&self, id: &PluginId, version: &Version) -> InstalledVerdict;
}

impl<S: TrustStore> VerdictSource for RegistryClient<S> {
    fn verdict(&self, id: &PluginId, version: &Version) -> InstalledVerdict {
        self.installed_verdict(id, version)
    }
}

/// Seconds since the Unix epoch.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// How a manager behaves; the defaults are for production.
#[derive(Clone)]
pub struct ManagerOptions {
    pub busy_timeout: Duration,
    pub failpoints: Arc<dyn Failpoints>,
    pub hooks: Arc<dyn PluginHooks>,
    pub clock: Clock,
    pub audit: AuditRetention,
}

impl Default for ManagerOptions {
    fn default() -> Self {
        Self {
            busy_timeout: DEFAULT_BUSY_TIMEOUT,
            failpoints: Arc::new(NoFailpoints),
            hooks: Arc::new(NoHooks),
            clock: Arc::new(crate::system_now),
            audit: AuditRetention::default(),
        }
    }
}

impl fmt::Debug for ManagerOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagerOptions")
            .field("busy_timeout", &self.busy_timeout)
            .field("failpoints", &self.failpoints)
            .field("hooks", &self.hooks)
            .field("audit", &self.audit)
            .finish_non_exhaustive()
    }
}

/// The result of [`PluginManager::update`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// No escalation: the new version is active (narrowed grants applied).
    Applied {
        version: Version,
        diff: PermissionDiff,
    },
    /// Escalation: the new version is staged; the running version is
    /// unchanged until [`PluginManager::approve_update`].
    AwaitingConsent {
        version: Version,
        diff: PermissionDiff,
    },
}

/// What [`PluginManager::apply_verdicts`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictChange {
    Quarantined { id: PluginId, reason: String },
    Released { id: PluginId },
    StagedDropped { id: PluginId, version: Version },
}

/// Values a transaction needs from the manager (taken before the
/// transaction borrows the store).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ctx {
    pub now: i64,
    pub retention: AuditRetention,
}

impl Ctx {
    pub(crate) fn audit(self, conn: &Connection, id: &PluginId, event: Audit<'_>) -> Result<()> {
        store::audit(conn, id, event, self.now, self.retention)
    }
}

/// The single writer of the plugin database and directories.
///
/// Holds an exclusive lock on `<data dir>/plugins/.lock` for its lifetime,
/// so one process at a time manages plugins; every write is a
/// `BEGIN IMMEDIATE` transaction with a busy timeout, so readers in other
/// processes ([`PluginStore::open`]) never see a half-written step (a
/// running operation shows as its transient state). Call
/// [`PluginManager::recover`] once after opening.
#[derive(Debug)]
pub struct PluginManager {
    pub(crate) store: PluginStore,
    pub(crate) layout: Layout,
    _lock: File,
    pub(crate) options: ManagerOptions,
}

impl PluginManager {
    /// Opens the plugin store under `data_dir` (`<data_dir>/plugins`).
    pub fn open(data_dir: &Path, key: &StoreKey, options: ManagerOptions) -> Result<Self> {
        let layout = Layout::new(data_dir);
        layout.ensure()?;
        let lock = layout.lock()?;
        let store = PluginStore::open(&layout.database(), key, options.busy_timeout)?;
        Ok(Self {
            store,
            layout,
            _lock: lock,
            options,
        })
    }

    pub fn store(&self) -> &PluginStore {
        &self.store
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub(crate) fn ctx(&self) -> Ctx {
        Ctx {
            now: (self.options.clock)(),
            retention: self.options.audit,
        }
    }

    pub(crate) fn crash(&self, op: Operation, point: Failpoint) -> Result<()> {
        if self.options.failpoints.should_crash(op, point) {
            Err(StoreError::Crashed { op, point })
        } else {
            Ok(())
        }
    }

    /// Runs `f` in one `BEGIN IMMEDIATE` transaction, committed on success.
    pub(crate) fn tx<T>(&mut self, f: impl FnOnce(&Connection, Ctx) -> Result<T>) -> Result<T> {
        let ctx = self.ctx();
        let tx = self.store.db().write()?;
        let value = f(&tx, ctx)?;
        tx.commit()?;
        Ok(value)
    }

    fn plugin(&self, id: &PluginId) -> Result<PluginRecord> {
        self.store
            .plugin(id)?
            .ok_or_else(|| StoreError::NotInstalled(id.clone()))
    }

    // ---------------------------------------------------------------- install

    /// Installs a verified package as a new plugin, in state `Disabled`,
    /// recording `consent` as its grants.
    ///
    /// Only a [`VerifiedPackage`] is accepted (its bytes matched the
    /// registry pins and its contents passed every check); the unpacked
    /// directory is verified again against its integrity record before the
    /// install is committed.
    pub fn install(
        &mut self,
        package: &VerifiedPackage,
        source: &InstallSource,
        consent: &Consent,
    ) -> Result<PluginRecord> {
        let id = package.manifest().id.clone();
        store::check_source(&id, source)?;
        let declared = GrantSet::from_manifest(package.manifest());
        check_privileged_source(&id, &declared, source)?;
        check_consent(&id, &declared, consent)?;
        if *source == InstallSource::Bundled {
            check_bundled_consent(&id, &declared, consent)?;
        }
        let pending = pending_version(package)?;
        let payload = Payload::Install {
            package: pending.clone(),
            source: source.clone(),
            consent: consent.clone(),
        };
        self.tx(|tx, ctx| {
            ensure_idle(tx, &id)?;
            if let Some(existing) = store::load_plugin(tx, &id)? {
                return Err(StoreError::AlreadyInstalled {
                    id: id.clone(),
                    version: existing
                        .active()
                        .map(|v| v.version.to_string())
                        .unwrap_or_default(),
                });
            }
            store::ensure_not_tombstoned(tx, &id)?;
            store::insert_plugin(tx, &id, PluginState::Installing, source, ctx.now)?;
            store::insert_journal(tx, &id, &payload, ctx.now)
        })?;
        let result = self.install_steps(&id, package, &pending, consent);
        self.settle(&id, result)?;
        self.plugin(&id)
    }

    fn install_steps(
        &mut self,
        id: &PluginId,
        package: &VerifiedPackage,
        pending: &PendingVersion,
        consent: &Consent,
    ) -> Result<()> {
        let op = Operation::Install;
        self.crash(op, Failpoint::JournalWritten)?;
        self.unpack(id, package, pending)?;
        self.crash(op, Failpoint::FilesUnpacked)?;
        self.tx(|tx, ctx| store::set_journal_step(tx, id, "unpacked", ctx.now))?;
        self.crash(op, Failpoint::StepRecorded)?;
        self.tx(|tx, ctx| commit_install(tx, ctx, id, pending, consent))?;
        self.crash(op, Failpoint::Committed)?;
        self.clean_up()
    }

    /// After a journaled operation: an injected crash is returned as is
    /// (nothing may clean up); any other error undoes the operation at
    /// once, as recovery would.
    fn settle<T>(&mut self, id: &PluginId, result: Result<T>) -> Result<T> {
        match result {
            Err(error) if !error.is_crash() => {
                // The original error is what the caller needs. If undoing
                // fails too, the journal entry stays and recovery retries.
                if let Ok(Some(entry)) = store::journal_for(self.store.connection(), id)
                    && entry.op != Operation::Uninstall
                {
                    let _ = self.undo(id, &entry.payload);
                }
                Err(error)
            }
            other => other,
        }
    }

    /// Unpacks into `<id>/<version>` through `.staging`, replacing a
    /// leftover directory no record refers to, and verifies the result.
    fn unpack(
        &mut self,
        id: &PluginId,
        package: &VerifiedPackage,
        pending: &PendingVersion,
    ) -> Result<()> {
        let target = self.layout.resolve(&pending.install_dir);
        self.layout.move_to_trash(&target)?;
        package.unpack(&self.layout.staging(), &target)?;
        verify_installed(&target, package.integrity()).map_err(|e| StoreError::Integrity {
            id: id.clone(),
            version: pending.version.to_string(),
            problem: e.to_string(),
        })
    }

    pub(crate) fn clean_up(&mut self) -> Result<()> {
        self.layout.empty_staging()?;
        self.layout.empty_trash()?;
        Ok(())
    }

    // ------------------------------------------------------- enable, disable

    /// `Disabled → Enabled`. Every declared grant but the optional ones
    /// (`requiresPermissions`, plugin API 1.9) must be granted (with at
    /// least the declared scope) and the files must match their record;
    /// a mismatch marks the plugin `Broken`.
    pub fn enable(&mut self, id: &PluginId) -> Result<()> {
        let plugin = self.plugin(id)?;
        if !plugin.state.can_transition_to(PluginState::Enabled) {
            return Err(StoreError::IllegalTransition {
                id: id.clone(),
                from: plugin.state,
                to: PluginState::Enabled,
            });
        }
        store::ensure_not_tombstoned(self.store.connection(), id)?;
        let active = active_of(&plugin)?;
        let missing = crate::grants::missing_grants(&self.store.grants(id)?, &active.grants());
        if !missing.is_empty() {
            return Err(StoreError::ConsentRequired {
                id: id.clone(),
                missing,
            });
        }
        self.verify(id)?;
        let version = active.version.to_string();
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            store::ensure_not_tombstoned(tx, id)?;
            store::transition(tx, id, plugin.state, PluginState::Enabled, None, ctx.now)?;
            ctx.audit(tx, id, event(Some(&version), "enable", None, "ok", None))
        })
    }

    /// `Enabled → Disabled`, for the agent's own reasons (a crash loop, a
    /// broker failure). Writes no tombstone: a user's choice goes through
    /// [`disable_by_user`](Self::disable_by_user).
    pub fn disable(&mut self, id: &PluginId) -> Result<()> {
        self.disable_as(id, false)
    }

    /// `Enabled → Disabled` because the user asked. A first-party plugin
    /// also gets a `disabled` tombstone in the same transaction, so seeding
    /// and migration never enable it again; [`enable`](Self::enable) is
    /// refused until the user clears it
    /// ([`clear_tombstone`](Self::clear_tombstone)).
    pub fn disable_by_user(&mut self, id: &PluginId) -> Result<()> {
        self.disable_as(id, true)
    }

    fn disable_as(&mut self, id: &PluginId, by_user: bool) -> Result<()> {
        let plugin = self.plugin(id)?;
        let tombstone = by_user && store::is_first_party(id);
        // A first-party plugin seeded disabled (waiting for consent) can
        // still be turned down for good.
        let already = tombstone && plugin.state == PluginState::Disabled;
        if plugin.state != PluginState::Enabled && !already {
            return Err(StoreError::IllegalTransition {
                id: id.clone(),
                from: plugin.state,
                to: PluginState::Disabled,
            });
        }
        let version = plugin.active().map(|v| v.version.to_string());
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            if !already {
                store::transition(tx, id, plugin.state, PluginState::Disabled, None, ctx.now)?;
                ctx.audit(
                    tx,
                    id,
                    event(version.as_deref(), "disable", None, "ok", None),
                )?;
            }
            if tombstone {
                store::set_tombstone(tx, id, TombstoneDecision::Disabled, ctx.now)?;
                ctx.audit(
                    tx,
                    id,
                    event(version.as_deref(), "tombstone", None, "disabled", None),
                )?;
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------- tombstones

    /// The user's undo of an uninstall or disable of a first-party plugin
    /// (installing or enabling it again in Settings › Plugins): removes its
    /// tombstone, so [`install`](Self::install) and
    /// [`enable`](Self::enable) work again. Never called by seeding or
    /// migration. Returns whether there was one.
    pub fn clear_tombstone(&mut self, id: &PluginId) -> Result<bool> {
        self.tx(|tx, ctx| {
            let removed = store::delete_tombstone(tx, id)?;
            if removed {
                ctx.audit(tx, id, event(None, "tombstone", None, "cleared", None))?;
            }
            Ok(removed)
        })
    }

    // -------------------------------------------------------- bundled restore

    /// The user's restore of a first-party plugin they uninstalled
    /// (Settings › Plugins › Restore, `restoreBundledPlugin`): clears its
    /// tombstone and installs `package` (the copy of the application bundle,
    /// verified by the caller against the pins compiled into the agent) as
    /// source `bundled` with `consent` (low-tier keys only, as seeding).
    /// One step for the user: if the install fails, the tombstone is written
    /// back, so nothing changed. Settings and credentials kept at the
    /// uninstall belong to the plugin again. Refused for a plugin that is
    /// installed ([`StoreError::AlreadyInstalled`]).
    pub fn restore_by_user(
        &mut self,
        package: &VerifiedPackage,
        consent: &Consent,
    ) -> Result<PluginRecord> {
        let id = package.manifest().id.clone();
        store::check_source(&id, &InstallSource::Bundled)?;
        let version = package.manifest().version.to_string();
        let removed = self.tx(|tx, ctx| {
            ensure_idle(tx, &id)?;
            if let Some(existing) = store::load_plugin(tx, &id)? {
                return Err(StoreError::AlreadyInstalled {
                    id: id.clone(),
                    version: existing
                        .active()
                        .map(|v| v.version.to_string())
                        .unwrap_or_default(),
                });
            }
            let tombstone = store::load_tombstone(tx, &id)?;
            if tombstone.is_some() {
                store::delete_tombstone(tx, &id)?;
                ctx.audit(
                    tx,
                    &id,
                    event(
                        Some(&version),
                        "tombstone",
                        None,
                        "cleared",
                        Some("restore"),
                    ),
                )?;
            }
            Ok(tombstone.map(|t| t.decision))
        })?;
        match self.install(package, &InstallSource::Bundled, consent) {
            Err(error) if !error.is_crash() => {
                if let Some(decision) = removed {
                    // The user's earlier decision stands until a restore
                    // succeeds.
                    let _ = self.tx(|tx, ctx| {
                        store::set_tombstone(tx, &id, decision, ctx.now)?;
                        ctx.audit(
                            tx,
                            &id,
                            event(
                                Some(&version),
                                "tombstone",
                                None,
                                decision.as_str(),
                                Some("restore failed"),
                            ),
                        )
                    });
                }
                Err(error)
            }
            other => other,
        }
    }

    /// The user's reinstall of a broken bundled plugin (Settings › Plugins ›
    /// Reinstall, `restoreBundledPlugin`): uninstalls the broken copy
    /// (without a tombstone, keeping its settings and credentials) and
    /// installs `package` (verified by the caller against the pins compiled
    /// into the agent) as source `bundled` with `consent` (low-tier keys
    /// only). The plugin's earlier permission decisions (grants and
    /// revocations) and its consents to servers the user entered come back
    /// where the reinstalled version declares them alike. The result is
    /// `Disabled`; enabling it is the caller's decision. Only a `Broken`
    /// plugin of source `bundled` is reinstalled.
    pub fn reinstall_bundled_by_user(
        &mut self,
        package: &VerifiedPackage,
        consent: &Consent,
    ) -> Result<PluginRecord> {
        let id = package.manifest().id.clone();
        store::check_source(&id, &InstallSource::Bundled)?;
        let plugin = self.plugin(&id)?;
        if plugin.source != InstallSource::Bundled {
            return Err(StoreError::SourceNotAllowed {
                id,
                origin: plugin.source.as_str(),
            });
        }
        if plugin.state != PluginState::Broken {
            return Err(StoreError::IllegalTransition {
                id,
                from: plugin.state,
                to: PluginState::Installing,
            });
        }
        let grants = self.store.grants(&id)?;
        let origins = self.store.user_origins(&id)?;
        let broken = plugin.active().map(|v| v.version.to_string());
        self.tx(|tx, ctx| {
            ctx.audit(
                tx,
                &id,
                event(
                    broken.as_deref(),
                    "reinstall",
                    None,
                    "started",
                    plugin.state_reason.as_deref(),
                ),
            )
        })?;
        self.uninstall_keeping(
            &id,
            Retention {
                settings: true,
                credentials: true,
            },
        )?;
        let record = self.install(package, &InstallSource::Bundled, consent)?;
        let active = active_of(&record)?;
        let declared = active.grants();
        let manifest = active.manifest.clone();
        let version = active.version.to_string();
        self.tx(|tx, ctx| {
            for grant in &grants {
                if declared.get(&grant.key) != Some(&grant.scope) {
                    continue;
                }
                store::upsert_grant(
                    tx,
                    &id,
                    &StoredGrant {
                        granted_for_version: version.clone(),
                        ..grant.clone()
                    },
                )?;
                ctx.audit(
                    tx,
                    &id,
                    event(
                        Some(&version),
                        "grant",
                        Some(&grant.key),
                        grant.status.as_str(),
                        Some("kept at reinstall"),
                    ),
                )?;
            }
            for origin in &origins {
                if crate::origins::check(&id, &manifest, &origin.consent).is_err() {
                    continue;
                }
                crate::origins::upsert(
                    tx,
                    &id,
                    &crate::origins::StoredUserOrigin {
                        granted_for_version: version.clone(),
                        ..origin.clone()
                    },
                )?;
            }
            ctx.audit(
                tx,
                &id,
                event(Some(&version), "reinstall", None, "ok", None),
            )
        })?;
        self.plugin(&id)
    }

    // ------------------------------------------------------ module migrations

    /// Records a step of a built-in module's migration to its first-party
    /// plugin (`module_migrations`), idempotently: recording the phase the
    /// module is already in, with the same details, changes nothing. A
    /// `Failed` step counts an attempt
    /// ([`ModuleMigration::retries_automatically`]); a cut-over resets the
    /// count.
    pub fn record_migration(&mut self, step: &MigrationStep) -> Result<ModuleMigration> {
        self.tx(|tx, ctx| store::record_migration(tx, step, ctx.now))
    }

    /// Stores the host-held value `key` (schema 6; `None` removes it).
    /// Returns whether it changed. Host-held values belong to the agent:
    /// no plugin reads or writes them.
    ///
    /// # Errors
    ///
    /// For a key of more than 64 characters or with control characters,
    /// and when the store refuses.
    pub fn set_host_setting(
        &mut self,
        key: &str,
        value: Option<&serde_json::Value>,
    ) -> Result<bool> {
        if key.is_empty() || key.len() > 64 || key.chars().any(char::is_control) {
            return Err(StoreError::InvalidData(format!(
                "`{key}` is not a host setting key"
            )));
        }
        if self.store.host_setting(key)?.as_ref() == value {
            return Ok(false);
        }
        let json = value
            .map(|v| serde_json::to_string(v).map_err(StoreError::record))
            .transpose()?;
        self.tx(|tx, ctx| {
            match &json {
                Some(json) => tx.execute(
                    "INSERT INTO host_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT (key) DO UPDATE SET value_json = excluded.value_json,
                         updated_at = excluded.updated_at",
                    rusqlite::params![key, json, ctx.now],
                )?,
                None => tx.execute("DELETE FROM host_settings WHERE key = ?1", [key])?,
            };
            Ok(())
        })?;
        Ok(true)
    }

    /// The user's Retry of a migration: back to `Pending` with no failed
    /// attempts. `None` when the module has no record.
    pub fn retry_migration(&mut self, module_id: &str) -> Result<Option<ModuleMigration>> {
        self.tx(|tx, ctx| store::retry_migration(tx, module_id, ctx.now))
    }

    /// Checks the active version's files against their integrity record;
    /// a mismatch marks the plugin `Broken` and is returned as an error.
    pub fn verify(&mut self, id: &PluginId) -> Result<()> {
        let plugin = self.plugin(id)?;
        let active = active_of(&plugin)?.clone();
        let dir = self.layout.resolve(&active.install_dir);
        let Err(problem) = verify_installed(&dir, &active.integrity) else {
            return Ok(());
        };
        let problem = problem.to_string();
        self.mark_broken(id, &problem)?;
        Err(StoreError::Integrity {
            id: id.clone(),
            version: active.version.to_string(),
            problem,
        })
    }

    /// Marks the plugin `Broken` with `problem` (audited) when its state
    /// allows it: for integrity checks the store cannot make itself, such
    /// as the agent's check of a bundled plugin against the digests
    /// compiled into it.
    pub fn mark_broken(&mut self, id: &PluginId, problem: &str) -> Result<()> {
        self.tx(|tx, ctx| {
            let Some(plugin) = store::load_plugin(tx, id)? else {
                return Ok(());
            };
            if plugin.state.can_transition_to(PluginState::Broken) {
                let version = plugin.active().map(|v| v.version.to_string());
                store::transition(
                    tx,
                    id,
                    plugin.state,
                    PluginState::Broken,
                    Some(problem),
                    ctx.now,
                )?;
                ctx.audit(
                    tx,
                    id,
                    event(
                        version.as_deref(),
                        "integrity",
                        None,
                        "broken",
                        Some(problem),
                    ),
                )?;
            }
            Ok(())
        })
    }

    // ----------------------------------------------------------------- grants

    /// Grants declared permissions of the active version (Settings ›
    /// Plugins, or a later consent dialog).
    pub fn grant(&mut self, id: &PluginId, consent: &Consent) -> Result<()> {
        let plugin = self.plugin(id)?;
        let active = active_of(&plugin)?;
        let declared = active.grants();
        check_consent(id, &declared, consent)?;
        let version = active.version.to_string();
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            for key in &consent.keys {
                let scope = declared.get(key).cloned().ok_or_else(|| unknown(id, key))?;
                store::upsert_grant(
                    tx,
                    id,
                    &StoredGrant {
                        key: key.clone(),
                        scope,
                        status: GrantStatus::Granted,
                        source: GrantSource::User,
                        decided_by: consent.decided_by.clone(),
                        decided_at: ctx.now,
                        granted_for_version: version.clone(),
                    },
                )?;
                ctx.audit(
                    tx,
                    id,
                    event(Some(&version), "grant", Some(key), "granted", None),
                )?;
            }
            Ok(())
        })
    }

    /// Records grants that a decision other than the user's derived
    /// (`plugin_grants.source = 'derived'`): a module migration granting
    /// what the built-in demonstrably used on this computer
    /// (`docs/marketplace/module-migration.md` §5.5). Only low- and
    /// medium-tier permissions; a high-tier one (a credential, the private
    /// network) is refused, it always needs the user's consent. A key
    /// already granted is left as it is, so the user's own decision and a
    /// revocation are never overwritten: a revoked key stays revoked.
    /// Returns the keys it granted.
    pub fn derive_grants(
        &mut self,
        id: &PluginId,
        keys: &BTreeSet<GrantKey>,
        decided_by: &str,
    ) -> Result<Vec<GrantKey>> {
        let plugin = self.plugin(id)?;
        let active = active_of(&plugin)?;
        let declared = active.grants();
        let consent = Consent::of(keys.iter().cloned(), decided_by);
        check_consent(id, &declared, &consent)?;
        check_bundled_tiers(id, &declared, &consent, crate::grants::Tier::Medium)?;
        let version = active.version.to_string();
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            let stored = store::load_grants(tx, id)?;
            let mut granted = Vec::new();
            for key in keys {
                if stored.iter().any(|g| &g.key == key) {
                    continue;
                }
                let scope = declared.get(key).cloned().ok_or_else(|| unknown(id, key))?;
                store::upsert_grant(
                    tx,
                    id,
                    &StoredGrant {
                        key: key.clone(),
                        scope,
                        status: GrantStatus::Granted,
                        source: GrantSource::Derived,
                        decided_by: decided_by.to_owned(),
                        decided_at: ctx.now,
                        granted_for_version: version.clone(),
                    },
                )?;
                ctx.audit(
                    tx,
                    id,
                    event(Some(&version), "grant", Some(key), "derived", None),
                )?;
                granted.push(key.clone());
            }
            Ok(granted)
        })
    }

    /// Revokes one declared permission. The plugin keeps its state; the
    /// broker refuses the permission from now on (section 5.6), and
    /// enabling again requires a new grant.
    pub fn revoke(&mut self, id: &PluginId, key: &GrantKey, decided_by: &str) -> Result<()> {
        let plugin = self.plugin(id)?;
        let active = active_of(&plugin)?;
        let scope = active
            .grants()
            .get(key)
            .cloned()
            .ok_or_else(|| unknown(id, key))?;
        let version = active.version.to_string();
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            store::upsert_grant(
                tx,
                id,
                &StoredGrant {
                    key: key.clone(),
                    scope,
                    status: GrantStatus::Revoked,
                    source: GrantSource::User,
                    decided_by: decided_by.to_owned(),
                    decided_at: ctx.now,
                    granted_for_version: version.clone(),
                },
            )?;
            ctx.audit(
                tx,
                id,
                event(Some(&version), "revoke", Some(key), "revoked", None),
            )
        })
    }

    /// Records the user's consent to the origin they entered in an origin
    /// setting (G2), replacing an earlier one for that setting. The active
    /// version must declare the setting as a user-origin slot that allows
    /// what the consent lists, and the slot itself must be granted
    /// (`network:user-origin:<setting>`). Consent to a private network is
    /// recorded only as part of this explicit consent, never derived.
    pub fn grant_user_origin(
        &mut self,
        id: &PluginId,
        consent: &crate::origins::UserOriginConsent,
        decided_by: &str,
    ) -> Result<()> {
        let plugin = self.plugin(id)?;
        let active = active_of(&plugin)?;
        crate::origins::check(id, &active.manifest, consent)?;
        let key = crate::origins::user_origin_key(&consent.setting);
        let declared = active.grants();
        let slot_granted = self.store.grants(id)?.iter().any(|row| {
            row.key == key
                && row.status == GrantStatus::Granted
                && declared
                    .get(&key)
                    .is_some_and(|scope| row.scope.covers(scope))
        });
        if !slot_granted {
            return Err(StoreError::ConsentRequired {
                id: id.clone(),
                missing: vec![key],
            });
        }
        let version = active.version.to_string();
        let shown = format!(
            "{}{}",
            consent.display(),
            if consent.private_network {
                " (private network)"
            } else {
                ""
            }
        );
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            crate::origins::upsert(
                tx,
                id,
                &crate::origins::StoredUserOrigin {
                    consent: consent.clone(),
                    decided_by: decided_by.to_owned(),
                    decided_at: ctx.now,
                    granted_for_version: version.clone(),
                },
            )?;
            ctx.audit(
                tx,
                id,
                event(Some(&version), "grant", Some(&key), "granted", Some(&shown)),
            )
        })
    }

    /// Deletes the consent to the origin of one origin setting (G2): the
    /// user revoked it, or the setting changed (`why`). Returns the origin
    /// that was granted, if any.
    pub fn revoke_user_origin(
        &mut self,
        id: &PluginId,
        setting: &str,
        why: &str,
    ) -> Result<Option<String>> {
        let version = self.plugin(id)?.active().map(|v| v.version.to_string());
        let key = crate::origins::user_origin_key(setting);
        self.tx(|tx, ctx| {
            let Some(shown) = crate::origins::delete(tx, id, setting)? else {
                return Ok(None);
            };
            let reason = format!("{shown}: {why}");
            ctx.audit(
                tx,
                id,
                event(
                    version.as_deref(),
                    "revoke",
                    Some(&key),
                    "revoked",
                    Some(&reason),
                ),
            )?;
            Ok(Some(shown))
        })
    }

    /// Forgets the refusals that `decided_by` recorded (revoked grants it
    /// decided), so those permissions are undecided again: a module
    /// migration asking again for what the user declined in its consent
    /// dialog. It never grants anything; grants, and refusals anybody else
    /// decided, stay as they are. Returns the keys it forgot.
    pub fn forget_declines(&mut self, id: &PluginId, decided_by: &str) -> Result<Vec<GrantKey>> {
        let plugin = self.plugin(id)?;
        let version = plugin.active().map(|v| v.version.to_string());
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            let mut forgotten = Vec::new();
            for grant in store::load_grants(tx, id)? {
                if grant.status != GrantStatus::Revoked || grant.decided_by != decided_by {
                    continue;
                }
                store::delete_grant(tx, id, &grant.key)?;
                ctx.audit(
                    tx,
                    id,
                    event(
                        version.as_deref(),
                        "revoke",
                        Some(&grant.key),
                        "undecided",
                        None,
                    ),
                )?;
                forgotten.push(grant.key);
            }
            Ok(forgotten)
        })
    }

    // ----------------------------------------------------------------- update

    /// Unpacks a newer verified version next to the running one. Without a
    /// permission escalation it becomes active at once (narrowed grants
    /// applied); with one it stays staged until
    /// [`approve_update`](Self::approve_update).
    pub fn update(
        &mut self,
        package: &VerifiedPackage,
        source: &InstallSource,
    ) -> Result<UpdateOutcome> {
        let id = package.manifest().id.clone();
        store::check_source(&id, source)?;
        check_privileged_source(&id, &GrantSet::from_manifest(package.manifest()), source)?;
        let pending = pending_version(package)?;
        self.tx(|tx, ctx| {
            ensure_idle(tx, &id)?;
            let plugin =
                store::load_plugin(tx, &id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
            if let Some(staged) = plugin.staged() {
                return Err(StoreError::UpdatePending {
                    id: id.clone(),
                    version: staged.version.to_string(),
                });
            }
            let new = pending.version.to_string();
            if plugin
                .versions
                .iter()
                .any(|v| v.version.to_string().eq_ignore_ascii_case(&new))
            {
                return Err(StoreError::VersionInstalled {
                    id: id.clone(),
                    version: new,
                });
            }
            if let Some(active) = plugin.active()
                && active.version >= pending.version
            {
                return Err(StoreError::NotNewer {
                    id: id.clone(),
                    version: new,
                    active: active.version.to_string(),
                });
            }
            store::transition(tx, &id, plugin.state, PluginState::Updating, None, ctx.now)?;
            store::insert_journal(
                tx,
                &id,
                &Payload::Update {
                    package: pending.clone(),
                    source: source.clone(),
                    prior_state: plugin.state,
                },
                ctx.now,
            )
        })?;
        let result = self.update_steps(&id, package, &pending, source);
        self.settle(&id, result)
    }

    fn update_steps(
        &mut self,
        id: &PluginId,
        package: &VerifiedPackage,
        pending: &PendingVersion,
        source: &InstallSource,
    ) -> Result<UpdateOutcome> {
        let op = Operation::Update;
        self.crash(op, Failpoint::JournalWritten)?;
        self.unpack(id, package, pending)?;
        self.crash(op, Failpoint::FilesUnpacked)?;
        self.tx(|tx, ctx| store::set_journal_step(tx, id, "unpacked", ctx.now))?;
        self.crash(op, Failpoint::StepRecorded)?;
        let (outcome, retired) = self.tx(|tx, ctx| commit_update(tx, ctx, id, pending, source))?;
        self.crash(op, Failpoint::Committed)?;
        self.retire(&retired)?;
        Ok(outcome)
    }

    /// The permission diff of the staged update, if any.
    pub fn pending_update(&self, id: &PluginId) -> Result<Option<(Version, PermissionDiff)>> {
        let plugin = self.plugin(id)?;
        let Some(staged) = plugin.staged() else {
            return Ok(None);
        };
        let diff = PermissionDiff::compute(&self.store.grants(id)?, &staged.grants());
        Ok(Some((staged.version.clone(), diff)))
    }

    /// Activates the staged update with fresh consent for its escalations
    /// (every escalated key must be in `consent`).
    pub fn approve_update(&mut self, id: &PluginId, consent: &Consent) -> Result<Version> {
        let plugin = self.plugin(id)?;
        let staged = plugin
            .staged()
            .ok_or_else(|| StoreError::NoStagedUpdate(id.clone()))?
            .clone();
        check_consent(id, &staged.grants(), consent)?;
        let dir = self.layout.resolve(&staged.install_dir);
        if let Err(problem) = verify_installed(&dir, &staged.integrity) {
            self.decline_update(id)?;
            return Err(StoreError::Integrity {
                id: id.clone(),
                version: staged.version.to_string(),
                problem: problem.to_string(),
            });
        }
        let retired = self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            let plugin =
                store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
            store::transition(tx, id, plugin.state, PluginState::Updating, None, ctx.now)?;
            let candidate = staged.grants();
            let diff = PermissionDiff::compute(&store::load_grants(tx, id)?, &candidate);
            let retired = activate(
                tx,
                ctx,
                id,
                plugin.state,
                &staged.version,
                &diff,
                &candidate,
                Some(consent),
            )?;
            ctx.audit(
                tx,
                id,
                event(
                    Some(&staged.version.to_string()),
                    "update",
                    None,
                    "approved",
                    None,
                ),
            )?;
            Ok(retired)
        })?;
        self.crash(Operation::ApproveUpdate, Failpoint::Committed)?;
        self.retire(&retired)?;
        Ok(staged.version)
    }

    /// Drops the staged update; the running version stays.
    pub fn decline_update(&mut self, id: &PluginId) -> Result<()> {
        let dir = self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            let plugin =
                store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
            let staged = plugin
                .staged()
                .ok_or_else(|| StoreError::NoStagedUpdate(id.clone()))?;
            store::delete_version(tx, id, &staged.version)?;
            ctx.audit(
                tx,
                id,
                event(
                    Some(&staged.version.to_string()),
                    "update",
                    None,
                    "declined",
                    None,
                ),
            )?;
            Ok(staged.install_dir.clone())
        })?;
        self.crash(Operation::DeclineUpdate, Failpoint::Committed)?;
        self.retire(&[dir])
    }

    /// Moves version directories no record refers to any more into
    /// `.trash` and empties it. After a crash, recovery's orphan sweep does
    /// the same.
    fn retire(&mut self, dirs: &[String]) -> Result<()> {
        for dir in dirs {
            self.layout.move_to_trash(&self.layout.resolve(dir))?;
        }
        self.clean_up()
    }

    // --------------------------------------------------------------- rollback

    /// Makes the previous version active again (one transaction). Refused
    /// when that version is revoked (`disable`). Grants are recomputed from
    /// the older manifest and the stored consent; grants only the newer
    /// version declared are deleted, and if the older version needs a grant
    /// that is not stored, the plugin ends `Disabled`.
    pub fn rollback(&mut self, id: &PluginId, verdicts: &dyn VerdictSource) -> Result<Version> {
        let plugin = self.plugin(id)?;
        if let Some(staged) = plugin.staged() {
            return Err(StoreError::UpdatePending {
                id: id.clone(),
                version: staged.version.to_string(),
            });
        }
        let previous = plugin
            .previous()
            .ok_or_else(|| StoreError::NoPreviousVersion(id.clone()))?
            .clone();
        if let InstalledVerdict::Disable { reason, .. } = verdicts.verdict(id, &previous.version) {
            return Err(StoreError::Revoked {
                id: id.clone(),
                version: previous.version.to_string(),
                reason,
            });
        }
        let dir = self.layout.resolve(&previous.install_dir);
        verify_installed(&dir, &previous.integrity).map_err(|e| StoreError::Integrity {
            id: id.clone(),
            version: previous.version.to_string(),
            problem: e.to_string(),
        })?;
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            let plugin =
                store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
            let active = active_of(&plugin)?;
            store::transition(
                tx,
                id,
                plugin.state,
                PluginState::RollingBack,
                None,
                ctx.now,
            )?;
            // Swap through `staged` (free: a pending update was refused).
            store::set_role(tx, id, &active.version, VersionRole::Staged, ctx.now)?;
            store::set_role(tx, id, &previous.version, VersionRole::Active, ctx.now)?;
            store::set_role(tx, id, &active.version, VersionRole::Previous, ctx.now)?;
            let candidate = previous.grants();
            let diff = PermissionDiff::compute(&store::load_grants(tx, id)?, &candidate);
            apply_grant_changes(tx, ctx, id, &previous.version, &diff, &candidate, None)?;
            let escalated = diff.is_escalation();
            let to = if plugin.state == PluginState::Enabled && !escalated {
                PluginState::Enabled
            } else {
                PluginState::Disabled
            };
            let reason = escalated.then_some("the previous version needs consent".to_owned());
            store::transition(
                tx,
                id,
                PluginState::RollingBack,
                to,
                reason.as_deref(),
                ctx.now,
            )?;
            ctx.audit(
                tx,
                id,
                event(
                    Some(&previous.version.to_string()),
                    "rollback",
                    None,
                    "ok",
                    Some(&format!("from {}", active.version)),
                ),
            )
        })?;
        self.crash(Operation::Rollback, Failpoint::Committed)?;
        Ok(previous.version)
    }

    // -------------------------------------------------------------- uninstall

    /// Removes the plugin: its directories, its records and grants, its
    /// storage, settings and credentials, and (by the hook) its external
    /// secrets. Once started it only goes forward; recovery finishes it
    /// after a crash or an error. The audit log and `plugin_known` are
    /// kept.
    pub fn uninstall(&mut self, id: &PluginId) -> Result<()> {
        self.uninstall_keeping(id, Retention::default())
    }

    /// [`PluginManager::uninstall`], keeping the settings and/or the
    /// credentials `keep` names for a later install of the same plugin
    /// (storage is always deleted). The choice is journaled, so recovery
    /// honours it.
    pub fn uninstall_keeping(&mut self, id: &PluginId, keep: Retention) -> Result<()> {
        self.uninstall_as(id, keep, false)
    }

    /// [`PluginManager::uninstall_keeping`] because the user asked. A
    /// first-party plugin also gets an `uninstalled` tombstone, written with
    /// the start of the uninstall (which then only goes forward), so seeding
    /// and migration never install it again; [`install`](Self::install) is
    /// refused until the user clears it
    /// ([`clear_tombstone`](Self::clear_tombstone)).
    pub fn uninstall_by_user(&mut self, id: &PluginId, keep: Retention) -> Result<()> {
        self.uninstall_as(id, keep, true)
    }

    fn uninstall_as(&mut self, id: &PluginId, keep: Retention, by_user: bool) -> Result<()> {
        let tombstone = by_user && store::is_first_party(id);
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            let plugin =
                store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
            if tombstone {
                store::set_tombstone(tx, id, TombstoneDecision::Uninstalled, ctx.now)?;
                ctx.audit(
                    tx,
                    id,
                    event(
                        plugin.active().map(|v| v.version.to_string()).as_deref(),
                        "tombstone",
                        None,
                        "uninstalled",
                        None,
                    ),
                )?;
            }
            store::transition(
                tx,
                id,
                plugin.state,
                PluginState::Uninstalling,
                None,
                ctx.now,
            )?;
            store::insert_journal(
                tx,
                id,
                &Payload::Uninstall {
                    prior_state: plugin.state,
                    keep,
                },
                ctx.now,
            )
        })?;
        self.finish_uninstall(id, false)
    }

    pub(crate) fn finish_uninstall(&mut self, id: &PluginId, recovering: bool) -> Result<()> {
        let op = Operation::Uninstall;
        let crash = |manager: &Self, point| {
            if recovering {
                Ok(())
            } else {
                manager.crash(op, point)
            }
        };
        crash(self, Failpoint::JournalWritten)?;
        self.layout.move_to_trash(&self.layout.plugin_dir(id))?;
        crash(self, Failpoint::FilesTrashed)?;
        self.tx(|tx, ctx| store::set_journal_step(tx, id, "files_removed", ctx.now))?;
        crash(self, Failpoint::StepRecorded)?;
        let keep = match store::journal_for(self.store.connection(), id)? {
            Some(JournalEntry {
                payload: Payload::Uninstall { keep, .. },
                ..
            }) => keep,
            _ => Retention::default(),
        };
        if !keep.credentials {
            self.options
                .hooks
                .delete_secrets(id)
                .map_err(|message| StoreError::Hook {
                    id: id.clone(),
                    message,
                })?;
        }
        crash(self, Failpoint::SecretsDeleted)?;
        if recovering {
            self.crash(Operation::Recover, Failpoint::RecoveryFilesResolved)?;
        }
        self.tx(|tx, ctx| {
            if let Some(plugin) = store::load_plugin(tx, id)? {
                if !plugin.state.can_be_removed() {
                    return Err(StoreError::record(format!(
                        "plugin {id} is {} during uninstall",
                        plugin.state
                    )));
                }
                data::delete_plugin_data(tx, id, keep)?;
                store::delete_plugin(tx, id)?;
            }
            store::delete_journal(tx, id)?;
            ctx.audit(tx, id, event(None, "uninstall", None, "ok", None))
        })?;
        crash(self, Failpoint::Committed)?;
        self.clean_up()
    }

    // ------------------------------------------------- credentials, settings

    /// Stores the secret of credential slot `key` (declared in the active
    /// version's `contributes.credentials`), replacing an earlier one. The
    /// audit records the slot, never the secret.
    pub fn set_credential(&mut self, id: &PluginId, key: &str, secret: &str) -> Result<()> {
        if secret.is_empty() || secret.len() > data::MAX_SECRET_BYTES {
            return Err(StoreError::InvalidData(format!(
                "a secret is 1 to {} bytes",
                data::MAX_SECRET_BYTES
            )));
        }
        if secret.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
            return Err(StoreError::InvalidData(
                "a secret cannot contain line breaks or NUL characters".into(),
            ));
        }
        self.tx(|tx, ctx| {
            let plugin =
                store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
            declared_slot(&plugin, key)?;
            data::set_credential(tx, id, key, secret, ctx.now)?;
            let target = GrantKey::new(deckpilot_plugin_manifest::PermissionKind::Credentials, key);
            ctx.audit(
                tx,
                id,
                event(None, "credential.set", Some(&target), "ok", None),
            )
        })
    }

    /// Deletes the secret of credential slot `key`; whether there was one.
    pub fn delete_credential(&mut self, id: &PluginId, key: &str) -> Result<bool> {
        self.tx(|tx, ctx| {
            if store::load_plugin(tx, id)?.is_none() {
                return Err(StoreError::NotInstalled(id.clone()));
            }
            let deleted = data::delete_credential(tx, id, key)?;
            let target = GrantKey::new(deckpilot_plugin_manifest::PermissionKind::Credentials, key);
            ctx.audit(
                tx,
                id,
                event(None, "credential.delete", Some(&target), "ok", None),
            )?;
            Ok(deleted)
        })
    }

    /// Replaces the plugin's saved settings values. The caller has
    /// validated them against the manifest (`contributes.settings`); every
    /// key must be a declared setting.
    pub fn set_settings(
        &mut self,
        id: &PluginId,
        values: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<()> {
        self.tx(|tx, ctx| {
            let plugin =
                store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
            let manifest = &plugin
                .active()
                .ok_or_else(|| StoreError::NotInstalled(id.clone()))?
                .manifest;
            if let Some(key) = values
                .keys()
                .find(|k| !manifest.contributes.settings().iter().any(|s| &s.key == *k))
            {
                return Err(StoreError::InvalidData(format!(
                    "{id} has no setting `{key}`"
                )));
            }
            data::set_settings(tx, id, values, ctx.now)?;
            ctx.audit(tx, id, event(None, "settings.set", None, "ok", None))
        })
    }

    // ------------------------------------------------------------- revocation

    /// Quarantines the plugin (registry `disable` verdict): it stops and
    /// cannot be enabled until the revocation is lifted.
    pub fn quarantine(&mut self, id: &PluginId, reason: &str) -> Result<()> {
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            let plugin =
                store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
            store::transition(
                tx,
                id,
                plugin.state,
                PluginState::Quarantined,
                Some(reason),
                ctx.now,
            )?;
            let version = plugin.active().map(|v| v.version.to_string());
            ctx.audit(
                tx,
                id,
                event(
                    version.as_deref(),
                    "revocation",
                    None,
                    "quarantined",
                    Some(reason),
                ),
            )
        })
    }

    /// Applies the registry's verdicts to every installed plugin (at every
    /// start and after each refresh): a `disable` revocation of the active
    /// version quarantines the plugin, a quarantined plugin whose active
    /// version is no longer revoked goes back to `Disabled` (never straight
    /// to `Enabled`), and a revoked staged update is dropped.
    pub fn apply_verdicts(&mut self, verdicts: &dyn VerdictSource) -> Result<Vec<VerdictChange>> {
        let mut changes = Vec::new();
        for plugin in self.store.plugins()? {
            if !plugin.state.is_stable() {
                continue;
            }
            if let Some(staged) = plugin.staged()
                && matches!(
                    verdicts.verdict(&plugin.id, &staged.version),
                    InstalledVerdict::Disable { .. }
                )
            {
                self.decline_update(&plugin.id)?;
                changes.push(VerdictChange::StagedDropped {
                    id: plugin.id.clone(),
                    version: staged.version.clone(),
                });
            }
            let Some(active) = plugin.active() else {
                continue;
            };
            match verdicts.verdict(&plugin.id, &active.version) {
                InstalledVerdict::Disable { reason, advisory } => {
                    if plugin.state != PluginState::Quarantined {
                        let reason = match advisory {
                            Some(advisory) => format!("revoked: {reason} ({advisory})"),
                            None => format!("revoked: {reason}"),
                        };
                        self.quarantine(&plugin.id, &reason)?;
                        changes.push(VerdictChange::Quarantined {
                            id: plugin.id.clone(),
                            reason,
                        });
                    }
                }
                InstalledVerdict::Run | InstalledVerdict::RunWithNotice { .. } => {
                    if plugin.state == PluginState::Quarantined {
                        let id = plugin.id.clone();
                        let version = active.version.to_string();
                        self.tx(|tx, ctx| {
                            ensure_idle(tx, &id)?;
                            store::transition(
                                tx,
                                &id,
                                PluginState::Quarantined,
                                PluginState::Disabled,
                                None,
                                ctx.now,
                            )?;
                            ctx.audit(
                                tx,
                                &id,
                                event(Some(&version), "revocation", None, "lifted", None),
                            )
                        })?;
                        changes.push(VerdictChange::Released { id });
                    }
                }
            }
        }
        Ok(changes)
    }
}

// ------------------------------------------------------------------ helpers

pub(crate) fn event<'a>(
    version: Option<&'a str>,
    op: &'a str,
    target: Option<&'a GrantKey>,
    decision: &'a str,
    reason: Option<&'a str>,
) -> Audit<'a> {
    Audit {
        version,
        op,
        target,
        decision,
        reason,
    }
}

fn unknown(id: &PluginId, key: &GrantKey) -> StoreError {
    StoreError::UnknownGrant {
        id: id.clone(),
        key: key.clone(),
    }
}

/// `key` must be a credential slot of the active version.
fn declared_slot(plugin: &PluginRecord, key: &str) -> Result<()> {
    let declared = plugin.active().is_some_and(|v| {
        v.manifest
            .contributes
            .credentials()
            .iter()
            .any(|c| c.key == key)
    });
    if declared {
        Ok(())
    } else {
        Err(StoreError::InvalidData(format!(
            "{} has no credential slot `{key}`",
            plugin.id
        )))
    }
}

fn check_consent(id: &PluginId, declared: &GrantSet, consent: &Consent) -> Result<()> {
    match consent.keys.iter().find(|key| !declared.contains(key)) {
        Some(key) => Err(unknown(id, key)),
        None => Ok(()),
    }
}

/// Plugin API 1.9's privileged kinds (`tools`, `resources`,
/// `projectContext`, `hostViews`, `openUrl.fromParams`) come only with a
/// plugin shipped inside the app (X12 in
/// `docs/marketplace/module-migration.md` §7). The manifest rules already
/// refuse them for every other package; this keeps a store caller from
/// recording them from another source all the same.
fn check_privileged_source(
    id: &PluginId,
    declared: &GrantSet,
    source: &InstallSource,
) -> Result<()> {
    if *source != InstallSource::Bundled && declared.keys().any(GrantKey::is_first_party_only) {
        return Err(StoreError::SourceNotAllowed {
            id: id.clone(),
            origin: source.as_str(),
        });
    }
    Ok(())
}

/// Seeding a bundled plugin grants low-tier permissions only (recorded as
/// `derived`); a medium- or high-tier one waits for the user's consent.
fn check_bundled_consent(id: &PluginId, declared: &GrantSet, consent: &Consent) -> Result<()> {
    check_bundled_tiers(id, declared, consent, crate::grants::Tier::Low)
}

/// Every key of `consent` is at most of tier `highest`.
fn check_bundled_tiers(
    id: &PluginId,
    declared: &GrantSet,
    consent: &Consent,
    highest: crate::grants::Tier,
) -> Result<()> {
    for key in &consent.keys {
        let tier = declared
            .get(key)
            .map(crate::grants::GrantScope::tier)
            .ok_or_else(|| unknown(id, key))?;
        if tier > highest {
            return Err(StoreError::BundledGrant {
                id: id.clone(),
                key: key.clone(),
                tier,
            });
        }
    }
    Ok(())
}

/// Refuses a second operation on a plugin with an unfinished one.
pub(crate) fn ensure_idle(conn: &Connection, id: &PluginId) -> Result<()> {
    match store::journal_for(conn, id)? {
        Some(entry) => Err(StoreError::OperationInProgress {
            id: id.clone(),
            op: entry.op,
        }),
        None => Ok(()),
    }
}

fn active_of(plugin: &PluginRecord) -> Result<&VersionRecord> {
    plugin
        .active()
        .ok_or_else(|| StoreError::record(format!("plugin {} has no active version", plugin.id)))
}

fn pending_version(package: &VerifiedPackage) -> Result<PendingVersion> {
    let manifest = package.manifest();
    Ok(PendingVersion {
        version: manifest.version.clone(),
        install_dir: Layout::relative_version_dir(&manifest.id, &manifest.version),
        manifest_json: serde_json::to_string(manifest).map_err(StoreError::record)?,
        integrity: package.integrity().clone(),
    })
}

/// The last transaction of an install (also run by recovery).
pub(crate) fn commit_install(
    tx: &Connection,
    ctx: Ctx,
    id: &PluginId,
    pending: &PendingVersion,
    consent: &Consent,
) -> Result<()> {
    let plugin = store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
    store::transition(tx, id, plugin.state, PluginState::Disabled, None, ctx.now)?;
    store::insert_version(tx, id, pending, VersionRole::Active, ctx.now)?;
    let declared = GrantSet::from_manifest(&pending.manifest()?);
    let version = pending.version.to_string();
    // Seeding decides for a bundled plugin, not an install dialog.
    let grant_source = if plugin.source == InstallSource::Bundled {
        GrantSource::Derived
    } else {
        GrantSource::Install
    };
    for key in &consent.keys {
        let scope = declared.get(key).cloned().ok_or_else(|| unknown(id, key))?;
        store::upsert_grant(
            tx,
            id,
            &StoredGrant {
                key: key.clone(),
                scope,
                status: GrantStatus::Granted,
                source: grant_source,
                decided_by: consent.decided_by.clone(),
                decided_at: ctx.now,
                granted_for_version: version.clone(),
            },
        )?;
        ctx.audit(
            tx,
            id,
            event(Some(&version), "grant", Some(key), "granted", None),
        )?;
    }
    ctx.audit(tx, id, event(Some(&version), "install", None, "ok", None))?;
    store::delete_journal(tx, id)
}

/// The last transaction of an update (also run by recovery): records the
/// new version as staged, then activates it unless it escalates.
pub(crate) fn commit_update(
    tx: &Connection,
    ctx: Ctx,
    id: &PluginId,
    pending: &PendingVersion,
    source: &InstallSource,
) -> Result<(UpdateOutcome, Vec<String>)> {
    let Some(entry) = store::journal_for(tx, id)? else {
        return Err(StoreError::record(format!("update of {id} has no journal")));
    };
    let Payload::Update { prior_state, .. } = entry.payload else {
        return Err(StoreError::record(format!(
            "journal of {id} is not an update"
        )));
    };
    store::insert_version(tx, id, pending, VersionRole::Staged, ctx.now)?;
    store::set_source(tx, id, source)?;
    let candidate = GrantSet::from_manifest(&pending.manifest()?);
    let diff = PermissionDiff::compute(&store::load_grants(tx, id)?, &candidate);
    store::delete_journal(tx, id)?;
    let version = pending.version.to_string();
    if diff.is_escalation() {
        store::transition(tx, id, PluginState::Updating, prior_state, None, ctx.now)?;
        let keys = diff
            .escalations()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        ctx.audit(
            tx,
            id,
            event(Some(&version), "update", None, "staged", Some(&keys)),
        )?;
        return Ok((
            UpdateOutcome::AwaitingConsent {
                version: pending.version.clone(),
                diff,
            },
            Vec::new(),
        ));
    }
    let retired = activate(
        tx,
        ctx,
        id,
        prior_state,
        &pending.version,
        &diff,
        &candidate,
        None,
    )?;
    ctx.audit(
        tx,
        id,
        event(Some(&version), "update", None, "applied", None),
    )?;
    Ok((
        UpdateOutcome::Applied {
            version: pending.version.clone(),
            diff,
        },
        retired,
    ))
}

/// Makes the staged `version` active (the plugin is in `Updating`, coming
/// from `prior`): grants updated, old active becomes previous, the older
/// previous is retired (its directory returned). Escalations must be in
/// `consent`.
#[allow(clippy::too_many_arguments)]
fn activate(
    tx: &Connection,
    ctx: Ctx,
    id: &PluginId,
    prior: PluginState,
    version: &Version,
    diff: &PermissionDiff,
    candidate: &GrantSet,
    consent: Option<&Consent>,
) -> Result<Vec<String>> {
    // An optional grant the update adds or broadens waits for its own
    // consent, as on install: without it the update applies and only the
    // actions naming it stay unavailable.
    let missing: Vec<GrantKey> = diff
        .escalations()
        .into_iter()
        .filter(|key| !candidate.is_optional(key))
        .filter(|key| consent.is_none_or(|c| !c.keys.contains(key)))
        .collect();
    if !missing.is_empty() {
        return Err(StoreError::ConsentRequired {
            id: id.clone(),
            missing,
        });
    }
    apply_grant_changes(tx, ctx, id, version, diff, candidate, consent)?;
    let plugin = store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
    let mut retired = Vec::new();
    if let Some(previous) = plugin.previous() {
        store::delete_version(tx, id, &previous.version)?;
        retired.push(previous.install_dir.clone());
    }
    if let Some(active) = plugin.active() {
        store::set_role(tx, id, &active.version, VersionRole::Previous, ctx.now)?;
    }
    store::set_role(tx, id, version, VersionRole::Active, ctx.now)?;
    let to = if prior == PluginState::Enabled {
        PluginState::Enabled
    } else {
        PluginState::Disabled
    };
    store::transition(tx, id, PluginState::Updating, to, None, ctx.now)?;
    Ok(retired)
}

/// Writes the grant side of a diff: covered keys copy their covering
/// decision, narrowed keys take the new scope, consented escalations are
/// granted, keys no longer declared are deleted.
fn apply_grant_changes(
    tx: &Connection,
    ctx: Ctx,
    id: &PluginId,
    version: &Version,
    diff: &PermissionDiff,
    candidate: &GrantSet,
    consent: Option<&Consent>,
) -> Result<()> {
    let stored = store::load_grants(tx, id)?;
    let version_text = version.to_string();
    let scope_of = |key: &GrantKey| {
        candidate
            .get(key)
            .cloned()
            .ok_or_else(|| StoreError::record(format!("{key} is not declared")))
    };
    for (key, covering) in &diff.covered {
        let Some(row) = stored.iter().find(|row| row.key == *covering) else {
            continue;
        };
        store::upsert_grant(
            tx,
            id,
            &StoredGrant {
                key: key.clone(),
                scope: scope_of(key)?,
                status: row.status,
                source: GrantSource::Derived,
                decided_by: format!("derived from {covering}"),
                decided_at: ctx.now,
                granted_for_version: version_text.clone(),
            },
        )?;
    }
    for key in &diff.narrowed {
        store::set_grant_scope(tx, id, key, &scope_of(key)?)?;
    }
    if let Some(consent) = consent {
        for key in diff.escalations() {
            if !consent.keys.contains(&key) {
                continue;
            }
            store::upsert_grant(
                tx,
                id,
                &StoredGrant {
                    scope: scope_of(&key)?,
                    key: key.clone(),
                    status: GrantStatus::Granted,
                    source: GrantSource::Update,
                    decided_by: consent.decided_by.clone(),
                    decided_at: ctx.now,
                    granted_for_version: version_text.clone(),
                },
            )?;
            ctx.audit(
                tx,
                id,
                event(Some(&version_text), "grant", Some(&key), "granted", None),
            )?;
        }
    }
    for key in &diff.removed {
        store::delete_grant(tx, id, key)?;
        ctx.audit(
            tx,
            id,
            event(Some(&version_text), "grant", Some(key), "removed", None),
        )?;
    }
    Ok(())
}

#[cfg(all(test, feature = "sqlcipher"))]
mod tests {
    //! First-party plugins, tombstones and the module migration journal.
    //! These tests set up the plugin rows directly, through the store
    //! helpers the lifecycle uses. (A first-party package is only ever a
    //! bundled one, `deckpilot_plugin_package::verify_bundled_files`; the
    //! lifecycle tests in `tests/lifecycle.rs` install one.)

    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;
    use crate::state::MigrationPhase;
    use crate::store::{MAX_MIGRATION_ATTEMPTS, Tombstone};

    const NOTIFICATIONS: &str = "deckpilot.notifications";

    fn id(text: &str) -> PluginId {
        PluginId::parse(text).unwrap()
    }

    fn registry() -> InstallSource {
        InstallSource::Registry {
            registry: "https://registry.example".into(),
        }
    }

    struct Env {
        dir: tempfile::TempDir,
        now: Arc<AtomicI64>,
    }

    impl Env {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                now: Arc::new(AtomicI64::new(1_000)),
            }
        }

        /// A manager whose clock advances one second per reading.
        fn open(&self) -> PluginManager {
            let now = self.now.clone();
            PluginManager::open(
                self.dir.path(),
                &StoreKey::from_bytes([0x33; 32]),
                ManagerOptions {
                    clock: Arc::new(move || now.fetch_add(1, Ordering::SeqCst)),
                    ..ManagerOptions::default()
                },
            )
            .unwrap()
        }
    }

    fn insert(
        manager: &mut PluginManager,
        plugin: &str,
        state: PluginState,
        source: &InstallSource,
    ) -> Result<()> {
        let plugin = id(plugin);
        manager.tx(|tx, ctx| store::insert_plugin(tx, &plugin, state, source, ctx.now))
    }

    fn state(manager: &PluginManager, plugin: &str) -> PluginState {
        manager.store().plugin(&id(plugin)).unwrap().unwrap().state
    }

    // ---------------------------------------------------------- host-held

    /// Host-held values (schema 6): stored, replaced, removed, kept across
    /// a reopen, and never a plugin's (no plugin row is needed).
    #[test]
    fn host_settings_are_the_agents_own() {
        let env = Env::new();
        let mut manager = env.open();
        let key = "tools.git";
        assert_eq!(manager.store().host_setting(key).unwrap(), None);
        let value = serde_json::json!({"gitExecutable": "/opt/git/bin/git"});
        assert!(manager.set_host_setting(key, Some(&value)).unwrap());
        assert!(!manager.set_host_setting(key, Some(&value)).unwrap());
        drop(manager);
        let mut manager = env.open();
        assert_eq!(manager.store().host_setting(key).unwrap(), Some(value));
        let other = serde_json::json!({"networkTimeoutSeconds": 60});
        assert!(manager.set_host_setting(key, Some(&other)).unwrap());
        assert_eq!(manager.store().host_setting(key).unwrap(), Some(other));
        assert!(manager.set_host_setting(key, None).unwrap());
        assert_eq!(manager.store().host_setting(key).unwrap(), None);
        assert!(!manager.set_host_setting(key, None).unwrap());
        for bad in ["", "a\nb", &"x".repeat(65)] {
            assert!(
                manager
                    .set_host_setting(bad, Some(&serde_json::json!(1)))
                    .is_err()
            );
        }
        assert_eq!(manager.store().schema_version().unwrap(), 6);
    }

    // ------------------------------------------------------- install source

    #[test]
    fn first_party_ids_come_only_from_the_bundle_or_a_registry() {
        let env = Env::new();
        let mut manager = env.open();
        for source in [InstallSource::Sideload, InstallSource::Dev] {
            let result = insert(
                &mut manager,
                "deckpilot.github",
                PluginState::Disabled,
                &source,
            );
            assert!(
                matches!(
                    &result,
                    Err(StoreError::SourceNotAllowed { id, origin })
                        if id.as_str() == "deckpilot.github" && *origin == source.as_str()
                ),
                "{result:?}"
            );
        }
        let result = insert(
            &mut manager,
            "acme.github",
            PluginState::Disabled,
            &InstallSource::Bundled,
        );
        assert!(
            matches!(
                result,
                Err(StoreError::SourceNotAllowed {
                    origin: "bundled",
                    ..
                })
            ),
            "{result:?}"
        );
        assert_eq!(manager.store().plugins().unwrap(), Vec::new());

        insert(
            &mut manager,
            NOTIFICATIONS,
            PluginState::Disabled,
            &InstallSource::Bundled,
        )
        .unwrap();
        insert(
            &mut manager,
            "deckpilot.github",
            PluginState::Disabled,
            &registry(),
        )
        .unwrap();
        insert(
            &mut manager,
            "acme.tool",
            PluginState::Disabled,
            &InstallSource::Dev,
        )
        .unwrap();
        let record = manager.store().plugin(&id(NOTIFICATIONS)).unwrap().unwrap();
        assert_eq!(record.source, InstallSource::Bundled);

        // An update cannot move a first-party plugin to another source.
        let notifications = id(NOTIFICATIONS);
        let result =
            manager.tx(|tx, _| store::set_source(tx, &notifications, &InstallSource::Sideload));
        assert!(matches!(result, Err(StoreError::SourceNotAllowed { .. })));
        manager
            .tx(|tx, _| store::set_source(tx, &notifications, &registry()))
            .unwrap();
        let tool = id("acme.tool");
        let result = manager.tx(|tx, _| store::set_source(tx, &tool, &InstallSource::Bundled));
        assert!(matches!(result, Err(StoreError::SourceNotAllowed { .. })));
    }

    #[test]
    fn install_source_rules() {
        let first_party = id("deckpilot.github");
        let third_party = id("acme.github");
        assert!(store::is_first_party(&first_party));
        assert!(!store::is_first_party(&third_party));
        assert!(!store::is_first_party(&id("deckpilots.github")));
        for (source, first, third) in [
            (registry(), true, true),
            (InstallSource::Bundled, true, false),
            (InstallSource::Sideload, false, true),
            (InstallSource::Dev, false, true),
        ] {
            assert_eq!(source.allows(&first_party), first, "{source:?}");
            assert_eq!(source.allows(&third_party), third, "{source:?}");
        }
    }

    // ----------------------------------------------------------- tombstones

    #[test]
    fn a_user_disable_of_a_first_party_plugin_sticks() {
        let env = Env::new();
        let notifications = id(NOTIFICATIONS);
        {
            let mut manager = env.open();
            insert(
                &mut manager,
                NOTIFICATIONS,
                PluginState::Enabled,
                &InstallSource::Bundled,
            )
            .unwrap();
            manager.disable_by_user(&notifications).unwrap();
            assert_eq!(state(&manager, NOTIFICATIONS), PluginState::Disabled);
        }
        // Persisted: a later start still sees it and cannot enable it.
        let mut manager = env.open();
        let tombstone = manager.store().tombstone(&notifications).unwrap().unwrap();
        assert_eq!(tombstone.decision, TombstoneDecision::Disabled);
        let result = manager.enable(&notifications);
        assert!(
            matches!(
                result,
                Err(StoreError::Tombstoned {
                    decision: TombstoneDecision::Disabled,
                    ..
                })
            ),
            "{result:?}"
        );
        assert_eq!(state(&manager, NOTIFICATIONS), PluginState::Disabled);
        let ops: Vec<_> = manager
            .store()
            .audit(&notifications)
            .unwrap()
            .into_iter()
            .map(|a| (a.op, a.decision))
            .collect();
        assert!(ops.contains(&("tombstone".into(), "disabled".into())));

        // Only the user's explicit undo clears it; enabling then gets past
        // the tombstone (and fails here only because the row has no files).
        assert!(manager.clear_tombstone(&notifications).unwrap());
        assert!(!manager.clear_tombstone(&notifications).unwrap());
        assert!(manager.store().tombstone(&notifications).unwrap().is_none());
        assert!(!matches!(
            manager.enable(&notifications),
            Err(StoreError::Tombstoned { .. })
        ));
    }

    #[test]
    fn a_user_can_turn_down_a_first_party_plugin_seeded_disabled() {
        let env = Env::new();
        let mut manager = env.open();
        insert(
            &mut manager,
            NOTIFICATIONS,
            PluginState::Disabled,
            &InstallSource::Bundled,
        )
        .unwrap();
        manager.disable_by_user(&id(NOTIFICATIONS)).unwrap();
        assert_eq!(
            manager
                .store()
                .tombstone(&id(NOTIFICATIONS))
                .unwrap()
                .map(|t| t.decision),
            Some(TombstoneDecision::Disabled)
        );
        // A third-party plugin has no tombstone and keeps the old rule.
        insert(
            &mut manager,
            "acme.tool",
            PluginState::Disabled,
            &registry(),
        )
        .unwrap();
        assert!(matches!(
            manager.disable_by_user(&id("acme.tool")),
            Err(StoreError::IllegalTransition { .. })
        ));
    }

    #[test]
    fn the_agents_own_disable_writes_no_tombstone() {
        let env = Env::new();
        let mut manager = env.open();
        insert(
            &mut manager,
            NOTIFICATIONS,
            PluginState::Enabled,
            &InstallSource::Bundled,
        )
        .unwrap();
        manager.disable(&id(NOTIFICATIONS)).unwrap();
        assert_eq!(manager.store().tombstones().unwrap(), Vec::new());
    }

    #[test]
    fn a_user_uninstall_of_a_first_party_plugin_is_never_undone_automatically() {
        let env = Env::new();
        let notifications = id(NOTIFICATIONS);
        {
            let mut manager = env.open();
            insert(
                &mut manager,
                NOTIFICATIONS,
                PluginState::Enabled,
                &InstallSource::Bundled,
            )
            .unwrap();
            manager.disable_by_user(&notifications).unwrap();
            insert(&mut manager, "acme.tool", PluginState::Enabled, &registry()).unwrap();
            manager
                .uninstall_by_user(&notifications, Retention::default())
                .unwrap();
            manager
                .uninstall_by_user(&id("acme.tool"), Retention::default())
                .unwrap();
            assert_eq!(manager.store().plugins().unwrap(), Vec::new());
        }
        let manager = env.open();
        // The uninstall replaced the disable; the third-party plugin left
        // nothing behind.
        let tombstones = manager.store().tombstones().unwrap();
        assert_eq!(tombstones.len(), 1);
        let Tombstone {
            plugin_id,
            decision,
            ..
        } = &tombstones[0];
        assert_eq!(plugin_id, &notifications);
        assert_eq!(*decision, TombstoneDecision::Uninstalled);
        // What `install` checks before it writes anything.
        let result = store::ensure_not_tombstoned(manager.store().connection(), &notifications);
        assert!(
            matches!(
                result,
                Err(StoreError::Tombstoned {
                    decision: TombstoneDecision::Uninstalled,
                    ..
                })
            ),
            "{result:?}"
        );
        assert!(
            store::ensure_not_tombstoned(manager.store().connection(), &id("acme.tool")).is_ok()
        );
    }

    #[test]
    fn only_first_party_plugins_have_tombstones() {
        let env = Env::new();
        let mut manager = env.open();
        let tool = id("acme.tool");
        let result = manager
            .tx(|tx, ctx| store::set_tombstone(tx, &tool, TombstoneDecision::Disabled, ctx.now));
        assert!(matches!(result, Err(StoreError::InvalidData(_))));
    }

    // ----------------------------------------------------- module migrations

    fn step(phase: MigrationPhase) -> MigrationStep {
        MigrationStep {
            module_id: "github".into(),
            plugin_id: id("deckpilot.github"),
            phase,
            reason: None,
            source_digest: None,
            app_version: "0.9.0".into(),
        }
    }

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn recording_a_migration_step_is_idempotent() {
        let env = Env::new();
        let mut manager = env.open();
        let first = manager
            .record_migration(&step(MigrationPhase::Pending))
            .unwrap();
        assert_eq!(first.attempts, 0);
        let again = manager
            .record_migration(&step(MigrationPhase::Pending))
            .unwrap();
        assert_eq!(again, first, "a rerun changed the record");

        let installed = manager
            .record_migration(&step(MigrationPhase::Installed))
            .unwrap();
        assert!(installed.updated_at > first.updated_at);
        let copied = manager
            .record_migration(&MigrationStep {
                source_digest: Some(DIGEST.into()),
                ..step(MigrationPhase::SettingsCopied)
            })
            .unwrap();
        assert_eq!(copied.source_digest.as_deref(), Some(DIGEST));
        // Later steps keep the digest without restating it.
        let cut = manager
            .record_migration(&step(MigrationPhase::CutOver))
            .unwrap();
        assert_eq!(cut.source_digest.as_deref(), Some(DIGEST));
        assert!(cut.phase.is_cut_over());
        assert_eq!(
            manager
                .record_migration(&step(MigrationPhase::CutOver))
                .unwrap(),
            cut
        );
        assert_eq!(
            manager.store().module_migration("github").unwrap(),
            Some(cut)
        );
        assert_eq!(manager.store().module_migrations().unwrap().len(), 1);
        assert!(
            manager
                .store()
                .module_migration("gitlab")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn failures_count_attempts_until_the_user_retries() {
        let env = Env::new();
        {
            let mut manager = env.open();
            manager
                .record_migration(&step(MigrationPhase::Installed))
                .unwrap();
            for attempt in 1..=MAX_MIGRATION_ATTEMPTS {
                let failed = manager
                    .record_migration(&MigrationStep {
                        reason: Some("the settings copy failed".into()),
                        ..step(MigrationPhase::Failed)
                    })
                    .unwrap();
                assert_eq!(failed.attempts, attempt);
                assert_eq!(
                    failed.retries_automatically(),
                    attempt < MAX_MIGRATION_ATTEMPTS
                );
            }
        }
        // Persisted across starts.
        let mut manager = env.open();
        let stored = manager.store().module_migration("github").unwrap().unwrap();
        assert_eq!(stored.phase, MigrationPhase::Failed);
        assert_eq!(stored.attempts, MAX_MIGRATION_ATTEMPTS);
        assert_eq!(stored.reason.as_deref(), Some("the settings copy failed"));
        assert!(!stored.retries_automatically());

        let retried = manager.retry_migration("github").unwrap().unwrap();
        assert_eq!(retried.phase, MigrationPhase::Pending);
        assert_eq!(retried.attempts, 0);
        assert_eq!(retried.reason, None);
        assert!(manager.retry_migration("jira").unwrap().is_none());

        // A failure after progress keeps counting; a cut-over resets it.
        manager
            .record_migration(&step(MigrationPhase::Failed))
            .unwrap();
        let cut = manager
            .record_migration(&step(MigrationPhase::CutOver))
            .unwrap();
        assert_eq!(cut.attempts, 0);
    }

    #[test]
    fn migration_steps_are_checked() {
        let env = Env::new();
        let mut manager = env.open();
        manager
            .record_migration(&step(MigrationPhase::Pending))
            .unwrap();
        let bad = [
            MigrationStep {
                plugin_id: id("deckpilot.other"),
                ..step(MigrationPhase::Installed)
            },
            MigrationStep {
                module_id: "gitlab".into(),
                plugin_id: id("acme.gitlab"),
                ..step(MigrationPhase::Pending)
            },
            MigrationStep {
                module_id: "Git Hub".into(),
                ..step(MigrationPhase::Pending)
            },
            MigrationStep {
                reason: Some("two\nlines".into()),
                ..step(MigrationPhase::Failed)
            },
            MigrationStep {
                reason: Some("x".repeat(store::MAX_MIGRATION_REASON_CHARS + 1)),
                ..step(MigrationPhase::Failed)
            },
            MigrationStep {
                source_digest: Some("not-a-digest".into()),
                ..step(MigrationPhase::SettingsCopied)
            },
            MigrationStep {
                app_version: String::new(),
                ..step(MigrationPhase::Installed)
            },
        ];
        for step in bad {
            let result = manager.record_migration(&step);
            assert!(
                matches!(result, Err(StoreError::InvalidData(_))),
                "{step:?}"
            );
        }
        let stored = manager.store().module_migration("github").unwrap().unwrap();
        assert_eq!(stored.phase, MigrationPhase::Pending);
        assert_eq!(stored.attempts, 0);
        assert_eq!(manager.store().module_migrations().unwrap().len(), 1);
    }
}
