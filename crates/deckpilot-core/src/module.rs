//! Feature modules and the module registry.
//!
//! DeckPilot's core knows nothing about individual features. A feature
//! module (projects, utilities, ...) declares what it contributes in a
//! [`ModuleManifest`] and provides the [`ActionHandler`]s for its action
//! types. Modules are registered explicitly; there is no dynamic loading.
//!
//! Each module owns a namespace: every action type it provides must have an
//! identifier of the form `<module id>.<name>`. That makes the owner of any
//! stored action instance derivable from the instance alone, even when the
//! module is disabled or no longer compiled in.
//!
//! Disabling a module only removes its action types from the
//! [`ActionRegistry`] that executes actions. User configuration that refers to
//! the module is never touched; it is reported as unavailable instead.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ActionHandler, ActionParameters, ActionRegistry, ActionType, ActionTypeId, IdentifierError, Os,
    ParameterError, ParameterSpec,
};

/// Stable identifier of a feature module, e.g. `projects`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModuleId(String);

impl ModuleId {
    pub fn new(id: impl Into<String>) -> Result<Self, IdentifierError> {
        let id = id.into();
        let error = |reason| IdentifierError {
            kind: "module id",
            value: id.clone(),
            reason,
        };
        if id.is_empty() || id.len() > 64 {
            return Err(error("must be between 1 and 64 characters"));
        }
        if !id.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(error("must start with a lowercase ASCII letter"));
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(error(
                "only lowercase ASCII letters, digits and `-` are allowed",
            ));
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The module that owns an action type, derived from its namespace.
    pub fn owner_of(action_type: &ActionTypeId) -> Option<Self> {
        let (namespace, _) = action_type.as_str().split_once('.')?;
        Self::new(namespace).ok()
    }
}

impl TryFrom<String> for ModuleId {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ModuleId> for String {
    fn from(value: ModuleId) -> Self {
        value.0
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a module's code is allowed to do. Declared in the manifest, shown to
/// the user, and enforced where the core can check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Read the active profile's configuration.
    ProfileRead,
    /// Change the active project of the active profile.
    ProjectContext,
    /// Read this machine's resource bindings (paths, URLs, commands).
    MachineResources,
    /// Open applications, files, folders and web pages with the programs
    /// the operating system chooses for them.
    OpenExternal,
    /// Start programs and wait for them, e.g. a command, `git` or
    /// `docker`. Running user-chosen commands is additionally switched off
    /// per module until the user allows it on this computer.
    ProcessExecution,
    /// See which programs are running, without their command lines or
    /// environment.
    ProcessInspect,
    /// Send requests over the network.
    Network,
    /// Show desktop notifications.
    Notifications,
    /// Use credentials stored on this computer, by their identifier. The
    /// secret itself is never part of a workflow or an export.
    Credentials,
    /// Read shared and system variables while running. Values a module
    /// receives through its parameters need no capability: the user put the
    /// reference there.
    VariablesRead,
}

impl Capability {
    pub fn description(self) -> &'static str {
        match self {
            Self::ProfileRead => "Read the active profile",
            Self::ProjectContext => "Change the active project",
            Self::MachineResources => "Read this computer's resource bindings",
            Self::OpenExternal => "Open applications, files and web pages",
            Self::ProcessExecution => "Run programs on this computer",
            Self::ProcessInspect => "See which programs are running",
            Self::Network => "Send network requests",
            Self::Notifications => "Show desktop notifications",
            Self::Credentials => "Use credentials stored on this computer",
            Self::VariablesRead => "Read shared variables",
        }
    }
}

/// A navigation page contributed by a module. The UI maps `id` to a view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModulePage {
    /// Unique within the module, e.g. `projects`.
    pub id: String,
    pub title: String,
    /// Icon name from the UI icon set.
    pub icon: String,
    /// Position relative to other module pages (lower first).
    pub order: i32,
}

/// How a module setting may travel in a configuration export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingScope {
    /// Exported and imported.
    Portable,
    /// Only meaningful on this computer; never exported.
    MachineSpecific,
    /// Credentials or similar; never exported.
    Sensitive,
}

