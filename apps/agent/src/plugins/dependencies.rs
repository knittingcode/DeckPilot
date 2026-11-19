//! Plugin dependencies in the agent (`docs/marketplace/dependencies.md`):
//! install plans resolved from the signed registry and verified before
//! anything is installed, carried out with the user's consent per plugin;
//! dependency-aware disable and uninstall; the state of each plugin's
//! dependencies for the app and for availability.
//!
//! A child module of `service.rs`, so it works on the service's own state.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use deckpilot_ipc::plugins::{self as ipc, PluginOperation, PluginStage, reason};
use deckpilot_ipc::{ErrorCode, IpcError, Request as IpcRequest, Response};
use deckpilot_plugin_manifest::{Dependency, DependencyTarget, PluginId};
use deckpilot_plugin_package::{Limits, VerifiedPackage, verify_package};
use deckpilot_plugin_store::deps::{
    self, BuiltinModule, Candidate, DependencyState, InstalledPlugin, Plan, Removal, Request,
    ResolveError, StepAction, Universe,
};
use deckpilot_plugin_store::{GrantSet, PermissionDiff, PlannedStep, Retention, TombstoneDecision};
use semver::Version;
use sha2::Digest as _;
use tracing::info;

use super::{PluginService, convert, error, host_environment, plugin_id, store_error};

/// Most plugins the registry walk of one plan reads.
const MAX_LOADED: usize = 64;

/// The verified packages of the last plan, for the install that follows
/// its confirmation (so they are not downloaded twice).
pub(super) struct PlanCache {
    fingerprint: String,
    packages: BTreeMap<PluginId, VerifiedPackage>,
}

