//! Dependency resolution (`docs/marketplace/dependencies.md`).
//!
//! Pure functions over a snapshot of what the host knows ([`Universe`]):
//! the installed plugins, the built-in modules and the registry's signed
//! versions. Nothing here downloads, installs or grants anything; the
//! caller turns a [`Plan`] into store operations only after every package
//! of it verified and the user confirmed it (with consent per plugin).
//!
//! # Resolution
//!
//! [`resolve`] walks the required dependencies of the requested plugin
//! recursively (and the optional ones the user selected), depth first in
//! manifest order, so the same inputs always give the same plan:
//!
//! - a dependency already installed in a version every constraint accepts
//!   is kept (no needless updates); a built-in module is checked against
//!   DeckPilot's version and never downloaded;
//! - otherwise the **highest** registry (or application bundle) version
//!   that satisfies every constraint, is not yanked or revoked, runs on
//!   this DeckPilot (app version, plugin API, platform) and, for a
//!   dependency, comes from a publisher the registry verified;
//! - constraints are those of the chosen versions **and of every installed
//!   plugin** that stays as it is, so a shared dependency is never moved to
//!   a version another installed plugin does not accept;
//! - a dependency cycle, a missing dependency, a version conflict and an
//!   incompatible or withdrawn version are errors naming the plugins
//!   involved.
//!
//! When a constraint found later rules out a version chosen earlier, the
//! walk starts again with every constraint seen so far (bounded by
//! [`MAX_ROUNDS`]). Constraints are kept across rounds, so in rare graphs
//! where only a discarded version imposed one this reports a conflict a
//! complete solver could avoid; it never returns a plan that violates a
//! constraint.
//!
//! # Removal
//!
//! [`plan_removal`] lists what disabling or uninstalling a plugin would
//! affect: enabled plugins that require it (the removal is refused unless
//! the user also disables them), plugins whose optional actions stop, and
//! the plugins installed only as its dependencies that nothing else needs
//! (offered for removal, never removed without confirmation).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use deckpilot_plugin_manifest::{
    Dependency, DependencyTarget, Engines, HostEnvironment, Incompatibility, Manifest, Platform,
    PluginId, VersionRange,
};
use semver::Version;

use crate::state::PluginState;

/// Walks of the graph before [`resolve`] gives up with a conflict.
pub const MAX_ROUNDS: usize = 32;
/// Most plugins one plan may hold (the requested one included).
pub const MAX_PLAN_NODES: usize = 32;

/// An installed plugin as resolution sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub id: PluginId,
    pub name: String,
    /// The active version.
    pub version: Version,
    pub state: PluginState,
    /// The active version's `dependencies`.
    pub dependencies: Vec<Dependency>,
    /// Installed only because another plugin needed it
    /// ([`crate::PluginStore::dependency_installs`]).
    pub as_dependency: bool,
    /// Shipped in the application bundle.
    pub bundled: bool,
    /// The active version's actions with the dependencies each one needs
    /// beyond the required ones (`requiresDependencies`).
    pub action_dependencies: Vec<(String, Vec<String>)>,
}

impl InstalledPlugin {
    /// From a store record's active manifest.
    pub fn from_manifest(
        manifest: &Manifest,
        state: PluginState,
        as_dependency: bool,
        bundled: bool,
    ) -> Self {
        Self {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            state,
            dependencies: manifest.dependencies().to_vec(),
            as_dependency,
            bundled,
            action_dependencies: manifest
                .contributes
                .actions()
                .iter()
                .filter(|a| !a.requires_dependencies().is_empty())
                .map(|a| (a.id.clone(), a.requires_dependencies().to_vec()))
                .collect(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.state == PluginState::Enabled
    }
}

/// A built-in module of DeckPilot (versioned with the app).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinModule {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub enabled: bool,
    /// It runs on this operating system.
    pub supported: bool,
}

/// One version a plugin could be installed in: from the verified registry
/// detail, or from the application bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: PluginId,
    pub name: String,
    pub version: Version,
    pub engines: Engines,
    pub platforms: Option<Vec<Platform>>,
    pub dependencies: Vec<Dependency>,
    /// The permissions shown for consent (manifest form).
    pub permissions: Vec<serde_json::Value>,
    /// Yanked or revoked: never chosen.
    pub withdrawn: Option<String>,
    /// The registry lists the publisher as verified (always true for the
    /// application bundle).
    pub publisher_verified: bool,
    /// From the application bundle (verified against the pins compiled
    /// into the agent), never downloaded.
    pub bundled: bool,
}

