//! Crash recovery and the database/file system consistency check.
//!
//! [`PluginManager::recover`] runs at every start, before any plugin is
//! loaded. Each step is idempotent, so a crash during recovery is repaired
//! by the next recovery:
//!
//! 1. Empty `.staging` (only crash leftovers: the manager holds the lock).
//! 2. Resolve each journal entry:
//!    * install / update: if the version directory exists and matches the
//!      integrity record in the entry, **complete** the operation (the same
//!      final transaction as the normal path); otherwise **undo** it (move
//!      the directory to `.trash`, restore the prior state or delete the
//!      `Installing` row, delete the entry);
//!    * uninstall: always **complete** (forward only).
//! 3. Move directories no record refers to into `.trash` (a crash after a
//!    commit but before its clean-up).
//! 4. Verify every recorded version: an active version that does not match
//!    marks the plugin `Broken`; a previous or staged one is dropped.
//! 5. Any transient state without a journal entry (cannot happen through
//!    this API) is resolved conservatively.
//! 6. Empty `.trash`.

use std::collections::BTreeSet;

use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_package::verify_installed;
use semver::Version;

use crate::error::{Result, StoreError};
use crate::failpoint::{Failpoint, Operation};
use crate::manager::{Ctx, PluginManager, commit_install, commit_update, event};
use crate::state::PluginState;
use crate::store::{self, Payload, PendingVersion, VersionRole};

/// What [`PluginManager::recover`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Interrupted operations that were finished.
    pub completed: Vec<(PluginId, Operation)>,
    /// Interrupted operations that were undone.
    pub rolled_back: Vec<(PluginId, Operation)>,
    /// Directories (relative to `<data dir>/plugins`) no record referred to.
    pub orphans_removed: Vec<String>,
    /// Previous or staged versions whose files were missing or modified.
    pub dropped_versions: Vec<(PluginId, Version)>,
    /// Plugins whose active version failed its integrity check.
    pub broken: Vec<PluginId>,
}

impl RecoveryReport {
    pub fn is_clean(&self) -> bool {
        *self == Self::default()
    }
}

/// A difference between the records and the plugin directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inconsistency {
    PendingOperation {
        id: String,
        op: Operation,
    },
    TransientState {
        id: PluginId,
        state: PluginState,
    },
    MissingActiveVersion(PluginId),
    MissingDirectory {
        id: PluginId,
        version: Version,
    },
    IntegrityMismatch {
        id: PluginId,
        version: Version,
        problem: String,
    },
    OrphanDirectory(String),
    Leftovers(usize),
    /// A grant for a key the active version does not declare.
    UndeclaredGrant {
        id: PluginId,
        key: String,
    },
    /// An enabled plugin with a declared grant nobody decided on.
    UndecidedGrant {
        id: PluginId,
        key: String,
    },
}

impl PluginManager {
    /// Completes or undoes interrupted operations and repairs the plugin
    /// directory. Idempotent; call it once after [`PluginManager::open`].
    pub fn recover(&mut self) -> Result<RecoveryReport> {
        let mut report = RecoveryReport::default();
        self.layout.empty_staging()?;

        for entry in self.store.journal()? {
            let id = store::parse_id(&entry.plugin_id)?;
            match &entry.payload {
                Payload::Install {
                    package, consent, ..
                } => {
                    if self.files_match(package) {
                        self.crash(Operation::Recover, Failpoint::RecoveryFilesResolved)?;
                        self.tx(|tx, ctx| commit_install(tx, ctx, &id, package, consent))?;
                        report.completed.push((id, entry.op));
                    } else {
                        self.undo(&id, &entry.payload)?;
                        report.rolled_back.push((id, entry.op));
                    }
                }
                Payload::Update {
                    package, source, ..
                } => {
                    if self.files_match(package) {
                        self.crash(Operation::Recover, Failpoint::RecoveryFilesResolved)?;
                        let (_, retired) =
                            self.tx(|tx, ctx| commit_update(tx, ctx, &id, package, source))?;
                        for dir in retired {
                            self.layout.move_to_trash(&self.layout.resolve(&dir))?;
                        }
                        report.completed.push((id, entry.op));
                    } else {
                        self.undo(&id, &entry.payload)?;
                        report.rolled_back.push((id, entry.op));
                    }
                }
                Payload::Uninstall { .. } => {
                    self.finish_uninstall(&id, true)?;
                    report.completed.push((id, entry.op));
                }
            }
        }

        report.orphans_removed = self.remove_orphans()?;
        self.verify_versions(&mut report)?;
        self.resolve_transient_states(&mut report)?;
        self.layout.empty_trash()?;
        self.layout.empty_staging()?;
        Ok(report)
    }