/// What a module contributes to a configuration export, versioned
/// independently of the module. Settings not listed are treated as
/// machine-specific and stay on this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Portability {
    /// Format version of the module's exported data. `0`: nothing exported.
    pub version: u32,
    pub settings: BTreeMap<String, SettingScope>,
}

impl Portability {
    pub fn scope(&self, key: &str) -> SettingScope {
        self.settings
            .get(key)
            .copied()
            .unwrap_or(SettingScope::MachineSpecific)
    }

    /// The portable subset of `config`.
    pub fn portable(&self, config: &ActionParameters) -> ActionParameters {
        let mut out = ActionParameters::new();
        for (key, value) in config.iter() {
            if self.scope(key) == SettingScope::Portable {
                out = out.with(key.clone(), value.clone());
            }
        }
        out
    }
}

/// Size behaviour of a native window. Windows size themselves to their
/// content within the limits of the mode and the screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowSize {
    /// Width and height follow the content.
    #[default]
    Fit,
    /// Small dialogs such as a password prompt.
    Compact,
    /// Regular dialogs.
    Medium,
    /// Logs and detailed information; resizable.
    Large,
    /// The whole screen; only when explicitly requested.
    Fullscreen,
}

/// A native window a module can show, identified by `<module id>.<name>`.
/// The view itself is part of the application bundle and registered under
/// the same identifier; only the identifier travels between processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleWindow {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub size: WindowSize,
    /// The window must be completed before anything else can be done;
    /// Escape and clicks outside do not close it.
    #[serde(default)]
    pub blocking: bool,
}

/// One step of a module's own database schema. The SQL is declared data:
/// the core applies it, so no module code runs during migrations, and the
/// schemas of disabled modules are kept up to date as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleMigration {
    /// 1-based, consecutive.
    pub version: u32,
    pub description: String,
    /// May be empty, e.g. to adopt tables created by an older core
    /// migration.
    pub sql: String,
}

/// Static description of a feature module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleManifest {
    pub id: ModuleId,
    pub version: String,
    pub name: String,
    pub description: String,
    /// Operating systems the module works on. Empty means all.
    pub platforms: Vec<Os>,
    pub pages: Vec<ModulePage>,
    pub capabilities: Vec<Capability>,
    /// Module-level settings, edited like action parameters.
    pub config: Vec<ParameterSpec>,
    /// Modules that must be enabled for this one to work.
    pub dependencies: Vec<ModuleId>,
    /// Whether a fresh installation enables the module.
    pub enabled_by_default: bool,
    /// What the module exports and imports.
    #[serde(default)]
    pub portability: Portability,
    /// The module's own database schema steps.
    #[serde(skip)]
    pub migrations: Vec<ModuleMigration>,
    /// Native windows the module can open.
    #[serde(default)]
    pub windows: Vec<ModuleWindow>,
}

impl ModuleManifest {
    pub fn supports(&self, os: Os) -> bool {
        self.platforms.is_empty() || self.platforms.contains(&os)
    }
}

/// A feature module compiled into DeckPilot.
pub trait FeatureModule: Send + Sync {
    fn manifest(&self) -> &ModuleManifest;

    /// Creates the handlers for the module's action types. `config` has been
    /// validated against [`ModuleManifest::config`].
    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModuleError {
    #[error("module `{0}` is already registered")]
    Duplicate(ModuleId),
    #[error("unknown module `{0}`")]
    Unknown(ModuleId),
    #[error("action type `{action_type}` is outside the namespace of module `{module}`")]
    Namespace {
        module: ModuleId,
        action_type: ActionTypeId,
    },
    #[error("action type `{0}` is provided by more than one handler")]
    DuplicateActionType(ActionTypeId),
    #[error("module `{module}` requires `{dependency}`, which is not enabled")]
    MissingDependency {
        module: ModuleId,
        dependency: ModuleId,
    },
    #[error("module `{module}` is required by `{dependent}`; disable that module first")]
    Required {
        module: ModuleId,
        dependent: ModuleId,
    },
    #[error("module `{0}` is not supported on this operating system")]
    Unsupported(ModuleId),
    #[error("window `{window}` is outside the namespace of module `{module}`")]
    WindowNamespace { module: ModuleId, window: String },
    #[error("invalid module configuration: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    InvalidConfig(Vec<ParameterError>),
    #[error(
        "module `{module}` cannot read exported data of version {found} (supports up to {supported})"
    )]
    IncompatibleExport {
        module: ModuleId,
        found: u32,
        supported: u32,
    },
}