impl Candidate {
    fn incompatibilities(&self, host: &HostEnvironment) -> Vec<Incompatibility> {
        let mut reasons = Vec::new();
        if !self.engines.deckpilot.matches(&host.app_version) {
            reasons.push(Incompatibility::AppVersion {
                required: self.engines.deckpilot.clone(),
                actual: host.app_version.clone(),
            });
        }
        if !self.engines.plugin_api.is_satisfied_by(host.plugin_api) {
            reasons.push(Incompatibility::PluginApi {
                required: self.engines.plugin_api,
                actual: host.plugin_api,
            });
        }
        if let Some(platforms) = &self.platforms
            && !platforms.contains(&host.platform)
        {
            reasons.push(Incompatibility::Platform {
                supported: platforms.clone(),
                actual: host.platform,
            });
        }
        reasons
    }
}

/// Everything resolution may use.
#[derive(Debug, Clone)]
pub struct Universe {
    pub host: HostEnvironment,
    pub installed: BTreeMap<PluginId, InstalledPlugin>,
    pub builtins: BTreeMap<String, BuiltinModule>,
    /// Every known version per plugin, in any order.
    pub available: BTreeMap<PluginId, Vec<Candidate>>,
}

/// What to resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub plugin: PluginId,
    /// That exact version; `None`: the best one (the installed one if it
    /// satisfies everything, for a plugin already installed).
    pub version: Option<Version>,
    /// Optional integrations the user chose to install too, by id. An
    /// optional dependency not listed here is never installed.
    pub optional: BTreeSet<String>,
    /// Whether installed dependencies may be updated to satisfy a
    /// constraint (never to a version another installed plugin refuses).
    pub allow_updates: bool,
}

impl Request {
    pub fn install(plugin: PluginId) -> Self {
        Self {
            plugin,
            version: None,
            optional: BTreeSet::new(),
            allow_updates: true,
        }
    }
}

/// Who imposed a constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    /// The plugin that declares the dependency, or `None` for the user's
    /// request.
    pub from: Option<PluginId>,
    /// The version of `from` that declares it.
    pub from_version: Option<Version>,
    pub range: Option<VersionRange>,
    /// `from` is installed and stays as it is.
    pub installed: bool,
    pub optional: bool,
}

impl Constraint {
    fn accepts(&self, version: &Version, exact: Option<&Version>) -> bool {
        exact.is_none_or(|v| v == version) && self.range.as_ref().is_none_or(|r| r.matches(version))
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let range = self
            .range
            .as_ref()
            .map_or("any version", VersionRange::as_str);
        match (&self.from, &self.from_version) {
            (Some(from), Some(version)) => {
                let installed = if self.installed { ", installed" } else { "" };
                write!(f, "{from} {version}{installed} needs {range}")
            }
            _ => write!(f, "requested: {range}"),
        }
    }
}

/// What the plan does with one plugin or module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAction {
    /// Already installed and enabled in a version that fits: nothing to do.
    Keep { version: Version },
    /// Installed and disabled: enabled after the install (with consent to
    /// whatever of its permissions is not granted).
    Enable { version: Version },
    /// Installed from the registry (downloaded and verified first).
    Install { candidate: Box<Candidate> },
    /// Installed from the application bundle (never downloaded).
    InstallBundled { candidate: Box<Candidate> },
    /// An installed dependency moved to a newer version.
    Update {
        from: Version,
        candidate: Box<Candidate>,
    },
    /// A built-in module of DeckPilot: never downloaded; `enabled` says
    /// whether it is on already.
    Builtin { version: Version, enabled: bool },
}

impl StepAction {
    /// The version the plugin or module has once the plan ran.
    pub fn version(&self) -> &Version {
        match self {
            Self::Keep { version } | Self::Enable { version } | Self::Builtin { version, .. } => {
                version
            }
            Self::Install { candidate }
            | Self::InstallBundled { candidate }
            | Self::Update { candidate, .. } => &candidate.version,
        }
    }