    fn files_match(&self, package: &PendingVersion) -> bool {
        let dir = self.layout.resolve(&package.install_dir);
        verify_installed(&dir, &package.integrity).is_ok()
    }

    /// Undoes an install or update whose final transaction did not commit.
    pub(crate) fn undo(&mut self, id: &PluginId, payload: &Payload) -> Result<()> {
        match payload {
            Payload::Install { .. } => {
                // A fresh install owns the whole plugin directory.
                self.layout.move_to_trash(&self.layout.plugin_dir(id))?;
                self.crash(Operation::Recover, Failpoint::RecoveryFilesResolved)?;
                self.tx(|tx, ctx| {
                    if let Some(plugin) = store::load_plugin(tx, id)? {
                        if plugin.state != PluginState::Installing {
                            return Err(StoreError::record(format!(
                                "plugin {id} is {} during install",
                                plugin.state
                            )));
                        }
                        store::delete_plugin(tx, id)?;
                    }
                    store::delete_journal(tx, id)?;
                    ctx.audit(tx, id, event(None, "install", None, "rolled_back", None))
                })?;
            }
            Payload::Update {
                package,
                prior_state,
                ..
            } => {
                let referenced = self.store.plugin(id)?.is_some_and(|p| {
                    p.versions
                        .iter()
                        .any(|v| v.install_dir == package.install_dir)
                });
                if !referenced {
                    self.layout
                        .move_to_trash(&self.layout.resolve(&package.install_dir))?;
                }
                self.crash(Operation::Recover, Failpoint::RecoveryFilesResolved)?;
                let prior = *prior_state;
                let version = package.version.to_string();
                self.tx(|tx, ctx| {
                    if let Some(plugin) = store::load_plugin(tx, id)?
                        && plugin.state == PluginState::Updating
                    {
                        store::transition(tx, id, PluginState::Updating, prior, None, ctx.now)?;
                    }
                    store::delete_journal(tx, id)?;
                    ctx.audit(
                        tx,
                        id,
                        event(Some(&version), "update", None, "rolled_back", None),
                    )
                })?;
            }
            Payload::Uninstall { .. } => return self.finish_uninstall(id, true),
        }
        self.clean_up()
    }

