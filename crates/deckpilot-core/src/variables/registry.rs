//! The unified variable registry: every read-only variable DeckPilot
//! provides, who owns it and how it is resolved (see `docs/variables.md`).
//!
//! Three kinds of owners exist:
//!
//! - **Core** owns the genuine system variables: `system.profile.…`,
//!   `system.device.current.id`, `system.platform` and `workflow.…`. They
//!   are registered by [`VariableRegistry::new`].
//! - **Modules** declare their variables in their manifest
//!   ([`crate::ModuleManifest::variables`]) and resolve them with a
//!   [`VariableSource`]. A module may only declare inside its own namespace
//!   ([`module_namespace`]): `projects.current.id` belongs to the `projects`
//!   module, `workSession.state` to `work-session`. Registration and
//!   unregistration follow the module's lifecycle in
//!   [`crate::ModuleRegistry`].
//! - **Users** own the shared variables (`shared.<key>`), which live in the
//!   configuration database and are not part of this registry.
//!
//! Older ids stay readable through explicit aliases ([`ALIASES`]): for
//! example `system.project.current.id` resolves as `projects.current.id`.
//! Stored references are never rewritten; the picker inserts canonical ids.
//!
//! The registry never stores a value. Every resolution asks the owner's
//! source again, so a module value can never be cached or persisted as a
//! user value.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use deckpilot_device::DeviceId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::catalog::{CatalogEntry, ReferenceAvailability, VariableGroup};
use super::resolution::{
    MissingReason, ProjectSource, Resolution, ResolutionContext, ResolutionPhase,
};
use super::{SystemVariable, VariableRef, VariableType, valid_segment};
use crate::work_session::WorkSessionSnapshot;
use crate::{ModuleId, Os, Profile, Project};

/// Roots of the reference grammar that no module can use as its namespace:
/// node outputs, shared variables and the two namespaces of the core.
pub const RESERVED_ROOTS: &[&str] = &["steps", "shared", "system", "workflow"];
/// The namespaces the core owns.
pub const CORE_NAMESPACES: &[&str] = &["system", "workflow"];
/// Most segments of a [`VariableId`].
pub const MAX_ID_SEGMENTS: usize = 6;

/// The canonical id of a registered variable, as written in a reference
/// without braces: `projects.current.id`, `system.platform`. The first
/// segment is its owner's namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VariableId(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "`{0}` is not a variable id: use namespace.name with letters, digits and `_`, \
     each part starting with a lowercase letter"
)]
pub struct InvalidVariableId(pub String);