    /// The candidate a step installs or updates to.
    pub fn candidate(&self) -> Option<&Candidate> {
        match self {
            Self::Install { candidate }
            | Self::InstallBundled { candidate }
            | Self::Update { candidate, .. } => Some(candidate),
            _ => None,
        }
    }

    /// Whether the step changes anything.
    pub fn changes(&self) -> bool {
        !matches!(
            self,
            Self::Keep { .. } | Self::Builtin { enabled: true, .. }
        )
    }

    /// The dependencies of the version the step leaves.
    fn dependencies<'a>(
        &'a self,
        universe: &'a Universe,
        id: &DependencyTarget,
    ) -> &'a [Dependency] {
        match self {
            Self::Install { candidate }
            | Self::InstallBundled { candidate }
            | Self::Update { candidate, .. } => &candidate.dependencies,
            Self::Keep { .. } | Self::Enable { .. } => id
                .plugin()
                .and_then(|id| universe.installed.get(id))
                .map_or(&[], |p| p.dependencies.as_slice()),
            Self::Builtin { .. } => &[],
        }
    }
}

/// One plugin or module of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: DependencyTarget,
    pub name: String,
    pub action: StepAction,
    /// The plugin the user asked for.
    pub requested: bool,
    /// Every constraint on it (the user's request, the dependents).
    pub required_by: Vec<Constraint>,
    /// Only optional dependents need it (an integration the user chose).
    pub optional: bool,
}

/// An optional integration of a plugin in the plan that is not part of
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalIntegration {
    pub of: PluginId,
    pub dependency: Dependency,
    /// It is installed already (in a version that may or may not fit).
    pub installed: Option<Version>,
    /// The actions of `of` that need it.
    pub actions: Vec<String>,
}

/// A resolved plan, dependencies before their dependents (the requested
/// plugin last).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub plugin: PluginId,
    pub steps: Vec<Step>,
    /// Optional integrations left out, for the user to choose.
    pub optional: Vec<OptionalIntegration>,
}

impl Plan {
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id.as_str() == id)
    }

    /// The steps that install or update a plugin from the registry: the
    /// packages to download and verify before anything is installed.
    pub fn downloads(&self) -> impl Iterator<Item = &Step> {
        self.steps
            .iter()
            .filter(|s| {
                matches!(
                    s.action,
                    StepAction::Install { .. } | StepAction::Update { .. }
                )
            })
            .filter(|s| !s.action.candidate().is_some_and(|c| c.bundled))
    }
}

/// Why no plan exists.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("the dependencies form a cycle: {}", .0.join(" → "))]
    Cycle(Vec<String>),
    #[error("{id} is needed ({}) but is not available{}", list(.required_by), builtin_hint(.id))]
    Missing {
        id: DependencyTarget,
        required_by: Vec<Constraint>,
    },
    #[error("no version of {id} satisfies every constraint ({}); available: {}", list(.constraints), versions(.available))]
    Conflict {
        id: DependencyTarget,
        constraints: Vec<Constraint>,
        available: Vec<Version>,
    },
    #[error("{id} {version} cannot run here: {}", .reasons.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Incompatible {
        id: DependencyTarget,
        version: Version,
        reasons: Vec<Incompatibility>,
    },
    #[error("{id} {version} was withdrawn by the registry: {reason}")]
    Withdrawn {
        id: DependencyTarget,
        version: Version,
        reason: String,
    },
    #[error(
        "{id} is published by an unverified publisher; install it yourself first if you trust it"
    )]
    UnverifiedPublisher { id: PluginId },
    #[error("{id} cannot be used: {reason}")]
    Unavailable {
        id: DependencyTarget,
        reason: String,
    },
    #[error("{id} is installed in {installed}, which does not satisfy {}, and updating it is not allowed here", list(.constraints))]
    UpdateNeeded {
        id: PluginId,
        installed: Version,
        constraints: Vec<Constraint>,
    },
    #[error("the plan needs more than {MAX_PLAN_NODES} plugins")]
    TooLarge,
}

