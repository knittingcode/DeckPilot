//! Carrying out a dependency plan (`deps.rs`) with the user's consent,
//! all or nothing (`docs/marketplace/dependencies.md` §4).
//!
//! The caller has resolved the plan, downloaded **and verified** every
//! package of it, shown it with the permissions of every plugin it adds
//! and got the user's confirmation. [`PluginManager::apply_plan`] then:
//!
//! 1. checks, before writing anything, that each consent names only
//!    permissions of its own plugin, and (when activating) that every
//!    plugin to enable will have all its permissions: a dependency never
//!    borrows the requesting plugin's consent, and a declined permission
//!    stops the plan before it starts;
//! 2. installs and updates in plan order (dependencies first), each one a
//!    crash-safe store operation of its own, new plugins `Disabled`;
//! 3. records which plugins were installed only as dependencies;
//! 4. when activating, enables in the same order, so a plugin is enabled
//!    only once everything it requires is.
//!
//! If a step fails, the steps already done are undone in reverse order: a
//! plugin the plan enabled is disabled, one it installed is uninstalled
//! (keeping any settings and credentials an earlier uninstall kept), one
//! it updated is rolled back. A crash in the middle leaves each finished
//! step complete and consistent (installed and disabled, or updated);
//! recovery finishes the interrupted one, and the plugins installed as
//! dependencies are then offered as orphans.

use std::collections::BTreeSet;

use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_package::VerifiedPackage;
use deckpilot_plugin_registry::InstalledVerdict;
use semver::Version;

use crate::data::Retention;
use crate::error::{Result, StoreError};
use crate::grants::{Consent, GrantKey, GrantSet, PermissionDiff, missing_grants};
use crate::manager::{PluginManager, UpdateOutcome, VerdictSource, event};
use crate::state::PluginState;
use crate::store::{self, InstallSource};

/// One step of a plan, ready to run.
#[derive(Debug, Clone)]
pub enum PlannedStep<'a> {
    /// A new plugin from a verified package.
    Install {
        package: &'a VerifiedPackage,
        source: InstallSource,
        /// The permissions the user granted **to this plugin**.
        consent: Consent,
        /// The plugin whose install needs it, when it is a dependency:
        /// recorded so it can be offered for removal once nothing needs
        /// it. `None` for the plugin the user asked for.
        dependency_of: Option<PluginId>,
    },
    /// An installed plugin moved to a newer verified version.
    Update {
        package: &'a VerifiedPackage,
        source: InstallSource,
        /// Covers the update's escalations, if it has any.
        consent: Consent,
    },
    /// An installed, disabled plugin to enable (granting `consent` first).
    Enable { id: PluginId, consent: Consent },
    /// The plugin the user asked for is installed already as a
    /// dependency: from now on it counts as installed by the user.
    Adopt { id: PluginId },
}

impl PlannedStep<'_> {
    fn id(&self) -> PluginId {
        match self {
            Self::Install { package, .. } | Self::Update { package, .. } => {
                package.manifest().id.clone()
            }
            Self::Enable { id, .. } | Self::Adopt { id } => id.clone(),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Install { package, .. } => format!(
                "installing {} {}",
                package.manifest().id,
                package.manifest().version
            ),
            Self::Update { package, .. } => format!(
                "updating {} to {}",
                package.manifest().id,
                package.manifest().version
            ),
            Self::Enable { id, .. } => format!("enabling {id}"),
            Self::Adopt { id } => format!("keeping {id}"),
        }
    }
}

/// What [`PluginManager::apply_plan`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppliedPlan {
    pub installed: Vec<PluginId>,
    pub updated: Vec<(PluginId, Version)>,
    pub enabled: Vec<PluginId>,
    /// Updates that stay staged (not activating, and an escalation).
    pub staged: Vec<(PluginId, Version)>,
}