impl VariableId {
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidVariableId> {
        let id = id.into();
        let segments: Vec<&str> = id.split('.').collect();
        if !(2..=MAX_ID_SEGMENTS).contains(&segments.len())
            || !segments.iter().all(|segment| valid_segment(segment))
        {
            return Err(InvalidVariableId(id));
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first segment: the owner's namespace.
    pub fn namespace(&self) -> &str {
        self.0.split('.').next().unwrap_or_default()
    }

    /// Everything after the namespace, e.g. `current.id`.
    pub fn name(&self) -> &str {
        self.0.split_once('.').map_or("", |(_, name)| name)
    }

    /// Whether the id is in a namespace of the core.
    pub fn is_core(&self) -> bool {
        CORE_NAMESPACES.contains(&self.namespace())
    }

    /// The id of a variable in `namespace`.
    ///
    /// # Panics
    ///
    /// If the result is not a valid id: a mistake in a constant.
    pub fn of(namespace: &str, name: &str) -> Self {
        Self::new(format!("{namespace}.{name}")).expect("a valid variable id")
    }
}

impl TryFrom<String> for VariableId {
    type Error = InvalidVariableId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<VariableId> for String {
    fn from(value: VariableId) -> Self {
        value.0
    }
}

impl fmt::Display for VariableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The namespace of a module's variables: its id in lower camel case, so
/// that it is a valid reference segment (`work-session` → `workSession`).
pub fn module_namespace(module: &ModuleId) -> String {
    let mut namespace = String::with_capacity(module.as_str().len());
    let mut upper = false;
    for c in module.as_str().chars() {
        if c == '-' {
            upper = true;
        } else if upper {
            namespace.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            namespace.push(c);
        }
    }
    namespace
}

/// Who owns a variable, and so who may declare, change or remove it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VariableOwner {
    /// DeckPilot itself: genuine system variables.
    Core,
    /// A feature module, inside its own namespace.
    Module { module: ModuleId },
    /// The user: shared variables.
    User,
}

impl VariableOwner {
    pub fn module(&self) -> Option<&ModuleId> {
        match self {
            Self::Module { module } => Some(module),
            Self::Core | Self::User => None,
        }
    }
}

/// What the value of a declared variable depends on: the scope the
/// variables page filters by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeclaredScope {
    /// This computer, the same everywhere (`system.platform`).
    Machine,
    /// The profile in the context.
    Profile,
    /// The project in the context.
    Project,
    /// The device whose control started the action.
    Device,
    /// The running workflow.
    Workflow,
    /// One run.
    Execution,
    /// The active work session.
    WorkSession,
}

impl DeclaredScope {
    pub fn name(self) -> &'static str {
        match self {
            Self::Machine => "machine",
            Self::Profile => "profile",
            Self::Project => "project",
            Self::Device => "device",
            Self::Workflow => "workflow",
            Self::Execution => "execution",
            Self::WorkSession => "workSession",
        }
    }
}

/// What must be in the context for a declared variable to have a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Requirement {
    /// A project (the active one, or the run's).
    Project,
    /// The device whose control started the action.
    Device,
    /// A workflow.
    Workflow,
    /// A running workflow: no value in previews.
    Execution,
    /// An active work session.
    WorkSession,
}

/// A read-only variable a module or the core declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableDeclaration {
    /// Canonical id, inside the owner's namespace.
    pub id: VariableId,
    /// Short name shown in the catalog and the picker.
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub value_type: VariableType,
    pub scope: DeclaredScope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
    /// The value is never shown: previews, logs, history and exports see it
    /// redacted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// Core and module variables are always read-only; a declaration that
    /// says otherwise is refused.
    #[serde(default = "read_only")]
    pub read_only: bool,
}

fn read_only() -> bool {
    true
}

impl VariableDeclaration {
    /// A read-only, non-sensitive declaration.
    ///
    /// # Panics
    ///
    /// If `id` is not a valid id: a mistake in a constant.
    pub fn new(id: &str, name: &str, value_type: VariableType, scope: DeclaredScope) -> Self {
        Self {
            id: VariableId::new(id).expect("a valid variable id"),
            name: name.to_owned(),
            description: String::new(),
            value_type,
            scope,
            requires: Vec::new(),
            sensitive: false,
            read_only: true,
        }
    }

    #[must_use]
    pub fn describe(mut self, description: &str) -> Self {
        description.clone_into(&mut self.description);
        self
    }

    #[must_use]
    pub fn requires(mut self, requirement: Requirement) -> Self {
        if !self.requires.contains(&requirement) {
            self.requires.push(requirement);
        }
        self
    }
}

/// Deprecation metadata of an alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deprecation {
    /// The id to use instead.
    pub canonical: VariableId,
    /// The version in which the alias was deprecated.
    pub since: String,
}

/// An older id that keeps resolving as a canonical one. Stored references
/// with the old spelling are read, shown and saved as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariableAlias {
    pub alias: SystemVariable,
    /// The canonical id.
    pub canonical: &'static str,
    /// The module that owns the canonical id.
    pub owner: &'static str,
    /// Version in which the alias was deprecated.
    pub since: &'static str,
}