fn list(constraints: &[Constraint]) -> String {
    constraints
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn versions(available: &[Version]) -> String {
    if available.is_empty() {
        "none".into()
    } else {
        available
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn builtin_hint(id: &DependencyTarget) -> &'static str {
    if id.is_builtin() {
        " (no built-in module of this DeckPilot has that id)"
    } else {
        ""
    }
}

struct Walk<'a> {
    universe: &'a Universe,
    request: &'a Request,
    /// Sticky across rounds.
    constraints: BTreeMap<String, Vec<Constraint>>,
    chosen: BTreeMap<String, (String, StepAction)>,
    order: Vec<DependencyTarget>,
    optional_only: BTreeSet<String>,
}

enum Interrupt {
    Restart,
    Fail(ResolveError),
}

impl From<ResolveError> for Interrupt {
    fn from(error: ResolveError) -> Self {
        Self::Fail(error)
    }
}

/// Resolves what installing (or updating to) `request` takes; see the
/// module documentation.
pub fn resolve(universe: &Universe, request: &Request) -> Result<Plan, ResolveError> {
    let root = DependencyTarget::Plugin(request.plugin.clone());
    let mut walk = Walk {
        universe,
        request,
        constraints: BTreeMap::new(),
        chosen: BTreeMap::new(),
        order: Vec::new(),
        optional_only: BTreeSet::new(),
    };
    walk.constraints.insert(
        root.as_str().to_owned(),
        vec![Constraint {
            from: None,
            from_version: None,
            range: None,
            installed: false,
            optional: false,
        }],
    );
    for _ in 0..MAX_ROUNDS {
        walk.chosen.clear();
        walk.order.clear();
        walk.optional_only.clear();
        match walk.visit(&root, &mut Vec::new(), false) {
            Ok(()) => return Ok(walk.plan(&root)),
            Err(Interrupt::Restart) => {}
            Err(Interrupt::Fail(error)) => return Err(error),
        }
    }
    let constraints = walk
        .constraints
        .get(root.as_str())
        .cloned()
        .unwrap_or_default();
    Err(ResolveError::Conflict {
        id: root,
        constraints,
        available: Vec::new(),
    })
}

impl Walk<'_> {
    fn visit(
        &mut self,
        id: &DependencyTarget,
        path: &mut Vec<String>,
        optional: bool,
    ) -> Result<(), Interrupt> {
        if path.iter().any(|p| p == id.as_str()) {
            let mut cycle = path.clone();
            cycle.push(id.as_str().to_owned());
            let start = cycle.iter().position(|p| p == id.as_str()).unwrap_or(0);
            return Err(ResolveError::Cycle(cycle[start..].to_vec()).into());
        }
        if let Some((_, action)) = self.chosen.get(id.as_str()) {
            if !optional {
                self.optional_only.remove(id.as_str());
            }
            let version = action.version().clone();
            return if self.accepts(id, &version) {
                Ok(())
            } else {
                Err(Interrupt::Restart)
            };
        }
        if self.chosen.len() >= MAX_PLAN_NODES {
            return Err(ResolveError::TooLarge.into());
        }
        let (name, action) = self.select(id)?;
        let dependencies: Vec<Dependency> = action.dependencies(self.universe, id).to_vec();
        let version = action.version().clone();
        self.chosen.insert(id.as_str().to_owned(), (name, action));
        if optional {
            self.optional_only.insert(id.as_str().to_owned());
        }
        path.push(id.as_str().to_owned());
        let from = id.plugin().cloned();
        let installed_unchanged = matches!(
            self.chosen.get(id.as_str()).map(|(_, a)| a),
            Some(StepAction::Keep { .. } | StepAction::Enable { .. })
        );
        for dependency in &dependencies {
            let selected = self.request.optional.contains(dependency.id.as_str());
            if dependency.is_optional() && !selected {
                continue;
            }
            let constraint = Constraint {
                from: from.clone(),
                from_version: Some(version.clone()),
                range: Some(dependency.version.clone()),
                installed: installed_unchanged,
                optional: dependency.is_optional(),
            };
            let list = self
                .constraints
                .entry(dependency.id.as_str().to_owned())
                .or_default();
            if !list.contains(&constraint) {
                list.push(constraint);
            }
            self.visit(&dependency.id, path, optional || dependency.is_optional())?;
        }
        path.pop();
        self.order.push(id.clone());
        Ok(())
    }

    /// Every constraint on `id` now: the sticky ones, plus those of the
    /// installed plugins that stay as they are (not the requested plugin,
    /// not one this plan updates). An installed plugin's optional
    /// dependency counts only while the dependency is installed.
    fn constraints_on(&self, id: &DependencyTarget) -> Vec<Constraint> {
        let mut all = self
            .constraints
            .get(id.as_str())
            .cloned()
            .unwrap_or_default();
        let dependency_installed = id
            .plugin()
            .is_some_and(|p| self.universe.installed.contains_key(p))
            || id.is_builtin();
        for plugin in self.universe.installed.values() {
            if plugin.id == self.request.plugin {
                continue;
            }
            if let Some((_, action)) = self.chosen.get(plugin.id.as_str())
                && matches!(action, StepAction::Update { .. })
            {
                continue;
            }
            for dependency in &plugin.dependencies {
                if dependency.id != *id || (dependency.is_optional() && !dependency_installed) {
                    continue;
                }
                let constraint = Constraint {
                    from: Some(plugin.id.clone()),
                    from_version: Some(plugin.version.clone()),
                    range: Some(dependency.version.clone()),
                    installed: true,
                    optional: dependency.is_optional(),
                };
                if !all.contains(&constraint) {
                    all.push(constraint);
                }
            }
        }
        all
    }

    fn exact(&self, id: &DependencyTarget) -> Option<&Version> {
        (id.plugin() == Some(&self.request.plugin))
            .then_some(self.request.version.as_ref())
            .flatten()
    }

    fn accepts(&self, id: &DependencyTarget, version: &Version) -> bool {
        let exact = self.exact(id);
        self.constraints_on(id)
            .iter()
            .all(|c| c.accepts(version, exact))
    }

    fn select(&self, id: &DependencyTarget) -> Result<(String, StepAction), ResolveError> {
        let constraints = self.constraints_on(id);
        let exact = self.exact(id);
        let fits = |version: &Version| constraints.iter().all(|c| c.accepts(version, exact));
        let plugin = match id {
            DependencyTarget::Builtin(name) => return self.select_builtin(id, name, constraints),
            DependencyTarget::Plugin(plugin) => plugin,
        };
        let installed = self.universe.installed.get(plugin);
        if let Some(installed) = installed
            && fits(&installed.version)
        {
            return match installed.state {
                PluginState::Enabled => Ok((
                    installed.name.clone(),
                    StepAction::Keep {
                        version: installed.version.clone(),
                    },
                )),
                PluginState::Disabled => Ok((
                    installed.name.clone(),
                    StepAction::Enable {
                        version: installed.version.clone(),
                    },
                )),
                state => Err(ResolveError::Unavailable {
                    id: id.clone(),
                    reason: format!("it is {state}"),
                }),
            };
        }
        let candidates = self
            .universe
            .available
            .get(plugin)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if candidates.is_empty() && installed.is_none() {
            return Err(ResolveError::Missing {
                id: id.clone(),
                required_by: constraints,
            });
        }
        let mut sorted: Vec<&Candidate> = candidates.iter().collect();
        sorted.sort_by(|a, b| b.version.cmp(&a.version));
        sorted.dedup_by(|a, b| a.version == b.version);
        let fitting: Vec<&Candidate> = sorted
            .iter()
            .copied()
            .filter(|c| fits(&c.version))
            .filter(|c| installed.is_none_or(|i| c.version > i.version))
            .collect();
        let is_request = plugin == &self.request.plugin;
        let mut first_problem = None;
        for candidate in &fitting {
            let problem = self.refusal(id, plugin, candidate, is_request);
            if let Some(problem) = problem {
                first_problem.get_or_insert(problem);
                continue;
            }
            let candidate = Box::new((*candidate).clone());
            let action = match installed {
                Some(installed) if !self.request.allow_updates && !is_request => {
                    return Err(ResolveError::UpdateNeeded {
                        id: plugin.clone(),
                        installed: installed.version.clone(),
                        constraints,
                    });
                }
                Some(installed) => StepAction::Update {
                    from: installed.version.clone(),
                    candidate,
                },
                None if candidate.bundled => StepAction::InstallBundled { candidate },
                None => StepAction::Install { candidate },
            };
            return Ok((
                installed.map_or_else(|| candidate_name(&action), |i| i.name.clone()),
                action,
            ));
        }
        if let Some(problem) = first_problem {
            return Err(problem);
        }
        Err(ResolveError::Conflict {
            id: id.clone(),
            constraints,
            available: sorted.iter().map(|c| c.version.clone()).collect(),
        })
    }

    /// Why `candidate` cannot be chosen: withdrawn, incompatible with
    /// this DeckPilot, or (for a dependency) from an unverified publisher.
    fn refusal(
        &self,
        id: &DependencyTarget,
        plugin: &PluginId,
        candidate: &Candidate,
        is_request: bool,
    ) -> Option<ResolveError> {
        if let Some(reason) = &candidate.withdrawn {
            return Some(ResolveError::Withdrawn {
                id: id.clone(),
                version: candidate.version.clone(),
                reason: reason.clone(),
            });
        }
        let reasons = candidate.incompatibilities(&self.universe.host);
        if !reasons.is_empty() {
            return Some(ResolveError::Incompatible {
                id: id.clone(),
                version: candidate.version.clone(),
                reasons,
            });
        }
        (!is_request && !candidate.publisher_verified && !candidate.bundled)
            .then(|| ResolveError::UnverifiedPublisher { id: plugin.clone() })
    }

    fn select_builtin(
        &self,
        id: &DependencyTarget,
        name: &str,
        constraints: Vec<Constraint>,
    ) -> Result<(String, StepAction), ResolveError> {
        let fits = |version: &Version| constraints.iter().all(|c| c.accepts(version, None));
        let Some(module) = self.universe.builtins.get(name) else {
            return Err(ResolveError::Missing {
                id: id.clone(),
                required_by: constraints,
            });
        };
        if !module.supported {
            return Err(ResolveError::Unavailable {
                id: id.clone(),
                reason: "the built-in module is not supported on this computer".into(),
            });
        }
        if !fits(&module.version) {
            return Err(ResolveError::Conflict {
                id: id.clone(),
                constraints,
                available: vec![module.version.clone()],
            });
        }
        Ok((
            module.name.clone(),
            StepAction::Builtin {
                version: module.version.clone(),
                enabled: module.enabled,
            },
        ))
    }

    fn plan(&self, root: &DependencyTarget) -> Plan {
        let steps: Vec<Step> = self
            .order
            .iter()
            .filter_map(|id| {
                let (name, action) = self.chosen.get(id.as_str())?;
                Some(Step {
                    id: id.clone(),
                    name: name.clone(),
                    action: action.clone(),
                    requested: id == root,
                    required_by: self.active_constraints(id),
                    optional: self.optional_only.contains(id.as_str()),
                })
            })
            .collect();
        let mut optional = Vec::new();
        for step in &steps {
            let Some(of) = step.id.plugin() else {
                continue;
            };
            let manifest_actions = self.action_dependencies(step);
            for dependency in step.action.dependencies(self.universe, &step.id) {
                if !dependency.is_optional() || self.chosen.contains_key(dependency.id.as_str()) {
                    continue;
                }
                optional.push(OptionalIntegration {
                    of: of.clone(),
                    dependency: dependency.clone(),
                    installed: dependency
                        .id
                        .plugin()
                        .and_then(|p| self.universe.installed.get(p))
                        .map(|p| p.version.clone()),
                    actions: manifest_actions
                        .iter()
                        .filter(|(_, needs)| needs.iter().any(|n| n == dependency.id.as_str()))
                        .map(|(action, _)| action.clone())
                        .collect(),
                });
            }
        }
        Plan {
            plugin: self.request.plugin.clone(),
            steps,
            optional,
        }
    }

    /// The constraints that hold for the plan as chosen: the request's,
    /// those of chosen versions and of installed plugins left as they are.
    fn active_constraints(&self, id: &DependencyTarget) -> Vec<Constraint> {
        self.constraints_on(id)
            .into_iter()
            .filter(|c| match (&c.from, &c.from_version) {
                (Some(from), Some(version)) => match self.chosen.get(from.as_str()) {
                    Some((_, action)) => action.version() == version,
                    None => self
                        .universe
                        .installed
                        .get(from)
                        .is_some_and(|p| &p.version == version),
                },
                _ => true,
            })
            .collect()
    }

    fn action_dependencies(&self, step: &Step) -> Vec<(String, Vec<String>)> {
        // Candidates carry no actions (the registry detail lists none):
        // the integration's actions are known for installed plugins, and
        // from the verified manifest once downloaded.
        step.id
            .plugin()
            .and_then(|p| self.universe.installed.get(p))
            .filter(|p| &p.version == step.action.version())
            .map(|p| p.action_dependencies.clone())
            .unwrap_or_default()
    }
}