    fn referenced_dirs(&self) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
        let mut plugins = BTreeSet::new();
        let mut versions = BTreeSet::new();
        for plugin in self.store.plugins()? {
            plugins.insert(plugin.id.as_str().to_owned());
            for version in &plugin.versions {
                versions.insert(version.install_dir.clone());
            }
        }
        for entry in self.store.journal()? {
            plugins.insert(entry.plugin_id.clone());
            if let Payload::Install { package, .. } | Payload::Update { package, .. } =
                &entry.payload
            {
                versions.insert(package.install_dir.clone());
            }
        }
        Ok((plugins, versions))
    }

    fn orphans(&self) -> Result<Vec<String>> {
        let (plugins, versions) = self.referenced_dirs()?;
        let mut orphans = Vec::new();
        for name in self.layout.plugin_dirs()? {
            if !plugins.contains(&name) {
                orphans.push(name);
                continue;
            }
            for version in self.layout.version_dirs(&name)? {
                let relative = format!("{name}/{version}");
                if !versions.contains(&relative) {
                    orphans.push(relative);
                }
            }
        }
        Ok(orphans)
    }

    fn remove_orphans(&mut self) -> Result<Vec<String>> {
        let orphans = self.orphans()?;
        for relative in &orphans {
            self.layout.move_to_trash(&self.layout.resolve(relative))?;
        }
        Ok(orphans)
    }

    fn verify_versions(&mut self, report: &mut RecoveryReport) -> Result<()> {
        for plugin in self.store.plugins()? {
            for version in &plugin.versions {
                let dir = self.layout.resolve(&version.install_dir);
                let Err(problem) = verify_installed(&dir, &version.integrity) else {
                    continue;
                };
                if version.role == VersionRole::Active {
                    if plugin.state.can_transition_to(PluginState::Broken) {
                        self.mark_broken(&plugin.id, &problem.to_string())?;
                        report.broken.push(plugin.id.clone());
                    }
                } else {
                    let id = plugin.id.clone();
                    let number = version.version.clone();
                    self.tx(|tx, ctx| {
                        store::delete_version(tx, &id, &number)?;
                        ctx.audit(
                            tx,
                            &id,
                            event(
                                Some(&number.to_string()),
                                "integrity",
                                None,
                                "dropped",
                                Some(&problem.to_string()),
                            ),
                        )
                    })?;
                    self.layout.move_to_trash(&dir)?;
                    report
                        .dropped_versions
                        .push((plugin.id.clone(), version.version.clone()));
                }
            }
        }
        Ok(())
    }

    fn resolve_transient_states(&mut self, report: &mut RecoveryReport) -> Result<()> {
        for plugin in self.store.plugins()? {
            let id = plugin.id.clone();
            match plugin.state {
                PluginState::Installing => {
                    self.layout.move_to_trash(&self.layout.plugin_dir(&id))?;
                    self.tx(|tx, _| store::delete_plugin(tx, &id))?;
                    report.rolled_back.push((id, Operation::Install));
                }
                PluginState::Uninstalling => {
                    self.finish_uninstall(&id, true)?;
                    report.completed.push((id, Operation::Uninstall));
                }
                PluginState::Updating | PluginState::RollingBack => {
                    let to = if plugin.state == PluginState::Updating {
                        PluginState::Broken
                    } else {
                        PluginState::Disabled
                    };
                    self.tx(|tx, ctx: Ctx| {
                        store::transition(tx, &id, plugin.state, to, Some("recovered"), ctx.now)
                    })?;
                    report.broken.push(id);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Compares the records with the plugin directory without changing
    /// anything. Empty after a successful [`recover`](Self::recover).
    pub fn check_consistency(&self) -> Result<Vec<Inconsistency>> {
        let mut found = Vec::new();
        for entry in self.store.journal()? {
            found.push(Inconsistency::PendingOperation {
                id: entry.plugin_id,
                op: entry.op,
            });
        }
        for plugin in self.store.plugins()? {
            if !plugin.state.is_stable() {
                found.push(Inconsistency::TransientState {
                    id: plugin.id.clone(),
                    state: plugin.state,
                });
            }
            let Some(active) = plugin.active() else {
                if plugin.state.is_stable() {
                    found.push(Inconsistency::MissingActiveVersion(plugin.id.clone()));
                }
                continue;
            };
            for version in &plugin.versions {
                let dir = self.layout.resolve(&version.install_dir);
                if !dir.is_dir() {
                    found.push(Inconsistency::MissingDirectory {
                        id: plugin.id.clone(),
                        version: version.version.clone(),
                    });
                } else if let Err(problem) = verify_installed(&dir, &version.integrity)
                    && !(version.role == VersionRole::Active && plugin.state == PluginState::Broken)
                {
                    found.push(Inconsistency::IntegrityMismatch {
                        id: plugin.id.clone(),
                        version: version.version.clone(),
                        problem: problem.to_string(),
                    });
                }
            }
            let declared = active.grants();
            let stored = self.store.grants(&plugin.id)?;
            for row in &stored {
                if !declared.contains(&row.key) {
                    found.push(Inconsistency::UndeclaredGrant {
                        id: plugin.id.clone(),
                        key: row.key.to_string(),
                    });
                }
            }
            if plugin.state == PluginState::Enabled {
                for key in declared.keys() {
                    if !stored.iter().any(|row| row.key == *key) {
                        found.push(Inconsistency::UndecidedGrant {
                            id: plugin.id.clone(),
                            key: key.to_string(),
                        });
                    }
                }
            }
        }
        for orphan in self.orphans()? {
            found.push(Inconsistency::OrphanDirectory(orphan));
        }
        let leftovers = self.layout.leftovers()?;
        if leftovers > 0 {
            found.push(Inconsistency::Leftovers(leftovers));
        }
        Ok(found)
    }
}