/// Every alias. Project and work session values used to be core system
/// variables; they belong to the `projects` and `work-session` modules.
pub const ALIASES: [VariableAlias; 9] = {
    const fn alias(
        alias: SystemVariable,
        canonical: &'static str,
        owner: &'static str,
    ) -> VariableAlias {
        VariableAlias {
            alias,
            canonical,
            owner,
            since: "0.6.0",
        }
    }
    use SystemVariable as S;
    [
        alias(S::ProjectCurrentId, "projects.current.id", "projects"),
        alias(S::ProjectCurrentKey, "projects.current.key", "projects"),
        alias(S::ProjectCurrentName, "projects.current.name", "projects"),
        alias(S::WorkSessionId, "workSession.id", "work-session"),
        alias(S::WorkSessionState, "workSession.state", "work-session"),
        alias(
            S::WorkSessionStartedAt,
            "workSession.startedAt",
            "work-session",
        ),
        alias(
            S::WorkSessionElapsedSeconds,
            "workSession.elapsedSeconds",
            "work-session",
        ),
        alias(
            S::WorkSessionProjectId,
            "workSession.projectId",
            "work-session",
        ),
        alias(
            S::WorkSessionCanFinish,
            "workSession.canFinish",
            "work-session",
        ),
    ]
};

impl SystemVariable {
    /// The alias entry of a deprecated id, `None` for a core variable.
    pub fn alias(self) -> Option<&'static VariableAlias> {
        ALIASES.iter().find(|alias| alias.alias == self)
    }

    /// The canonical id this resolves as: its own path for a core variable,
    /// the module-owned id for an alias.
    ///
    /// # Panics
    ///
    /// Never: every path and alias target is a valid id (tested).
    pub fn canonical(self) -> VariableId {
        VariableId::new(self.alias().map_or(self.path(), |alias| alias.canonical))
            .expect("built-in ids are valid")
    }

    /// Deprecation metadata, for an alias.
    pub fn deprecation(self) -> Option<Deprecation> {
        self.alias().map(|alias| Deprecation {
            canonical: self.canonical(),
            since: alias.since.to_owned(),
        })
    }
}

/// Values that change while the agent runs and are not part of the
/// resolution context. The agent supplies them; everything else uses
/// [`NoLiveState`].
pub trait LiveState: Send + Sync {
    /// The work session now, if the agent has one.
    fn work_session(&self) -> Option<WorkSessionSnapshot> {
        None
    }

    /// Wall clock, milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// No live state: no work session.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLiveState;

impl LiveState for NoLiveState {}

/// The device in the context, as variables see it.
///
/// This is the device slot of the resolver, kept small and generic on
/// purpose. Today it is implemented by [`DeviceId`] (`model#serial`). The
/// generic `deckpilot_device::DeviceDescriptor` of the device refactoring
/// (vendor, model, display name, serial, capabilities, driver) will
/// implement it as well once it lands, so `system.device.current.id` and
/// later device variables can read the descriptor without any variable
/// source changing.
pub trait ContextDevice: Send + Sync {
    /// Stable id, the value of `system.device.current.id`.
    fn device_id(&self) -> String;

    /// A name for people, if the device has one.
    fn display_name(&self) -> Option<String> {
        None
    }
}

impl ContextDevice for DeviceId {
    fn device_id(&self) -> String {
        self.to_string()
    }
}

/// Everything a variable is resolved from: the explicit context, the data
/// it names, and live state.
#[derive(Clone, Copy)]
pub struct VariableEnvironment<'a> {
    pub context: &'a ResolutionContext,
    /// The profile the context names.
    pub profile: &'a Profile,
    pub os: Os,
    pub live: &'a dyn LiveState,
}

impl fmt::Debug for VariableEnvironment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VariableEnvironment")
            .field("context", self.context)
            .field("profile", &self.profile.id)
            .field("os", &self.os)
            .finish_non_exhaustive()
    }
}