fn candidate_name(action: &StepAction) -> String {
    action
        .candidate()
        .map(|c| c.name.clone())
        .unwrap_or_default()
}

// ------------------------------------------------------------------ status

/// The state of one declared dependency on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyState {
    /// Installed (or built in), enabled, in a version that fits.
    Satisfied { version: Version },
    /// Installed in a version that fits, but not enabled.
    Disabled { version: Version },
    /// Installed in a version outside the range.
    WrongVersion { version: Version },
    /// Not installed (or no such built-in module).
    Missing,
    /// Installed but broken, quarantined or busy.
    Unusable { reason: String },
}

impl DependencyState {
    pub fn is_satisfied(&self) -> bool {
        matches!(self, Self::Satisfied { .. })
    }
}

/// One declared dependency and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyStatus {
    pub dependency: Dependency,
    pub state: DependencyState,
    pub builtin: bool,
    /// The name of what satisfies it, when known.
    pub name: Option<String>,
}

/// The state of each of `dependencies`.
pub fn dependency_status(
    dependencies: &[Dependency],
    installed: &BTreeMap<PluginId, InstalledPlugin>,
    builtins: &BTreeMap<String, BuiltinModule>,
) -> Vec<DependencyStatus> {
    dependencies
        .iter()
        .map(|dependency| {
            let (state, name, builtin) = match &dependency.id {
                DependencyTarget::Builtin(id) => match builtins.get(id) {
                    None => (DependencyState::Missing, None, true),
                    Some(module) => {
                        let state = if !dependency.version.matches(&module.version) {
                            DependencyState::WrongVersion {
                                version: module.version.clone(),
                            }
                        } else if !module.supported {
                            DependencyState::Unusable {
                                reason: "not supported on this computer".into(),
                            }
                        } else if module.enabled {
                            DependencyState::Satisfied {
                                version: module.version.clone(),
                            }
                        } else {
                            DependencyState::Disabled {
                                version: module.version.clone(),
                            }
                        };
                        (state, Some(module.name.clone()), true)
                    }
                },
                DependencyTarget::Plugin(id) => match installed.get(id) {
                    None => (DependencyState::Missing, None, false),
                    Some(plugin) => {
                        let version = plugin.version.clone();
                        let state = if dependency.version.matches(&version) {
                            match plugin.state {
                                PluginState::Enabled => DependencyState::Satisfied { version },
                                PluginState::Disabled => DependencyState::Disabled { version },
                                state => DependencyState::Unusable {
                                    reason: format!("it is {state}"),
                                },
                            }
                        } else {
                            DependencyState::WrongVersion { version }
                        };
                        (state, Some(plugin.name.clone()), plugin.bundled)
                    }
                },
            };
            DependencyStatus {
                dependency: dependency.clone(),
                state,
                builtin,
                name,
            }
        })
        .collect()
}