/// Persisted user choices for one module.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSettings {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "ActionParameters::is_empty")]
    pub config: ActionParameters,
}

/// Persisted state of all modules. Modules without an entry use their
/// defaults; entries of unknown modules are kept as they are.
pub type ModuleStates = BTreeMap<ModuleId, ModuleSettings>;

/// Why an action type cannot run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Availability {
    Available,
    ModuleDisabled {
        module: ModuleId,
        name: String,
    },
    /// No module provides the type (unknown namespace or removed type).
    Unknown,
}

/// A registered module together with its current state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleInfo {
    #[serde(flatten)]
    pub manifest: ModuleManifest,
    pub enabled: bool,
    /// The module can run on this operating system.
    pub supported: bool,
    pub config_values: ActionParameters,
    pub action_types: Vec<ActionType>,
}

struct Entry {
    module: Box<dyn FeatureModule>,
    enabled: bool,
    config: ActionParameters,
    /// Definitions of the module's action types, known even while disabled.
    action_types: Vec<ActionType>,
}

/// Explicitly registered feature modules and their enabled state.
pub struct ModuleRegistry {
    os: Os,
    modules: BTreeMap<ModuleId, Entry>,
}

impl fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleRegistry")
            .field("os", &self.os)
            .field(
                "modules",
                &self
                    .modules
                    .iter()
                    .map(|(id, entry)| (id, entry.enabled))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl ModuleRegistry {
    pub fn new(os: Os) -> Self {
        Self {
            os,
            modules: BTreeMap::new(),
        }
    }

    /// Registers a module with its default state.
    pub fn register(&mut self, module: Box<dyn FeatureModule>) -> Result<(), ModuleError> {
        let manifest = module.manifest().clone();
        if self.modules.contains_key(&manifest.id) {
            return Err(ModuleError::Duplicate(manifest.id));
        }
        let config = normalize_config(&manifest, &ActionParameters::new())?;
        let action_types: Vec<ActionType> = module
            .action_handlers(&config)
            .iter()
            .map(|handler| handler.definition().clone())
            .collect();
        for window in &manifest.windows {
            let owner = window
                .id
                .split_once('.')
                .and_then(|(namespace, name)| (!name.is_empty()).then_some(namespace));
            if owner != Some(manifest.id.as_str()) {
                return Err(ModuleError::WindowNamespace {
                    module: manifest.id,
                    window: window.id.clone(),
                });
            }
        }
        let mut seen = BTreeSet::new();
        for action_type in &action_types {
            if ModuleId::owner_of(&action_type.id).as_ref() != Some(&manifest.id) {
                return Err(ModuleError::Namespace {
                    module: manifest.id,
                    action_type: action_type.id.clone(),
                });
            }
            if !seen.insert(action_type.id.clone()) {
                return Err(ModuleError::DuplicateActionType(action_type.id.clone()));
            }
        }
        let enabled = manifest.enabled_by_default && manifest.supports(self.os);
        self.modules.insert(
            manifest.id,
            Entry {
                module,
                enabled,
                config,
                action_types,
            },
        );
        Ok(())
    }

    fn entry(&self, id: &ModuleId) -> Result<&Entry, ModuleError> {
        self.modules
            .get(id)
            .ok_or_else(|| ModuleError::Unknown(id.clone()))
    }

    pub fn contains(&self, id: &ModuleId) -> bool {
        self.modules.contains_key(id)
    }

    pub fn is_enabled(&self, id: &ModuleId) -> bool {
        self.modules.get(id).is_some_and(|entry| entry.enabled)
    }

    pub fn manifest(&self, id: &ModuleId) -> Option<&ModuleManifest> {
        self.modules.get(id).map(|entry| entry.module.manifest())
    }

    /// Enables or disables a module after checking platform support and
    /// dependencies. Only the enabled flag changes; nothing is deleted.
    pub fn set_enabled(&mut self, id: &ModuleId, enabled: bool) -> Result<(), ModuleError> {
        let manifest = self.entry(id)?.module.manifest();
        if enabled {
            if !manifest.supports(self.os) {
                return Err(ModuleError::Unsupported(id.clone()));
            }
            if let Some(dependency) = manifest
                .dependencies
                .iter()
                .find(|dependency| !self.is_enabled(dependency))
            {
                return Err(ModuleError::MissingDependency {
                    module: id.clone(),
                    dependency: dependency.clone(),
                });
            }
        } else if let Some(dependent) = self.modules.iter().find_map(|(other, entry)| {
            (entry.enabled && entry.module.manifest().dependencies.contains(id))
                .then(|| other.clone())
        }) {
            return Err(ModuleError::Required {
                module: id.clone(),
                dependent,
            });
        }
        if let Some(entry) = self.modules.get_mut(id) {
            entry.enabled = enabled;
        }
        Ok(())
    }

    /// Validates and stores a module's configuration.
    pub fn set_config(
        &mut self,
        id: &ModuleId,
        config: &ActionParameters,
    ) -> Result<(), ModuleError> {
        let normalized = normalize_config(self.entry(id)?.module.manifest(), config)?;
        if let Some(entry) = self.modules.get_mut(id) {
            entry.config = normalized;
        }
        Ok(())
    }

    /// Applies persisted states. Invalid entries (a dependency that is
    /// disabled, a configuration that no longer validates) fall back to safe
    /// values instead of failing, so a bad setting can never lock the user
    /// out. Returns a description of every entry that was not applied.
    pub fn apply_states(&mut self, states: &ModuleStates) -> Vec<String> {
        let mut problems = Vec::new();
        for (id, settings) in states {
            let Some(entry) = self.modules.get_mut(id) else {
                continue;
            };
            // Settings removed in a newer version of a module are ignored
            // instead of invalidating the settings that still exist.
            let manifest = entry.module.manifest();
            let mut known = ActionParameters::new();
            for (key, value) in settings.config.iter() {
                if manifest.config.iter().any(|spec| &spec.key == key) {
                    known = known.with(key.clone(), value.clone());
                } else {
                    problems.push(format!("{id}: ignoring unknown setting `{key}`"));
                }
            }
            match normalize_config(manifest, &known) {
                Ok(config) => entry.config = config,
                Err(error) => problems.push(format!("{id}: {error}")),
            }
            entry.enabled = settings.enabled && entry.module.manifest().supports(self.os);
        }
        // Disable modules whose dependencies are not enabled, until stable.
        loop {
            let broken: Vec<ModuleId> = self
                .modules
                .iter()
                .filter(|(_, entry)| entry.enabled)
                .filter(|(_, entry)| {
                    entry
                        .module
                        .manifest()
                        .dependencies
                        .iter()
                        .any(|dependency| !self.is_enabled(dependency))
                })
                .map(|(id, _)| id.clone())
                .collect();
            if broken.is_empty() {
                break;
            }
            for id in broken {
                problems.push(format!("{id}: disabled because a dependency is disabled"));
                if let Some(entry) = self.modules.get_mut(&id) {
                    entry.enabled = false;
                }
            }
        }
        problems
    }

    /// The current state of every registered module, for persisting.
    pub fn states(&self) -> ModuleStates {
        self.modules
            .iter()
            .map(|(id, entry)| {
                (
                    id.clone(),
                    ModuleSettings {
                        enabled: entry.enabled,
                        config: entry.config.clone(),
                    },
                )
            })
            .collect()
    }

    /// Every registered module with its state and action types.
    pub fn modules(&self) -> Vec<ModuleInfo> {
        self.modules
            .values()
            .map(|entry| {
                let manifest = entry.module.manifest().clone();
                ModuleInfo {
                    supported: manifest.supports(self.os),
                    manifest,
                    enabled: entry.enabled,
                    config_values: entry.config.clone(),
                    action_types: entry.action_types.clone(),
                }
            })
            .collect()
    }

    /// Declared schema migrations of every registered module, enabled or
    /// not, for [`deckpilot-storage`].
    pub fn schemas(&self) -> Vec<(ModuleId, Vec<ModuleMigration>)> {
        self.modules
            .iter()
            .map(|(id, entry)| (id.clone(), entry.module.manifest().migrations.clone()))
            .collect()
    }

    /// The portable settings of a module, for export.
    pub fn export_settings(&self, id: &ModuleId) -> Option<(u32, bool, ActionParameters)> {
        let entry = self.modules.get(id)?;
        let portability = &entry.module.manifest().portability;
        (portability.version > 0).then(|| {
            (
                portability.version,
                entry.enabled,
                portability.portable(&entry.config),
            )
        })
    }

    /// Validates exported settings of a module without running module code:
    /// the format version must be supported, non-portable keys are dropped
    /// (and reported), and the rest must satisfy the configuration schema.
    /// Returns the settings to apply and the dropped keys.
    pub fn validate_import(
        &self,
        id: &ModuleId,
        version: u32,
        settings: &ActionParameters,
    ) -> Result<(ActionParameters, Vec<String>), ModuleError> {
        let entry = self.entry(id)?;
        let manifest = entry.module.manifest();
        let supported = manifest.portability.version;
        if version == 0 || version > supported {
            return Err(ModuleError::IncompatibleExport {
                module: id.clone(),
                found: version,
                supported,
            });
        }
        let mut dropped = Vec::new();
        let mut kept = entry.config.clone();
        for (key, value) in settings.iter() {
            if manifest.portability.scope(key) == SettingScope::Portable {
                kept = kept.with(key.clone(), value.clone());
            } else {
                dropped.push(key.clone());
            }
        }
        let normalized = normalize_config(manifest, &kept)?;
        Ok((normalized, dropped))
    }

    /// The window `id` of an enabled module, if one is declared.
    /// `Err(name)`: the module exists but is disabled.
    pub fn window(&self, id: &str) -> Option<Result<&ModuleWindow, String>> {
        let (namespace, _) = id.split_once('.')?;
        let entry = self.modules.get(&ModuleId::new(namespace).ok()?)?;
        let manifest = entry.module.manifest();
        let window = manifest.windows.iter().find(|w| w.id == id)?;
        Some(if entry.enabled {
            Ok(window)
        } else {
            Err(manifest.name.clone())
        })
    }

    /// Pages contributed by enabled modules, in navigation order.
    pub fn pages(&self) -> Vec<(ModuleId, ModulePage)> {
        let mut pages: Vec<(ModuleId, ModulePage)> = self
            .modules
            .iter()
            .filter(|(_, entry)| entry.enabled)
            .flat_map(|(id, entry)| {
                entry
                    .module
                    .manifest()
                    .pages
                    .iter()
                    .map(|page| (id.clone(), page.clone()))
            })
            .collect();
        pages.sort_by(|a, b| a.1.order.cmp(&b.1.order).then_with(|| a.0.cmp(&b.0)));
        pages
    }

    /// Whether instances of `action_type` can currently run.
    pub fn availability(&self, action_type: &ActionTypeId) -> Availability {
        let Some(module) = ModuleId::owner_of(action_type) else {
            return Availability::Unknown;
        };
        let Some(entry) = self.modules.get(&module) else {
            return Availability::Unknown;
        };
        if !entry.action_types.iter().any(|t| &t.id == action_type) {
            Availability::Unknown
        } else if entry.enabled {
            Availability::Available
        } else {
            Availability::ModuleDisabled {
                name: entry.module.manifest().name.clone(),
                module,
            }
        }
    }

    /// Builds the registry that executes actions: the handlers of every
    /// enabled module, each limited to its module's capabilities.
    ///
    /// # Panics
    ///
    /// Never in practice: action type ids are checked for uniqueness and
    /// namespacing when a module is registered.
    pub fn action_registry(&self) -> ActionRegistry {
        let mut registry = ActionRegistry::default();
        for (id, entry) in &self.modules {
            let manifest = entry.module.manifest();
            if !entry.enabled {
                registry.mark_unavailable(
                    entry.action_types.iter().map(|t| t.id.clone()),
                    format!("the {} module is disabled", manifest.name),
                );
                continue;
            }
            for handler in entry.module.action_handlers(&entry.config) {
                registry
                    .register_for_module(id.clone(), &manifest.capabilities, handler)
                    .expect("action types are unique per module and namespaced");
            }
        }
        registry
    }
}

fn normalize_config(
    manifest: &ModuleManifest,
    config: &ActionParameters,
) -> Result<ActionParameters, ModuleError> {
    let schema = ActionType {
        id: ActionTypeId::new(format!("{}.config", manifest.id)).expect("module ids are valid"),
        name: manifest.name.clone(),
        description: String::new(),
        category: String::new(),
        parameters: manifest.config.clone(),
        requires_project: false,
        node: crate::graph::NodeSpec::default(),
    };
    schema.normalize(config).map_err(ModuleError::InvalidConfig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestModule, echo_type};
    use crate::{ParameterKind, ParameterValue};

    fn registry() -> ModuleRegistry {
        let mut registry = ModuleRegistry::new(Os::Linux);
        registry.register(Box::new(TestModule::base())).unwrap();
        registry
            .register(Box::new(TestModule::dependent()))
            .unwrap();
        registry
    }

    fn id(value: &str) -> ModuleId {
        ModuleId::new(value).unwrap()
    }

    #[test]
    fn module_ids_are_validated() {
        assert!(ModuleId::new("projects").is_ok());
        assert!(ModuleId::new("home-assistant").is_ok());
        assert!(ModuleId::new("").is_err());
        assert!(ModuleId::new("Projects").is_err());
        assert!(ModuleId::new("1password").is_err());
        assert!(ModuleId::new("a.b").is_err());
        assert_eq!(
            ModuleId::owner_of(&ActionTypeId::new("projects.next").unwrap()),
            Some(id("projects"))
        );
        assert_eq!(
            ModuleId::owner_of(&ActionTypeId::new("plain").unwrap()),
            None
        );
    }

    #[test]
    fn registers_modules_and_rejects_conflicts() {
        let mut registry = registry();
        assert!(registry.is_enabled(&id("test")));
        assert_eq!(
            registry.register(Box::new(TestModule::base())),
            Err(ModuleError::Duplicate(id("test")))
        );
        assert!(matches!(
            registry.register(Box::new(TestModule::outside_namespace())),
            Err(ModuleError::Namespace { .. })
        ));
        let types: Vec<_> = registry
            .modules()
            .into_iter()
            .flat_map(|m| m.action_types)
            .map(|t| t.id.to_string())
            .collect();
        assert_eq!(types, ["test.echo", "test.select", "test-extra.noop"]);
    }

    #[test]
    fn disabling_removes_action_types_but_keeps_them_known() {
        let mut registry = registry();
        registry.set_enabled(&id("test-extra"), false).unwrap();
        registry.set_enabled(&id("test"), false).unwrap();
        assert_eq!(
            registry.availability(&echo_type()),
            Availability::ModuleDisabled {
                module: id("test"),
                name: "Test".into()
            }
        );
        let actions = registry.action_registry();
        assert!(actions.handler(&echo_type()).is_none());
        assert_eq!(
            actions.unavailable_reason(&echo_type()),
            Some("the Test module is disabled")
        );
        assert_eq!(
            registry.availability(&ActionTypeId::new("test.gone").unwrap()),
            Availability::Unknown
        );

        registry.set_enabled(&id("test"), true).unwrap();
        assert!(registry.action_registry().handler(&echo_type()).is_some());
    }

    #[test]
    fn dependencies_are_enforced() {
        let mut registry = registry();
        assert!(matches!(
            registry.set_enabled(&id("test"), false),
            Err(ModuleError::Required { .. })
        ));
        registry.set_enabled(&id("test-extra"), false).unwrap();
        registry.set_enabled(&id("test"), false).unwrap();
        assert!(matches!(
            registry.set_enabled(&id("test-extra"), true),
            Err(ModuleError::MissingDependency { .. })
        ));
        assert!(matches!(
            registry.set_enabled(&id("missing"), true),
            Err(ModuleError::Unknown(_))
        ));
    }

    #[test]
    fn persisted_states_are_applied_safely() {
        let mut registry = registry();
        let mut states = ModuleStates::new();
        states.insert(
            id("test"),
            ModuleSettings {
                enabled: false,
                config: ActionParameters::new(),
            },
        );
        states.insert(
            id("unknown-module"),
            ModuleSettings {
                enabled: true,
                config: ActionParameters::new(),
            },
        );
        let problems = registry.apply_states(&states);
        assert!(!registry.is_enabled(&id("test")));
        // The dependent module cannot stay enabled without its dependency.
        assert!(!registry.is_enabled(&id("test-extra")));
        assert_eq!(problems.len(), 1);
        assert_eq!(registry.states().len(), 2);

        // Unknown settings are dropped, known ones survive.
        let mut states = ModuleStates::new();
        states.insert(
            id("test"),
            ModuleSettings {
                enabled: true,
                config: ActionParameters::new()
                    .with("prefix", ParameterValue::Text(">".into()))
                    .with("removed", ParameterValue::Boolean(true)),
            },
        );
        let problems = registry.apply_states(&states);
        assert_eq!(problems, ["test: ignoring unknown setting `removed`"]);
        assert_eq!(
            registry.states()[&id("test")].config.text("prefix"),
            Some(">")
        );
    }

    #[test]
    fn unsupported_platforms_stay_disabled() {
        let mut registry = ModuleRegistry::new(Os::Windows);
        registry.register(Box::new(TestModule::base())).unwrap();
        registry
            .register(Box::new(TestModule::dependent()))
            .unwrap();
        // `test-extra` only supports Linux.
        assert!(!registry.is_enabled(&id("test-extra")));
        assert_eq!(
            registry.set_enabled(&id("test-extra"), true),
            Err(ModuleError::Unsupported(id("test-extra")))
        );
        let info = registry.modules();
        assert!(!info[1].supported);
    }

    #[test]
    fn module_windows_are_namespaced_and_follow_the_enabled_state() {
        let mut registry = registry();
        let window = registry.window("test.panel").unwrap().unwrap();
        assert_eq!(window.size, crate::WindowSize::Medium);
        assert!(registry.window("test.unknown").is_none());
        assert!(registry.window("missing.panel").is_none());
        assert!(registry.window("no-namespace").is_none());
        registry.set_enabled(&id("test-extra"), false).unwrap();
        registry.set_enabled(&id("test"), false).unwrap();
        assert_eq!(registry.window("test.panel"), Some(Err("Test".into())));

        let mut module = TestModule::outside_namespace();
        module.set_window("test.panel");
        assert!(matches!(
            ModuleRegistry::new(Os::Linux).register(Box::new(module)),
            Err(ModuleError::WindowNamespace { .. })
        ));
    }

    #[test]
    fn pages_come_from_enabled_modules() {
        let mut registry = registry();
        let titles = |r: &ModuleRegistry| {
            r.pages()
                .into_iter()
                .map(|(_, page)| page.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(&registry), ["Extra", "Test page"]);
        registry.set_enabled(&id("test-extra"), false).unwrap();
        assert_eq!(titles(&registry), ["Test page"]);
    }

    #[test]
    fn configuration_is_validated_and_passed_to_handlers() {
        let mut registry = registry();
        let bad = ActionParameters::new().with("prefix", ParameterValue::Integer(3));
        assert!(matches!(
            registry.set_config(&id("test"), &bad),
            Err(ModuleError::InvalidConfig(_))
        ));
        let good = ActionParameters::new().with("prefix", ParameterValue::Text(">".into()));
        registry.set_config(&id("test"), &good).unwrap();
        let info = registry.modules();
        assert!(matches!(
            info[0].manifest.config[0].kind,
            ParameterKind::Text { .. }
        ));
        assert_eq!(info[0].config_values.text("prefix"), Some(">"));
    }
}