impl<'a> VariableEnvironment<'a> {
    /// The project in the context. In a preview of a run's project it is
    /// unavailable until the run: the active project never stands in for
    /// it.
    pub fn project(&self) -> Result<&'a Project, Resolution> {
        let context = self.context;
        if context.phase == ResolutionPhase::Preview
            && context.project_source == ProjectSource::Execution
        {
            return Err(Resolution::UnavailableUntilRuntime);
        }
        let id = context
            .project
            .ok_or(Resolution::Missing(MissingReason::NoProject))?;
        self.profile
            .project(id)
            .ok_or(Resolution::Missing(MissingReason::OtherProfile))
    }

    /// The device in the context (the device slot, see
    /// [`ContextDevice`]).
    pub fn device(&self) -> Option<&'a dyn ContextDevice> {
        self.context
            .device
            .as_ref()
            .map(|device| device as &dyn ContextDevice)
    }
}

/// Resolves the variables of one owner. Called for every resolution; never
/// cached by the registry.
pub trait VariableSource: Send + Sync {
    /// The value of `declaration` in `environment`. The registry checks the
    /// type and applies the declared sensitivity afterwards.
    fn resolve(
        &self,
        declaration: &VariableDeclaration,
        environment: &VariableEnvironment<'_>,
    ) -> Resolution;
}

/// Why a declaration was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VariableRegistryError {
    #[error("variable `{id}` is outside the namespace `{namespace}` of module `{module}`")]
    Namespace {
        module: ModuleId,
        namespace: String,
        id: VariableId,
    },
    #[error("module `{module}` cannot declare variables: `{namespace}` is reserved")]
    ReservedNamespace { module: ModuleId, namespace: String },
    #[error(
        "the variable namespace `{namespace}` of module `{module}` is already owned by `{owner}`"
    )]
    NamespaceTaken {
        module: ModuleId,
        namespace: String,
        owner: ModuleId,
    },
    #[error("variable `{0}` is declared more than once")]
    Duplicate(VariableId),
    #[error("variable `{0}` must be read-only")]
    Writable(VariableId),
    #[error("module `{0}` declares variables but no source resolves them")]
    NoSource(ModuleId),
}

#[derive(Clone)]
struct Registered {
    declaration: VariableDeclaration,
    owner: VariableOwner,
}

#[derive(Clone)]
struct ModuleEntry {
    namespace: String,
    source: Arc<dyn VariableSource>,
    /// `None` while the module can run; else why not (shown to users).
    unavailable: Option<String>,
}

/// One registered variable, as listed.
#[derive(Debug, Clone, Copy)]
pub struct RegisteredVariable<'a> {
    pub declaration: &'a VariableDeclaration,
    pub owner: &'a VariableOwner,
    /// Why its owner cannot provide it now, if it cannot.
    pub unavailable: Option<&'a str>,
}

/// The core's and the modules' variables, by canonical id.
#[derive(Clone)]
pub struct VariableRegistry {
    variables: BTreeMap<VariableId, Registered>,
    modules: BTreeMap<ModuleId, ModuleEntry>,
    core: Arc<dyn VariableSource>,
}