/// Why a plugin's actions cannot run for lack of a required dependency,
/// worded for the user; `None` when every required one is satisfied.
pub fn missing_required(status: &[DependencyStatus]) -> Option<String> {
    status
        .iter()
        .find(|s| s.dependency.is_required() && !s.state.is_satisfied())
        .map(|s| unmet(s, "requires"))
}

/// Why `action` cannot run for lack of a dependency it names in
/// `requiresDependencies`; `None` when they are all satisfied.
pub fn action_unmet(requires: &[String], status: &[DependencyStatus]) -> Option<String> {
    requires.iter().find_map(|id| {
        status
            .iter()
            .find(|s| s.dependency.id.as_str() == id && !s.state.is_satisfied())
            .map(|s| unmet(s, "needs"))
    })
}

fn unmet(status: &DependencyStatus, verb: &str) -> String {
    let id = &status.dependency.id;
    let range = status.dependency.version.as_str();
    let kind = if status.builtin {
        "the built-in module"
    } else {
        "the plugin"
    };
    match &status.state {
        DependencyState::Satisfied { .. } => String::new(),
        DependencyState::Disabled { .. } => format!("it {verb} {kind} {id}, which is disabled"),
        DependencyState::WrongVersion { version } => {
            format!("it {verb} {kind} {id} {range}; {version} is installed")
        }
        DependencyState::Missing => {
            format!("it {verb} {kind} {id} {range}, which is not installed")
        }
        DependencyState::Unusable { reason } => {
            format!("it {verb} {kind} {id}, which cannot be used ({reason})")
        }
    }
}