/// What to undo after a failed step.
#[derive(Debug, Clone)]
enum Undo {
    Uninstall(PluginId),
    Rollback(PluginId),
    DeclineUpdate(PluginId),
    Disable(PluginId),
    MarkDependency(PluginId, String),
}

/// Every version was running a moment ago: undoing an update rolls back
/// whatever the registry says now.
struct Undoing;

impl VerdictSource for Undoing {
    fn verdict(&self, _id: &PluginId, _version: &Version) -> InstalledVerdict {
        InstalledVerdict::Run
    }
}

impl PluginManager {
    /// Marks `id` as installed only because `required_by` needed it.
    pub fn mark_dependency_install(&mut self, id: &PluginId, required_by: &str) -> Result<()> {
        self.tx(|tx, ctx| {
            if store::load_plugin(tx, id)?.is_none() {
                return Err(StoreError::NotInstalled(id.clone()));
            }
            store::set_dependency_install(tx, id, required_by, ctx.now)?;
            ctx.audit(
                tx,
                id,
                event(None, "dependency", None, "installed", Some(required_by)),
            )
        })
    }

    /// The user installed `id` themselves (it was a dependency): it is no
    /// longer offered as an orphan. Returns whether it was marked.
    pub fn mark_explicit(&mut self, id: &PluginId) -> Result<bool> {
        self.tx(|tx, ctx| {
            let removed = store::delete_dependency_install(tx, id)?;
            if removed {
                ctx.audit(tx, id, event(None, "dependency", None, "adopted", None))?;
            }
            Ok(removed)
        })
    }