impl fmt::Debug for VariableRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VariableRegistry")
            .field("variables", &self.variables.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Default for VariableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl VariableRegistry {
    /// A registry with the core's variables and no module.
    pub fn new() -> Self {
        let variables = core_declarations()
            .into_iter()
            .map(|declaration| {
                (
                    declaration.id.clone(),
                    Registered {
                        declaration,
                        owner: VariableOwner::Core,
                    },
                )
            })
            .collect();
        Self {
            variables,
            modules: BTreeMap::new(),
            core: Arc::new(CoreVariables),
        }
    }

    /// Registers the variables of `module`, all or none. A module may only
    /// declare inside [`module_namespace`] of its id; a namespace has one
    /// owner. Registering a module again replaces its declarations.
    pub fn register_module(
        &mut self,
        module: &ModuleId,
        declarations: &[VariableDeclaration],
        source: Option<Arc<dyn VariableSource>>,
    ) -> Result<(), VariableRegistryError> {
        let namespace = module_namespace(module);
        if declarations.is_empty() {
            self.unregister_module(module);
            return Ok(());
        }
        if RESERVED_ROOTS.contains(&namespace.as_str()) {
            return Err(VariableRegistryError::ReservedNamespace {
                module: module.clone(),
                namespace,
            });
        }
        if let Some((owner, _)) = self
            .modules
            .iter()
            .find(|(other, entry)| *other != module && entry.namespace == namespace)
        {
            return Err(VariableRegistryError::NamespaceTaken {
                module: module.clone(),
                namespace,
                owner: owner.clone(),
            });
        }
        let source = source.ok_or_else(|| VariableRegistryError::NoSource(module.clone()))?;
        let mut seen = std::collections::BTreeSet::new();
        for declaration in declarations {
            if declaration.id.namespace() != namespace {
                return Err(VariableRegistryError::Namespace {
                    module: module.clone(),
                    namespace,
                    id: declaration.id.clone(),
                });
            }
            if !seen.insert(&declaration.id) {
                return Err(VariableRegistryError::Duplicate(declaration.id.clone()));
            }
            if !declaration.read_only {
                return Err(VariableRegistryError::Writable(declaration.id.clone()));
            }
        }
        let unavailable = self.unregister_module(module);
        let owner = VariableOwner::Module {
            module: module.clone(),
        };
        for declaration in declarations {
            self.variables.insert(
                declaration.id.clone(),
                Registered {
                    declaration: declaration.clone(),
                    owner: owner.clone(),
                },
            );
        }
        self.modules.insert(
            module.clone(),
            ModuleEntry {
                namespace,
                source,
                unavailable: unavailable.flatten(),
            },
        );
        Ok(())
    }

    /// Removes the variables of `module`. References to them stay as they
    /// are and resolve as unavailable. Returns whether the module was
    /// registered, with its availability then.
    pub fn unregister_module(&mut self, module: &ModuleId) -> Option<Option<String>> {
        let entry = self.modules.remove(module)?;
        self.variables
            .retain(|_, registered| registered.owner.module() != Some(module));
        Some(entry.unavailable)
    }

    /// Marks the variables of `module` as available or not, e.g. while the
    /// module is disabled. `reason` is shown to users.
    pub fn set_available(&mut self, module: &ModuleId, reason: Option<String>) {
        if let Some(entry) = self.modules.get_mut(module) {
            entry.unavailable = reason;
        }
    }

    /// Whether `module` has registered variables.
    pub fn has_module(&self, module: &ModuleId) -> bool {
        self.modules.contains_key(module)
    }

    /// The owner of `namespace`, if any.
    pub fn namespace_owner(&self, namespace: &str) -> Option<VariableOwner> {
        if CORE_NAMESPACES.contains(&namespace) {
            return Some(VariableOwner::Core);
        }
        self.modules
            .iter()
            .find(|(_, entry)| entry.namespace == namespace)
            .map(|(module, _)| VariableOwner::Module {
                module: module.clone(),
            })
    }

    /// The registered variable `id`.
    pub fn get(&self, id: &VariableId) -> Option<RegisteredVariable<'_>> {
        let registered = self.variables.get(id)?;
        Some(self.view(registered))
    }

    /// Every registered variable, by id.
    pub fn variables(&self) -> impl Iterator<Item = RegisteredVariable<'_>> {
        self.variables
            .values()
            .map(|registered| self.view(registered))
    }

    fn view<'a>(&'a self, registered: &'a Registered) -> RegisteredVariable<'a> {
        let unavailable = registered
            .owner
            .module()
            .and_then(|module| self.modules.get(module))
            .and_then(|entry| entry.unavailable.as_deref());
        RegisteredVariable {
            declaration: &registered.declaration,
            owner: &registered.owner,
            unavailable,
        }
    }

    /// The value of `id` now. A variable of a module that is disabled,
    /// unavailable or not registered at all is [`Resolution::Unavailable`];
    /// an unknown id in a known namespace is missing.
    pub fn resolve(&self, id: &VariableId, environment: &VariableEnvironment<'_>) -> Resolution {
        if environment.context.profile != environment.profile.id {
            return Resolution::Missing(MissingReason::OtherProfile);
        }
        let Some(registered) = self.variables.get(id) else {
            return match self.namespace_owner(id.namespace()) {
                Some(_) => Resolution::Missing(MissingReason::UnknownVariable),
                // Nothing owns the namespace: its module is not installed.
                None => Resolution::Unavailable {
                    owner: id.namespace().to_owned(),
                },
            };
        };
        let source = match registered.owner.module() {
            None => &self.core,
            Some(module) => match self.modules.get(module) {
                Some(entry) if entry.unavailable.is_none() => &entry.source,
                _ => {
                    return Resolution::Unavailable {
                        owner: module.to_string(),
                    };
                }
            },
        };
        let declaration = &registered.declaration;
        match source.resolve(declaration, environment) {
            Resolution::Value {
                value,
                source,
                sensitive,
            } => {
                if value.is_of(declaration.value_type) {
                    Resolution::Value {
                        value,
                        source,
                        sensitive: sensitive || declaration.sensitive,
                    }
                } else {
                    // A source that breaks its declaration provides nothing
                    // rather than a value of the wrong type.
                    Resolution::Missing(MissingReason::NoValue)
                }
            }
            other => other,
        }
    }
}