// ----------------------------------------------------------------- removal

/// What is being removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    Disable,
    Uninstall,
}

/// A plugin affected by a removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependent {
    pub id: PluginId,
    pub name: String,
    pub enabled: bool,
    pub range: VersionRange,
    /// The dependency is optional for it: only `actions` stop.
    pub optional: bool,
    pub actions: Vec<String>,
}

/// What disabling or uninstalling a plugin (or disabling a built-in
/// module) affects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalPlan {
    pub target: String,
    pub removal: Removal,
    /// Installed plugins that require it.
    pub required_by: Vec<Dependent>,
    /// Installed plugins whose optional actions use it.
    pub used_by: Vec<Dependent>,
    /// Plugins installed only as dependencies that nothing else would
    /// need after an uninstall (transitively). Offered, never removed
    /// without the user's confirmation.
    pub orphans: Vec<PluginId>,
}

impl RemovalPlan {
    /// Enabled plugins that require the target: the removal is refused
    /// unless they are disabled with it.
    pub fn blocking(&self) -> impl Iterator<Item = &Dependent> {
        self.required_by.iter().filter(|d| d.enabled)
    }

    pub fn is_blocked(&self) -> bool {
        self.blocking().next().is_some()
    }
}

/// The installed plugins that declare a dependency on `target`.
pub fn dependents(
    installed: &BTreeMap<PluginId, InstalledPlugin>,
    target: &str,
) -> (Vec<Dependent>, Vec<Dependent>) {
    let mut required = Vec::new();
    let mut optional = Vec::new();
    for plugin in installed.values() {
        if plugin.id.as_str() == target {
            continue;
        }
        let Some(dependency) = plugin.dependencies.iter().find(|d| d.id.as_str() == target) else {
            continue;
        };
        let dependent = Dependent {
            id: plugin.id.clone(),
            name: plugin.name.clone(),
            enabled: plugin.is_enabled(),
            range: dependency.version.clone(),
            optional: dependency.is_optional(),
            actions: plugin
                .action_dependencies
                .iter()
                .filter(|(_, needs)| needs.iter().any(|n| n == target))
                .map(|(action, _)| action.clone())
                .collect(),
        };
        if dependency.is_optional() {
            optional.push(dependent);
        } else {
            required.push(dependent);
        }
    }
    (required, optional)
}

