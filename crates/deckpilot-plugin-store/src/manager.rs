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

use crate::db::{DEFAULT_BUSY_TIMEOUT, StoreKey};
use crate::error::{Result, StoreError};
use crate::failpoint::{Failpoint, Failpoints, NoFailpoints, Operation};
use crate::grants::{
    Consent, GrantKey, GrantSet, GrantSource, GrantStatus, PermissionDiff, StoredGrant,
};
use crate::layout::Layout;
use crate::state::PluginState;
use crate::store::{
    self, Audit, AuditRetention, InstallSource, Payload, PendingVersion, PluginRecord, PluginStore,
    VersionRecord, VersionRole,
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
        let declared = GrantSet::from_manifest(package.manifest());
        check_consent(&id, &declared, consent)?;
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

    /// `Disabled → Enabled`. Every declared grant must be granted (with at
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
            store::transition(tx, id, plugin.state, PluginState::Enabled, None, ctx.now)?;
            ctx.audit(tx, id, event(Some(&version), "enable", None, "ok", None))
        })
    }

    /// `Enabled → Disabled`.
    pub fn disable(&mut self, id: &PluginId) -> Result<()> {
        let plugin = self.plugin(id)?;
        if plugin.state != PluginState::Enabled {
            return Err(StoreError::IllegalTransition {
                id: id.clone(),
                from: plugin.state,
                to: PluginState::Disabled,
            });
        }
        let version = plugin.active().map(|v| v.version.to_string());
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            store::transition(tx, id, plugin.state, PluginState::Disabled, None, ctx.now)?;
            ctx.audit(
                tx,
                id,
                event(version.as_deref(), "disable", None, "ok", None),
            )
        })
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

    pub(crate) fn mark_broken(&mut self, id: &PluginId, problem: &str) -> Result<()> {
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

    /// Removes the plugin: its directories, its records and grants, and (by
    /// the hook) its secrets. Once started it only goes forward; recovery
    /// finishes it after a crash or an error. The audit log is kept.
    pub fn uninstall(&mut self, id: &PluginId) -> Result<()> {
        self.tx(|tx, ctx| {
            ensure_idle(tx, id)?;
            let plugin =
                store::load_plugin(tx, id)?.ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
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
        self.options
            .hooks
            .delete_secrets(id)
            .map_err(|message| StoreError::Hook {
                id: id.clone(),
                message,
            })?;
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
                store::delete_plugin(tx, id)?;
            }
            store::delete_journal(tx, id)?;
            ctx.audit(tx, id, event(None, "uninstall", None, "ok", None))
        })?;
        crash(self, Failpoint::Committed)?;
        self.clean_up()
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

fn check_consent(id: &PluginId, declared: &GrantSet, consent: &Consent) -> Result<()> {
    match consent.keys.iter().find(|key| !declared.contains(key)) {
        Some(key) => Err(unknown(id, key)),
        None => Ok(()),
    }
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
    for key in &consent.keys {
        let scope = declared.get(key).cloned().ok_or_else(|| unknown(id, key))?;
        store::upsert_grant(
            tx,
            id,
            &StoredGrant {
                key: key.clone(),
                scope,
                status: GrantStatus::Granted,
                source: GrantSource::Install,
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
    let missing: Vec<GrantKey> = diff
        .escalations()
        .into_iter()
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