impl VariableRegistry {
    /// Catalog entries of every core variable, every alias (flagged
    /// deprecated) and every module variable in `context`: metadata only,
    /// never a value. `work_session`: whether a work session is active now.
    /// Core variables and aliases come first, in the order older clients
    /// listed them.
    ///
    /// # Panics
    ///
    /// Never: alias owners are valid module ids (tested).
    pub fn catalog(&self, context: &ResolutionContext, work_session: bool) -> Vec<CatalogEntry> {
        let mut entries: Vec<CatalogEntry> = SystemVariable::ALL
            .into_iter()
            .map(|variable| {
                let canonical = variable.canonical();
                let registered = self.get(&canonical);
                // Core variables are always registered; an alias whose
                // module is not installed still names its owner.
                let owner = registered.map_or_else(
                    || VariableOwner::Module {
                        module: ModuleId::new(variable.alias().map_or("", |a| a.owner))
                            .expect("alias owners are valid module ids"),
                    },
                    |r| r.owner.clone(),
                );
                CatalogEntry {
                    reference: VariableRef::System(variable),
                    label: registered.map_or_else(
                        || variable.path().to_owned(),
                        |r| r.declaration.name.clone(),
                    ),
                    description: variable.description().to_owned(),
                    group: if variable.path().starts_with("workflow.") {
                        VariableGroup::Workflow
                    } else {
                        VariableGroup::System
                    },
                    value_type: variable.value_type(),
                    scope: None,
                    module: owner.module().cloned(),
                    sensitive: registered.is_some_and(|r| r.declaration.sensitive),
                    dynamic: variable.dynamic(),
                    availability: match registered {
                        Some(registered) => availability(registered, context, work_session),
                        None => ReferenceAvailability::Unavailable,
                    },
                    owner: Some(owner),
                    read_only: true,
                    deprecated: variable.deprecation(),
                    declared_scope: registered.map(|r| r.declaration.scope),
                }
            })
            .collect();
        for registered in self.variables() {
            let Some(module) = registered.owner.module() else {
                continue;
            };
            let declaration = registered.declaration;
            entries.push(CatalogEntry {
                reference: VariableRef::Module(declaration.id.clone()),
                label: declaration.name.clone(),
                description: declaration.description.clone(),
                group: VariableGroup::Module,
                value_type: declaration.value_type,
                scope: None,
                module: Some(module.clone()),
                sensitive: declaration.sensitive,
                dynamic: declaration.scope != DeclaredScope::Machine,
                availability: availability(registered, context, work_session),
                owner: Some(registered.owner.clone()),
                read_only: true,
                deprecated: None,
                declared_scope: Some(declaration.scope),
            });
        }
        entries
    }
}