    /// Runs a confirmed plan; see the module documentation. `activate`:
    /// enable every plugin of the plan (dependencies first) once all of
    /// them are installed.
    pub fn apply_plan(&mut self, steps: &[PlannedStep<'_>], activate: bool) -> Result<AppliedPlan> {
        self.check_plan(steps, activate)?;
        let mut done: Vec<Undo> = Vec::new();
        let mut applied = AppliedPlan::default();
        let mut to_enable: Vec<PluginId> = Vec::new();
        for step in steps {
            if let Err(error) =
                self.run_step(step, activate, &mut done, &mut applied, &mut to_enable)
            {
                return Err(self.undo_all(step.label(), error, done));
            }
        }
        if activate {
            for id in to_enable {
                let result = self.enable(&id);
                if let Err(error) = result {
                    return Err(self.undo_all(format!("enabling {id}"), error, done));
                }
                done.push(Undo::Disable(id.clone()));
                applied.enabled.push(id);
            }
        }
        Ok(applied)
    }

    /// Refuses, before anything is written, consent to permissions a
    /// plugin does not declare and (when activating) any plugin that would
    /// be left without a permission it needs.
    fn check_plan(&self, steps: &[PlannedStep<'_>], activate: bool) -> Result<()> {
        let mut seen = BTreeSet::new();
        for step in steps {
            let id = step.id();
            if !seen.insert(id.clone()) {
                return Err(StoreError::InvalidData(format!(
                    "{id} appears twice in the plan"
                )));
            }
            match step {
                PlannedStep::Install {
                    package, consent, ..
                } => {
                    let declared = GrantSet::from_manifest(package.manifest());
                    check_declared(&id, &declared, consent)?;
                    if activate {
                        let missing: Vec<GrantKey> = declared
                            .keys()
                            .filter(|k| !declared.is_optional(k) && !consent.keys.contains(*k))
                            .cloned()
                            .collect();
                        if !missing.is_empty() {
                            return Err(StoreError::ConsentRequired { id, missing });
                        }
                    }
                }
                PlannedStep::Update {
                    package, consent, ..
                } => {
                    let declared = GrantSet::from_manifest(package.manifest());
                    check_declared(&id, &declared, consent)?;
                    let diff = PermissionDiff::compute(&self.store.grants(&id)?, &declared);
                    let missing: Vec<GrantKey> = diff
                        .escalations()
                        .into_iter()
                        .filter(|k| !declared.is_optional(k) && !consent.keys.contains(k))
                        .collect();
                    if activate && !missing.is_empty() {
                        return Err(StoreError::ConsentRequired { id, missing });
                    }
                }
                PlannedStep::Enable { consent, .. } => {
                    let plugin = self
                        .store
                        .plugin(&id)?
                        .ok_or_else(|| StoreError::NotInstalled(id.clone()))?;
                    let declared = plugin
                        .active()
                        .map(crate::store::VersionRecord::grants)
                        .unwrap_or_default();
                    check_declared(&id, &declared, consent)?;
                    if activate {
                        let missing: Vec<GrantKey> =
                            missing_grants(&self.store.grants(&id)?, &declared)
                                .into_iter()
                                .filter(|k| !consent.keys.contains(k))
                                .collect();
                        if !missing.is_empty() {
                            return Err(StoreError::ConsentRequired { id, missing });
                        }
                    }
                }
                PlannedStep::Adopt { .. } => {}
            }
        }
        Ok(())
    }

    fn run_step(
        &mut self,
        step: &PlannedStep<'_>,
        activate: bool,
        done: &mut Vec<Undo>,
        applied: &mut AppliedPlan,
        to_enable: &mut Vec<PluginId>,
    ) -> Result<()> {
        let id = step.id();
        match step {
            PlannedStep::Install {
                package,
                source,
                consent,
                dependency_of,
            } => {
                self.install(package, source, consent)?;
                done.push(Undo::Uninstall(id.clone()));
                applied.installed.push(id.clone());
                if let Some(of) = dependency_of {
                    self.mark_dependency_install(&id, of.as_str())?;
                }
                to_enable.push(id);
            }
            PlannedStep::Update {
                package,
                source,
                consent,
            } => {
                let was_enabled = self.store.plugin(&id)?.map(|p| p.state);
                match self.update(package, source)? {
                    UpdateOutcome::Applied { version, .. } => {
                        done.push(Undo::Rollback(id.clone()));
                        applied.updated.push((id.clone(), version));
                    }
                    UpdateOutcome::AwaitingConsent { version, .. } => {
                        if activate {
                            done.push(Undo::DeclineUpdate(id.clone()));
                            self.approve_update(&id, consent)?;
                            done.pop();
                            done.push(Undo::Rollback(id.clone()));
                            applied.updated.push((id.clone(), version));
                        } else {
                            done.push(Undo::DeclineUpdate(id.clone()));
                            applied.staged.push((id.clone(), version));
                        }
                    }
                }
                if was_enabled != Some(PluginState::Enabled)
                    && self.store.plugin(&id)?.map(|p| p.state) != Some(PluginState::Enabled)
                {
                    to_enable.push(id);
                }
            }
            PlannedStep::Enable { consent, .. } => {
                if !consent.keys.is_empty() {
                    self.grant(&id, consent)?;
                }
                to_enable.push(id);
            }
            PlannedStep::Adopt { .. } => {
                let required_by = self.store.dependency_installs()?.remove(&id);
                if self.mark_explicit(&id)?
                    && let Some(by) = required_by
                {
                    done.push(Undo::MarkDependency(id, by));
                }
            }
        }
        Ok(())
    }

    /// Undoes `done` in reverse; returns the step's error with whatever
    /// could not be undone.
    fn undo_all(&mut self, step: String, error: StoreError, done: Vec<Undo>) -> StoreError {
        if error.is_crash() {
            // A simulated crash: nothing may clean up after it.
            return error;
        }
        let mut undo_failed = Vec::new();
        for undo in done.into_iter().rev() {
            let (what, result) = match &undo {
                Undo::Uninstall(id) => (
                    format!("uninstalling {id}"),
                    self.uninstall_keeping(
                        id,
                        Retention {
                            settings: true,
                            credentials: true,
                        },
                    ),
                ),
                Undo::Rollback(id) => (
                    format!("rolling back {id}"),
                    self.rollback(id, &Undoing).map(|_| ()),
                ),
                Undo::DeclineUpdate(id) => (
                    format!("dropping the update of {id}"),
                    self.decline_update(id),
                ),
                Undo::Disable(id) => (format!("disabling {id}"), self.disable(id)),
                Undo::MarkDependency(id, by) => (
                    format!("marking {id} as a dependency again"),
                    self.mark_dependency_install(id, by),
                ),
            };
            if let Err(e) = result {
                undo_failed.push(format!("{what}: {e}"));
            }
        }
        StoreError::PlanFailed {
            step,
            source: Box::new(error),
            undo_failed,
        }
    }
}

fn check_declared(id: &PluginId, declared: &GrantSet, consent: &Consent) -> Result<()> {
    match consent.keys.iter().find(|key| !declared.contains(key)) {
        Some(key) => Err(StoreError::UnknownGrant {
            id: id.clone(),
            key: key.clone(),
        }),
        None => Ok(()),
    }
}

impl crate::store::PluginStore {
    /// Every installed plugin with an active version, as dependency
    /// resolution sees it. `bundled`: whether a plugin id comes from the
    /// application bundle.
    pub fn dependency_graph(
        &self,
    ) -> Result<std::collections::BTreeMap<PluginId, crate::deps::InstalledPlugin>> {
        let auto = self.dependency_installs()?;
        Ok(self
            .plugins()?
            .into_iter()
            .filter_map(|record| {
                let manifest = &record.active()?.manifest;
                Some((
                    record.id.clone(),
                    crate::deps::InstalledPlugin::from_manifest(
                        manifest,
                        record.state,
                        auto.contains_key(&record.id),
                        record.source == InstallSource::Bundled,
                    ),
                ))
            })
            .collect())
    }
}

impl PluginManager {
    /// What disabling or uninstalling `id` affects, refused with
    /// [`StoreError::RequiredBy`] while an enabled plugin requires it and
    /// is not in `with` (the dependents the user chose to disable along
    /// with it, transitively).
    pub fn check_removal(
        &self,
        id: &PluginId,
        removal: crate::deps::Removal,
        with: &BTreeSet<PluginId>,
    ) -> Result<crate::deps::RemovalPlan> {
        let graph = self.store.dependency_graph()?;
        let plan = crate::deps::plan_removal(&graph, id.as_str(), removal);
        let needed = enabled_dependents(&graph, id.as_str());
        let missing: Vec<String> = needed
            .iter()
            .filter(|d| !with.contains(*d))
            .map(ToString::to_string)
            .collect();
        if !missing.is_empty() {
            return Err(StoreError::RequiredBy {
                id: id.clone(),
                dependents: missing,
            });
        }
        Ok(plan)
    }
}

/// The enabled plugins that require `target`, directly or through other
/// enabled plugins, dependents of dependents first (the order to disable
/// them in).
pub fn enabled_dependents(
    graph: &std::collections::BTreeMap<PluginId, crate::deps::InstalledPlugin>,
    target: &str,
) -> Vec<PluginId> {
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
    visit_dependents(graph, target, &mut seen, &mut order);
    order
}

fn visit_dependents(
    graph: &std::collections::BTreeMap<PluginId, crate::deps::InstalledPlugin>,
    target: &str,
    seen: &mut BTreeSet<String>,
    order: &mut Vec<PluginId>,
) {
    for plugin in graph.values() {
        let requires = plugin
            .dependencies
            .iter()
            .any(|d| d.is_required() && d.id.as_str() == target);
        if !requires || !plugin.is_enabled() || !seen.insert(plugin.id.to_string()) {
            continue;
        }
        visit_dependents(graph, plugin.id.as_str(), seen, order);
        order.push(plugin.id.clone());
    }
}