impl std::fmt::Debug for PlanCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanCache")
            .field("fingerprint", &self.fingerprint)
            .field("packages", &self.packages.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn dependency_error(message: impl Into<String>) -> IpcError {
    error(ErrorCode::Failed, reason::DEPENDENCY, message)
}

fn ipc_state(state: &DependencyState) -> ipc::DependencyState {
    match state {
        DependencyState::Satisfied { .. } => ipc::DependencyState::Satisfied,
        DependencyState::Disabled { .. } => ipc::DependencyState::Disabled,
        DependencyState::WrongVersion { .. } => ipc::DependencyState::WrongVersion,
        DependencyState::Missing => ipc::DependencyState::Missing,
        DependencyState::Unusable { .. } => ipc::DependencyState::Unusable,
    }
}

fn installed_version(state: &DependencyState) -> Option<String> {
    match state {
        DependencyState::Satisfied { version }
        | DependencyState::Disabled { version }
        | DependencyState::WrongVersion { version } => Some(version.to_string()),
        DependencyState::Missing | DependencyState::Unusable { .. } => None,
    }
}

/// The installed plugins that declare a dependency on `id`.
fn dependents_of(
    id: &str,
    graph: &BTreeMap<PluginId, InstalledPlugin>,
) -> Vec<ipc::PluginDependentInfo> {
    let (required, optional) = deps::dependents(graph, id);
    required
        .iter()
        .chain(&optional)
        .map(dependent_info)
        .collect()
}

fn dependent_info(d: &deps::Dependent) -> ipc::PluginDependentInfo {
    ipc::PluginDependentInfo {
        id: d.id.to_string(),
        name: d.name.clone(),
        enabled: d.enabled,
        version: d.range.to_string(),
        optional: d.optional,
        actions: d.actions.clone(),
    }
}

impl PluginService {
    /// The dependency requests (feature `dependencies`).
    pub(super) fn handle_dependencies(
        &mut self,
        request: IpcRequest,
    ) -> Result<Response, IpcError> {
        match request {
            IpcRequest::PlanPluginInstall {
                plugin,
                version,
                optional,
            } => {
                let result = self.plan_install(&plugin, version.as_deref(), &optional);
                if let Err(e) = &result {
                    self.failed(&plugin, PluginOperation::Install, e);
                }
                result
            }
            IpcRequest::InstallPluginPlan {
                plugin,
                version,
                optional,
                fingerprint,
                consent,
                activate,
            } => self.install_plan(
                &plugin,
                version.as_deref(),
                &optional,
                &fingerprint,
                &consent,
                activate,
            ),
            IpcRequest::PlanPluginRemoval { plugin, uninstall } => {
                self.plan_removal(&plugin, uninstall)
            }
            IpcRequest::RemovePlugin {
                plugin,
                uninstall,
                keep,
                with_dependents,
                orphans,
            } => self.remove(
                &plugin,
                uninstall,
                Retention {
                    settings: keep.settings,
                    credentials: keep.credentials,
                },
                with_dependents,
                &orphans,
            ),
            _ => Err(IpcError::new(
                ErrorCode::InvalidRequest,
                "not a dependency request",
            )),
        }
    }

    /// The built-in modules of the agent's module registry (sent after
    /// every module load).
    pub fn set_builtins(&mut self, builtins: Vec<BuiltinModule>) {
        self.builtins = builtins.into_iter().map(|m| (m.id.clone(), m)).collect();
    }

    /// The installed plugins as dependency resolution sees them.
    pub(super) fn graph(&self) -> BTreeMap<PluginId, InstalledPlugin> {
        self.manager.store().dependency_graph().unwrap_or_default()
    }

    /// `dependencies` with their state here; `actions`: the actions of
    /// `manifest` (if known) that name each one.
    pub(super) fn dependency_infos(
        &self,
        dependencies: &[Dependency],
        manifest: Option<&deckpilot_plugin_manifest::Manifest>,
        graph: &BTreeMap<PluginId, InstalledPlugin>,
    ) -> Vec<ipc::PluginDependencyInfo> {
        deps::dependency_status(dependencies, graph, &self.builtins)
            .into_iter()
            .map(|status| {
                let id = status.dependency.id.to_string();
                let bundled =
                    status.dependency.id.plugin().is_some_and(|p| {
                        deckpilot_plugin_bundled::is_bundled_id(p, &self.bundle.pins)
                    });
                ipc::PluginDependencyInfo {
                    actions: manifest
                        .map(|m| {
                            m.contributes
                                .actions()
                                .iter()
                                .filter(|a| a.requires_dependencies().contains(&id))
                                .map(|a| a.id.clone())
                                .collect()
                        })
                        .unwrap_or_default(),
                    id,
                    version: status.dependency.version.to_string(),
                    optional: status.dependency.is_optional(),
                    justification: status.dependency.justification.clone(),
                    builtin: status.builtin || bundled,
                    name: status.name.clone(),
                    installed_version: installed_version(&status.state),
                    state: ipc_state(&status.state),
                }
            })
            .collect()
    }

    /// Why a plugin's actions cannot run for lack of a required
    /// dependency, and why each of its actions that names an optional one
    /// cannot.
    pub(super) fn unmet(
        &self,
        manifest: &deckpilot_plugin_manifest::Manifest,
        graph: &BTreeMap<PluginId, InstalledPlugin>,
    ) -> (Option<String>, BTreeMap<String, String>) {
        if manifest.dependencies().is_empty() {
            return (None, BTreeMap::new());
        }
        let status = deps::dependency_status(manifest.dependencies(), graph, &self.builtins);
        let plugin = deps::missing_required(&status);
        let actions = manifest
            .contributes
            .actions()
            .iter()
            .filter_map(|a| {
                deps::action_unmet(a.requires_dependencies(), &status)
                    .map(|why| (a.id.clone(), why))
            })
            .collect();
        (plugin, actions)
    }

    /// Everything resolution may use for installing `root`: the installed
    /// plugins, the built-in modules, and the verified registry versions
    /// of every plugin reachable from `root` (pre-releases only for the
    /// requested version of `root`).
    fn universe(
        &self,
        root: &PluginId,
        requested: Option<&Version>,
        optional: &BTreeSet<String>,
    ) -> Result<Universe, IpcError> {
        let host = host_environment().ok_or_else(|| {
            error(
                ErrorCode::Failed,
                reason::INCOMPATIBLE,
                "plugins are not supported on this platform",
            )
        })?;
        let registry = self.registry()?;
        let index = registry.client().index().ok_or_else(|| {
            error(
                ErrorCode::Failed,
                reason::REGISTRY,
                "the plugin registry has never been verified",
            )
        })?;
        let mut available: BTreeMap<PluginId, Vec<Candidate>> = BTreeMap::new();
        let mut queue = vec![root.clone()];
        let mut seen = BTreeSet::new();
        while let Some(id) = queue.pop() {
            if !seen.insert(id.clone()) || seen.len() > MAX_LOADED {
                continue;
            }
            // Bundled plugins come only from the application bundle (they
            // are installed by seeding), never from a registry.
            if deckpilot_plugin_bundled::is_bundled_id(&id, &self.bundle.pins)
                || registry.client().detail_ref(&id).is_none()
            {
                continue;
            }
            let detail = registry
                .detail(&id)
                .map_err(|m| error(ErrorCode::Failed, reason::REGISTRY, m))?;
            let verified = index
                .plugin(&id)
                .and_then(|entry| {
                    index
                        .publishers
                        .iter()
                        .find(|p| p.namespace == entry.publisher)
                })
                .is_some_and(|p| p.verified);
            let mut candidates = Vec::new();
            for v in &detail.versions {
                let exact = &id == root && requested == Some(&v.version);
                if !v.version.pre.is_empty() && !exact {
                    continue;
                }
                let revoked = index
                    .revocations
                    .iter()
                    .find(|r| r.matches(&id, &v.version))
                    .map(|r| r.reason.clone());
                let dependencies = v.dependencies.clone().unwrap_or_default();
                for dependency in &dependencies {
                    let wanted =
                        dependency.is_required() || optional.contains(dependency.id.as_str());
                    if let (true, Some(next)) = (wanted, dependency.id.plugin()) {
                        queue.push(next.clone());
                    }
                }
                candidates.push(Candidate {
                    id: id.clone(),
                    name: detail.name.clone(),
                    version: v.version.clone(),
                    engines: v.engines.clone(),
                    platforms: v.platforms.clone(),
                    dependencies,
                    permissions: v.permissions.clone(),
                    withdrawn: v.yanked.as_ref().map(|y| y.reason.clone()).or(revoked),
                    publisher_verified: verified,
                    bundled: false,
                });
            }
            available.insert(id, candidates);
        }
        Ok(Universe {
            host,
            installed: self.graph(),
            builtins: self.builtins.clone(),
            available,
        })
    }

    /// Resolves the plan of `plugin`; `None` version: the latest stable
    /// one (or the installed one, when it was installed as a dependency).
    fn resolve_plan(
        &mut self,
        plugin: &str,
        requested: Option<&str>,
        optional: &[String],
    ) -> Result<(Universe, Plan), IpcError> {
        let id = plugin_id(plugin)?;
        self.refresh()?;
        let graph = self.graph();
        let version = match (graph.get(&id), requested) {
            (Some(installed), None) if installed.as_dependency => None,
            // An update with what it needs (a newer version only).
            (Some(installed), Some(text))
                if super::version(text).is_ok_and(|v| v > installed.version) =>
            {
                Some(super::version(text)?)
            }
            (Some(installed), _) => {
                return Err(error(
                    ErrorCode::Failed,
                    reason::ALREADY_INSTALLED,
                    format!("{id} {} is already installed", installed.version),
                ));
            }
            (None, requested) => Some(self.pick_version(&id, requested)?),
        };
        let optional: BTreeSet<String> = optional.iter().cloned().collect();
        let universe = self.universe(&id, version.as_ref(), &optional)?;
        let request = Request {
            plugin: id,
            version,
            optional,
            allow_updates: true,
        };
        let plan = deps::resolve(&universe, &request).map_err(|e| self.resolve_error(&e))?;
        Ok((universe, plan))
    }

    /// Before `updatePlugin` installs `version` of `id`: refused if a
    /// plugin that stays installed does not accept that version, or if the
    /// version needs dependencies that are not there (its install plan,
    /// `planPluginInstall` with the version, brings them in).
    pub(super) fn check_update(&self, id: &PluginId, version: &Version) -> Result<(), IpcError> {
        let universe = self.universe(id, Some(version), &BTreeSet::new())?;
        let request = Request {
            plugin: id.clone(),
            version: Some(version.clone()),
            optional: BTreeSet::new(),
            allow_updates: false,
        };
        let plan = deps::resolve(&universe, &request).map_err(|e| self.resolve_error(&e))?;
        let needs: Vec<String> = plan
            .steps
            .iter()
            .filter(|s| !s.requested && s.action.changes())
            .map(|s| format!("{} {}", s.id, s.action.version()))
            .collect();
        if needs.is_empty() {
            Ok(())
        } else {
            Err(dependency_error(format!(
                "{id} {version} needs {}; install the update with its plan",
                needs.join(", ")
            )))
        }
    }

    /// Before a rollback: refused if a plugin that stays enabled does not
    /// accept the version it returns to.
    pub(super) fn check_rollback(&self, id: &PluginId) -> Result<(), IpcError> {
        let Ok(Some(record)) = self.manager.store().plugin(id) else {
            return Ok(());
        };
        let Some(previous) = record.previous() else {
            return Ok(());
        };
        let graph = self.graph();
        let (required, optional) = deps::dependents(&graph, id.as_str());
        let broken: Vec<String> = required
            .iter()
            .chain(&optional)
            .filter(|d| d.enabled && !d.range.matches(&previous.version))
            .map(|d| format!("{} needs {}", d.id, d.range))
            .collect();
        if broken.is_empty() {
            Ok(())
        } else {
            Err(dependency_error(format!(
                "{id} cannot go back to {}: {}",
                previous.version,
                broken.join("; ")
            )))
        }
    }

    fn resolve_error(&self, e: &ResolveError) -> IpcError {
        if let ResolveError::Missing {
            id: DependencyTarget::Plugin(id),
            ..
        } = e
            && deckpilot_plugin_bundled::is_bundled_id(id, &self.bundle.pins)
        {
            return dependency_error(format!(
                "{e}: {id} is part of DeckPilot and is not installed (it was removed); \
                 restore it in Settings › Plugins first"
            ));
        }
        let code = match e {
            ResolveError::Incompatible { .. } => reason::INCOMPATIBLE,
            ResolveError::Withdrawn { .. } => reason::REVOKED,
            _ => reason::DEPENDENCY,
        };
        error(ErrorCode::Failed, code, e.to_string())
    }

    /// The pinned package digests of a plan's downloads, which the
    /// fingerprint covers (no download needed).
    fn fingerprint(&self, plan: &Plan) -> Result<String, IpcError> {
        let mut text = String::new();
        for step in &plan.steps {
            let (action, from) = match &step.action {
                StepAction::Keep { .. } => ("keep", None),
                StepAction::Enable { .. } => ("enable", None),
                StepAction::Install { .. } | StepAction::InstallBundled { .. } => ("install", None),
                StepAction::Update { from, .. } => ("update", Some(from.to_string())),
                StepAction::Builtin { enabled, .. } => {
                    (if *enabled { "builtin" } else { "enableBuiltin" }, None)
                }
            };
            let pinned = match (step.id.plugin(), step.action.candidate()) {
                (Some(id), Some(candidate)) => {
                    let target = self.pins(id, &candidate.version)?;
                    target.expected.package.sha256.to_string()
                }
                _ => String::new(),
            };
            let _ = write!(
                text,
                "{}@{}:{action}:{}:{pinned};",
                step.id,
                step.action.version(),
                from.unwrap_or_default()
            );
        }
        let digest: [u8; 32] = sha2::Sha256::digest(text.as_bytes()).into();
        Ok(digest.iter().fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        }))
    }

    /// Downloads and verifies every package the plan installs or updates
    /// to: the bytes against the registry pins, the manifest's
    /// dependencies against the signed detail, the module against the
    /// plugin host (as an update is). Nothing is installed.
    fn verify_downloads(
        &mut self,
        plan: &Plan,
    ) -> Result<BTreeMap<PluginId, VerifiedPackage>, IpcError> {
        let mut packages = BTreeMap::new();
        for step in plan.downloads() {
            let (Some(id), Some(candidate)) = (step.id.plugin(), step.action.candidate()) else {
                continue;
            };
            self.progress(id.as_str(), PluginOperation::Install, PluginStage::Fetching);
            let (expected, bytes) = self.fetch(id, &candidate.version)?;
            self.progress(
                id.as_str(),
                PluginOperation::Install,
                PluginStage::Verifying,
            );
            let verified = verify_package(&bytes, &expected, &Limits::default())
                .map_err(|e| super::package_error(&e))?;
            if verified.manifest().dependencies() != candidate.dependencies.as_slice() {
                return Err(error(
                    ErrorCode::Failed,
                    reason::INVALID_PACKAGE,
                    format!(
                        "{id} {}: the package declares other dependencies than the signed \
                         registry entry; nothing was installed",
                        candidate.version
                    ),
                ));
            }
            self.trial_load(id, &verified)?;
            packages.insert(id.clone(), verified);
        }
        Ok(packages)
    }

    /// `planPluginInstall`.
    pub(super) fn plan_install(
        &mut self,
        plugin: &str,
        requested: Option<&str>,
        optional: &[String],
    ) -> Result<Response, IpcError> {
        let (universe, plan) = self.resolve_plan(plugin, requested, optional)?;
        let fingerprint = self.fingerprint(&plan)?;
        let packages = self.verify_downloads(&plan)?;
        let answer = self.plan_answer(&universe, &plan, &packages, fingerprint.clone());
        self.plan_cache = Some(PlanCache {
            fingerprint,
            packages,
        });
        Ok(Response::PluginInstallPlan {
            plan: Box::new(answer),
        })
    }

    fn plan_answer(
        &self,
        universe: &Universe,
        plan: &Plan,
        packages: &BTreeMap<PluginId, VerifiedPackage>,
        fingerprint: String,
    ) -> ipc::PluginInstallPlan {
        let steps = plan
            .steps
            .iter()
            .map(|step| self.plan_step(step, universe, packages))
            .collect();
        let optional = plan
            .optional
            .iter()
            .map(|o| {
                let manifest = packages.get(&o.of).map(VerifiedPackage::manifest);
                let mut info = self
                    .dependency_infos(
                        std::slice::from_ref(&o.dependency),
                        manifest,
                        &universe.installed,
                    )
                    .remove(0);
                if info.actions.is_empty() {
                    info.actions.clone_from(&o.actions);
                }
                self.name_from_registry(std::slice::from_mut(&mut info));
                ipc::PluginOptionalIntegration {
                    of: o.of.to_string(),
                    dependency: info,
                }
            })
            .collect();
        let root = plan.steps.iter().find(|s| s.requested);
        ipc::PluginInstallPlan {
            plugin: plan.plugin.to_string(),
            version: root
                .map(|s| s.action.version().to_string())
                .unwrap_or_default(),
            steps,
            optional,
            fingerprint,
        }
    }

    /// One step of a plan as the app shows it.
    fn plan_step(
        &self,
        step: &deps::Step,
        universe: &Universe,
        packages: &BTreeMap<PluginId, VerifiedPackage>,
    ) -> ipc::PluginPlanStep {
        let index = self.registry.as_ref().and_then(|r| r.client().index());
        let id = step.id.plugin();
        let installed = id.and_then(|id| universe.installed.get(id));
        let package = id.and_then(|id| packages.get(id));
        let record = id.and_then(|id| self.manager.store().plugin(id).ok().flatten());
        let stored = id
            .and_then(|id| self.manager.store().grants(id).ok())
            .unwrap_or_default();
        let (action, permissions, blocker) = match &step.action {
            StepAction::Keep { .. } => (ipc::PlanStepAction::Keep, Vec::new(), None),
            StepAction::Builtin { enabled: true, .. } => {
                (ipc::PlanStepAction::Builtin, Vec::new(), None)
            }
            StepAction::Builtin { enabled: false, .. } => (
                ipc::PlanStepAction::EnableBuiltin,
                Vec::new(),
                Some(format!(
                    "turn on the built-in module {} in Settings › Modules first",
                    step.name
                )),
            ),
            StepAction::Enable { .. } => {
                let manifest = record
                    .as_ref()
                    .and_then(|r| r.active())
                    .map(|v| &v.manifest);
                (
                    ipc::PlanStepAction::Enable,
                    manifest
                        .map(|m| convert::permissions(m, &stored))
                        .unwrap_or_default(),
                    None,
                )
            }
            StepAction::Install { .. } | StepAction::InstallBundled { .. } => (
                ipc::PlanStepAction::Install,
                package
                    .map(|p| convert::permissions(p.manifest(), &[]))
                    .unwrap_or_default(),
                None,
            ),
            StepAction::Update { .. } => (
                ipc::PlanStepAction::Update,
                package
                    .map(|p| {
                        let diff = PermissionDiff::compute(
                            &stored,
                            &GrantSet::from_manifest(p.manifest()),
                        );
                        let changes = convert::changes(&diff, p.manifest());
                        changes.added.into_iter().chain(changes.broadened).collect()
                    })
                    .unwrap_or_default(),
                None,
            ),
        };
        let publisher = id.and_then(|id| {
            let index = index?;
            let entry = index.plugin(id)?;
            index
                .publishers
                .iter()
                .find(|p| p.namespace == entry.publisher)
                .map(|p| ipc::PublisherInfo {
                    namespace: p.namespace.clone(),
                    display_name: p.display_name.clone(),
                    login: p.login.clone(),
                    verified: p.verified,
                })
        });
        ipc::PluginPlanStep {
            id: step.id.to_string(),
            name: if step.name.is_empty() {
                step.id.to_string()
            } else {
                step.name.clone()
            },
            version: step.action.version().to_string(),
            action,
            from_version: match &step.action {
                StepAction::Update { from, .. } => Some(from.to_string()),
                _ => None,
            },
            requested: step.requested,
            optional: step.optional,
            builtin: step.id.is_builtin()
                || installed.is_some_and(|p| p.bundled)
                || matches!(step.action, StepAction::InstallBundled { .. }),
            required_by: step
                .required_by
                .iter()
                .filter(|c| c.from.is_some())
                .map(ToString::to_string)
                .collect(),
            permissions,
            publisher,
            blocker,
        }
    }

    /// `installPluginPlan`: the plan again (refused if it is not the one
    /// confirmed), its packages verified, then all or nothing.
    pub(super) fn install_plan(
        &mut self,
        plugin: &str,
        requested: Option<&str>,
        optional: &[String],
        fingerprint: &str,
        consent: &BTreeMap<String, Vec<String>>,
        activate: bool,
    ) -> Result<Response, IpcError> {
        let (_, plan) = self.resolve_plan(plugin, requested, optional)?;
        let current = self.fingerprint(&plan)?;
        if current != fingerprint {
            return Err(error(
                ErrorCode::Failed,
                reason::PLAN_CHANGED,
                "what installing this plugin takes has changed since the plan was shown; review \
                 the new plan",
            ));
        }
        if let Some(step) = plan
            .steps
            .iter()
            .find(|s| matches!(s.action, StepAction::Builtin { enabled: false, .. }))
        {
            return Err(dependency_error(format!(
                "{} needs the built-in module {} ({}); turn it on in Settings › Modules first",
                plan.plugin, step.name, step.id
            )));
        }
        // Consent only for the plugins of this plan, each for its own
        // permissions (checked again by the store).
        let in_plan: BTreeSet<&str> = plan.steps.iter().map(|s| s.id.as_str()).collect();
        if let Some(stray) = consent.keys().find(|k| !in_plan.contains(k.as_str())) {
            return Err(error(
                ErrorCode::InvalidRequest,
                reason::UNKNOWN_PERMISSION,
                format!(
                    "{stray} is not part of this plan; consent is given per plugin of the plan"
                ),
            ));
        }
        let packages = match self.plan_cache.take() {
            Some(cache) if cache.fingerprint == current => cache.packages,
            _ => self.verify_downloads(&plan)?,
        };
        let root = plan.plugin.clone();
        let (steps, reopen) = self.planned_steps(&plan, &packages, consent)?;
        if activate {
            // Turning a first-party plugin the user turned off back on is
            // the user's decision, confirmed in the plan.
            for id in &reopen {
                if let Ok(Some(t)) = self.manager.store().tombstone(id)
                    && t.decision == TombstoneDecision::Disabled
                {
                    self.manager
                        .clear_tombstone(id)
                        .map_err(|e| store_error(&e))?;
                }
            }
        }
        self.progress(
            root.as_str(),
            PluginOperation::Install,
            PluginStage::Installing,
        );
        let applied = self.manager.apply_plan(&steps, activate);
        drop(steps);
        let applied = match applied {
            Ok(applied) => applied,
            Err(e) => {
                let e = store_error(&e);
                self.failed(root.as_str(), PluginOperation::Install, &e);
                for step in &plan.steps {
                    if let Some(id) = step.id.plugin() {
                        self.changed(id);
                    }
                }
                return Err(e);
            }
        };
        for (id, _) in &applied.updated {
            self.after_version_change(id);
        }
        for step in &plan.steps {
            if let Some(id) = step.id.plugin() {
                self.sync_grants(id);
                self.changed(id);
            }
        }
        info!(plugin = %root, ?applied, "plugin installed with its dependencies");
        self.progress(
            root.as_str(),
            PluginOperation::Install,
            PluginStage::Finished,
        );
        self.details(&root)
    }

    /// The store steps of a confirmed plan, each plugin with its own
    /// consent, and the installed plugins it enables.
    fn planned_steps<'a>(
        &self,
        plan: &Plan,
        packages: &'a BTreeMap<PluginId, VerifiedPackage>,
        consent: &BTreeMap<String, Vec<String>>,
    ) -> Result<(Vec<PlannedStep<'a>>, Vec<PluginId>), IpcError> {
        let consent_of = |id: &str| -> Result<deckpilot_plugin_store::Consent, IpcError> {
            Self::consent(consent.get(id).map(Vec::as_slice).unwrap_or_default())
        };
        let graph = self.graph();
        let root = &plan.plugin;
        let source = self.install_source();
        let mut steps: Vec<PlannedStep<'_>> = Vec::new();
        let mut reopen = Vec::new();
        for step in &plan.steps {
            let Some(id) = step.id.plugin() else {
                continue;
            };
            let dependency_of = (!step.requested).then(|| root.clone());
            match &step.action {
                StepAction::Install { .. } => {
                    let package = packages.get(id).ok_or_else(|| {
                        dependency_error(format!("{id}: its package was not verified"))
                    })?;
                    steps.push(PlannedStep::Install {
                        package,
                        source: source.clone(),
                        consent: consent_of(id.as_str())?,
                        dependency_of,
                    });
                }
                StepAction::Update { .. } => {
                    let package = packages.get(id).ok_or_else(|| {
                        dependency_error(format!("{id}: its package was not verified"))
                    })?;
                    steps.push(PlannedStep::Update {
                        package,
                        source: source.clone(),
                        consent: consent_of(id.as_str())?,
                    });
                }
                StepAction::Enable { .. } => {
                    reopen.push(id.clone());
                    steps.push(PlannedStep::Enable {
                        id: id.clone(),
                        consent: consent_of(id.as_str())?,
                    });
                }
                StepAction::Keep { .. } if step.requested => {
                    if graph.get(id).is_some_and(|p| p.as_dependency) {
                        steps.push(PlannedStep::Adopt { id: id.clone() });
                    }
                }
                StepAction::InstallBundled { .. } => {
                    return Err(dependency_error(format!(
                        "{id} is part of DeckPilot; restore it in Settings › Plugins first"
                    )));
                }
                StepAction::Keep { .. } | StepAction::Builtin { .. } => {}
            }
        }
        Ok((steps, reopen))
    }

    /// `planPluginRemoval`.
    pub(super) fn plan_removal(&self, plugin: &str, uninstall: bool) -> Result<Response, IpcError> {
        let id = plugin_id(plugin)?;
        let graph = self.graph();
        let removal = if uninstall {
            Removal::Uninstall
        } else {
            Removal::Disable
        };
        let plan = deps::plan_removal(&graph, id.as_str(), removal);
        let orphans = plan
            .orphans
            .iter()
            .filter_map(|o| graph.get(o))
            .map(|p| ipc::PluginDependentInfo {
                id: p.id.to_string(),
                name: p.name.clone(),
                enabled: p.is_enabled(),
                version: p.version.to_string(),
                optional: false,
                actions: Vec::new(),
            })
            .collect();
        Ok(Response::PluginRemovalPlan {
            plan: Box::new(ipc::PluginRemovalPlan {
                plugin: id.to_string(),
                uninstall,
                required_by: plan.required_by.iter().map(dependent_info).collect(),
                disables: deckpilot_plugin_store::enabled_dependents(&graph, id.as_str())
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                used_by: plan.used_by.iter().map(dependent_info).collect(),
                orphans,
            }),
        })
    }

    /// Refuses to disable or uninstall a plugin enabled plugins require
    /// (`disablePlugin` and `uninstallPlugin`, which have no way to say
    /// "with its dependents").
    pub(super) fn guard_removal(&self, id: &PluginId, removal: Removal) -> Result<(), IpcError> {
        self.manager
            .check_removal(id, removal, &BTreeSet::new())
            .map(|_| ())
            .map_err(|e| store_error(&e))
    }

    /// `removePlugin`.
    pub(super) fn remove(
        &mut self,
        plugin: &str,
        uninstall: bool,
        keep: Retention,
        with_dependents: bool,
        orphans: &[String],
    ) -> Result<Response, IpcError> {
        let id = plugin_id(plugin)?;
        let removal = if uninstall {
            Removal::Uninstall
        } else {
            Removal::Disable
        };
        let graph = self.graph();
        let dependents = deckpilot_plugin_store::enabled_dependents(&graph, id.as_str());
        let with: BTreeSet<PluginId> = if with_dependents {
            dependents.iter().cloned().collect()
        } else {
            BTreeSet::new()
        };
        self.manager
            .check_removal(&id, removal, &with)
            .map_err(|e| store_error(&e))?;
        let orphans: Vec<PluginId> = orphans
            .iter()
            .map(|o| plugin_id(o))
            .collect::<Result<_, _>>()?;
        if !orphans.is_empty() {
            let offered = deps::orphans_after(&graph, &[id.as_str()]);
            if let Some(other) = orphans.iter().find(|o| !offered.contains(o)) {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    reason::INVALID_STATE,
                    format!(
                        "{other} is not a dependency that nothing needs after removing {id}; it \
                         is not removed"
                    ),
                ));
            }
            if !uninstall {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    reason::INVALID_STATE,
                    "orphaned dependencies are only removed with an uninstall",
                ));
            }
        }
        for dependent in &dependents {
            info!(plugin = %dependent, because = %id, "disabling a plugin that requires one being removed");
            self.disable(dependent)?;
        }
        if uninstall {
            self.uninstall(&id, keep)?;
            for orphan in deps::orphans_after(&self.graph(), &[])
                .into_iter()
                .filter(|o| orphans.contains(o))
            {
                info!(plugin = %orphan, "uninstalling a dependency nothing needs");
                self.uninstall(&orphan, keep)?;
            }
        } else if graph.get(&id).is_some_and(InstalledPlugin::is_enabled) {
            self.disable(&id)?;
        }
        Ok(Response::Ok)
    }

    /// Marks the summary's actions that cannot run for lack of a
    /// dependency (every action for a required one).
    pub(super) fn apply_unmet(
        &self,
        summary: &mut ipc::PluginSummary,
        record: &deckpilot_plugin_store::PluginRecord,
        graph: &BTreeMap<PluginId, InstalledPlugin>,
    ) {
        let Some(manifest) = record.active().map(|v| &v.manifest) else {
            return;
        };
        let (missing, unmet) = self.unmet(manifest, graph);
        for action in &mut summary.actions {
            if !action.available {
                continue;
            }
            if let Some(why) = missing
                .clone()
                .or_else(|| unmet.get(&action.action).cloned())
            {
                action.available = false;
                action.unavailable_reason = Some(why);
            }
        }
    }

    /// The details' dependency fields.
    pub(super) fn add_dependencies(&self, details: &mut ipc::PluginDetails, id: &PluginId) {
        let graph = self.graph();
        if let Ok(Some(record)) = self.manager.store().plugin(id) {
            self.apply_unmet(&mut details.summary, &record, &graph);
        }
        if let Some(plugin) = graph.get(id) {
            let manifest = self
                .manager
                .store()
                .plugin(id)
                .ok()
                .flatten()
                .and_then(|r| r.active().map(|v| v.manifest.clone()));
            details.dependencies =
                self.dependency_infos(&plugin.dependencies, manifest.as_ref(), &graph);
        }
        details.dependents = dependents_of(id.as_str(), &graph);
        details.summary.installed_for = self
            .manager
            .store()
            .dependency_installs()
            .ok()
            .and_then(|mut m| m.remove(id));
    }

    /// One registry version's dependencies with their state here.
    pub(super) fn available_dependencies(
        &self,
        dependencies: &[Dependency],
        graph: &BTreeMap<PluginId, InstalledPlugin>,
    ) -> Vec<ipc::PluginDependencyInfo> {
        let mut infos = self.dependency_infos(dependencies, None, graph);
        self.name_from_registry(&mut infos);
        infos
    }

    /// Names the dependencies that are not installed as the registry
    /// index does.
    fn name_from_registry(&self, infos: &mut [ipc::PluginDependencyInfo]) {
        let Some(index) = self.registry.as_ref().and_then(|r| r.client().index()) else {
            return;
        };
        for info in infos.iter_mut().filter(|i| i.name.is_none()) {
            info.name = PluginId::parse(&info.id)
                .ok()
                .and_then(|id| index.plugin(&id))
                .map(|entry| entry.name.clone());
        }
    }
}