/// Whether a registered variable can have a value in `context`.
fn availability(
    registered: RegisteredVariable<'_>,
    context: &ResolutionContext,
    work_session: bool,
) -> ReferenceAvailability {
    if registered.unavailable.is_some() {
        return ReferenceAvailability::Unavailable;
    }
    let requires = &registered.declaration.requires;
    let preview = context.phase == ResolutionPhase::Preview;
    for requirement in requires {
        let available = match requirement {
            Requirement::Execution => return ReferenceAvailability::RuntimeOnly,
            // Listed without a session too: a workflow that reads them
            // runs during one.
            Requirement::WorkSession if !work_session => {
                return ReferenceAvailability::RuntimeOnly;
            }
            Requirement::WorkSession => true,
            Requirement::Project
                if preview && context.project_source == ProjectSource::Execution =>
            {
                return ReferenceAvailability::RuntimeOnly;
            }
            Requirement::Project => context.project.is_some(),
            Requirement::Device => context.device.is_some(),
            Requirement::Workflow => context.workflow.is_some(),
        };
        if !available {
            return ReferenceAvailability::NotInContext;
        }
    }
    ReferenceAvailability::Available
}

/// The variables the core owns, in a stable order.
pub fn core_declarations() -> Vec<VariableDeclaration> {
    SystemVariable::ALL
        .into_iter()
        .filter(|variable| variable.alias().is_none())
        .map(|variable| {
            let (name, scope, requires) = match variable {
                SystemVariable::ProfileCurrentId => ("Profile id", DeclaredScope::Profile, None),
                SystemVariable::ProfileCurrentName => {
                    ("Profile name", DeclaredScope::Profile, None)
                }
                SystemVariable::DeviceCurrentId => {
                    ("Device", DeclaredScope::Device, Some(Requirement::Device))
                }
                SystemVariable::Platform => ("Platform", DeclaredScope::Machine, None),
                SystemVariable::WorkflowId => (
                    "Workflow id",
                    DeclaredScope::Workflow,
                    Some(Requirement::Workflow),
                ),
                _ => (
                    "Run id",
                    DeclaredScope::Execution,
                    Some(Requirement::Execution),
                ),
            };
            let declaration =
                VariableDeclaration::new(variable.path(), name, variable.value_type(), scope)
                    .describe(variable.description());
            match requires {
                Some(requirement) => declaration.requires(requirement),
                None => declaration,
            }
        })
        .collect()
}

/// Resolves the core's variables from the context.
struct CoreVariables;

impl VariableSource for CoreVariables {
    fn resolve(
        &self,
        declaration: &VariableDeclaration,
        environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        let variable = SystemVariable::ALL
            .into_iter()
            .find(|variable| variable.path() == declaration.id.as_str());
        match variable {
            // The device slot: whatever describes the device in the context.
            Some(SystemVariable::DeviceCurrentId) => match environment.device() {
                Some(device) => Resolution::Value {
                    value: super::VariableValue::String(device.device_id()),
                    source: super::ValueSource::System,
                    sensitive: false,
                },
                None => Resolution::Missing(MissingReason::NoDevice),
            },
            Some(variable) => crate::resolution::system_value(
                variable,
                environment.profile,
                environment.os,
                environment.context,
            ),
            None => Resolution::Missing(MissingReason::UnknownVariable),
        }
    }
}

#[cfg(test)]
mod tests;