/// What removing `target` affects; see [`RemovalPlan`].
pub fn plan_removal(
    installed: &BTreeMap<PluginId, InstalledPlugin>,
    target: &str,
    removal: Removal,
) -> RemovalPlan {
    let (required_by, used_by) = dependents(installed, target);
    let orphans = match removal {
        Removal::Disable => Vec::new(),
        Removal::Uninstall => orphans_after(installed, &[target]),
    };
    RemovalPlan {
        target: target.to_owned(),
        removal,
        required_by,
        used_by,
        orphans,
    }
}

/// The plugins installed only as dependencies that no remaining plugin
/// declares a dependency on (required or optional) once `removed` are
/// gone, transitively, in removal order (dependents before their
/// dependencies). With nothing removed: today's orphans.
pub fn orphans_after(
    installed: &BTreeMap<PluginId, InstalledPlugin>,
    removed: &[&str],
) -> Vec<PluginId> {
    let mut gone: BTreeSet<String> = removed.iter().map(|s| (*s).to_owned()).collect();
    let mut orphans = Vec::new();
    loop {
        let next: Vec<PluginId> = installed
            .values()
            .filter(|p| p.as_dependency && !gone.contains(p.id.as_str()))
            .filter(|p| {
                !installed.values().any(|other| {
                    !gone.contains(other.id.as_str())
                        && other.id != p.id
                        && other
                            .dependencies
                            .iter()
                            .any(|d| d.id.as_str() == p.id.as_str())
                })
            })
            .map(|p| p.id.clone())
            .collect();
        if next.is_empty() {
            return orphans;
        }
        for id in next {
            gone.insert(id.as_str().to_owned());
            orphans.push(id);
        }
    }
}
