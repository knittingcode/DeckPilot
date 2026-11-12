//! Shared variables in the agent: the authoritative resolver for runs and
//! previews, the metadata the variable picker lists, and the events that
//! tell clients a context changed.
//!
//! The agent is the only place that resolves references against real data;
//! the configuration app asks it (`listVariables`, `previewReferences`).
//! Until the configuration database stores shared variables (workstream E),
//! the set is held in memory: empty in production, filled by tests through
//! [`Agent::with_shared_variables`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{PoisonError, RwLock};

use deckpilot_core::resolution::ContextResolver;
use deckpilot_core::variables::{
    CatalogEntry, CatalogFilter, DefinitionIssue, MissingReason, PreviewItem, ProjectResolver,
    ProjectSource, ReferenceAvailability, ReferencePreview, Resolution, ResolutionContext,
    ResolutionPhase, ScopeKey, SystemVariable, ValueSource, VariableDefinition, VariableGroup,
    VariableKey, VariableRef, VariableResolver, VariableScope, VariableValue, preview_reference,
    select_value,
};
use deckpilot_core::{ModuleId, Profile, ProfileId, ProjectId};
use deckpilot_ipc::{AgentEvent, ErrorCode, IpcError, Response, SecurityState};
use deckpilot_storage::ConfigStore;

use super::{Agent, locked, storage_error};

/// Definitions and values of shared variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedVariables {
    definitions: BTreeMap<VariableKey, VariableDefinition>,
    values: BTreeMap<VariableKey, Vec<(ScopeKey, VariableValue)>>,
}

impl SharedVariables {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a definition. A definition with problems is refused.
    pub fn define(&mut self, definition: VariableDefinition) -> Result<(), DefinitionIssue> {
        if let Some(issue) = definition.issues().into_iter().next() {
            return Err(issue);
        }
        self.definitions.insert(definition.key.clone(), definition);
        Ok(())
    }

    /// Sets the value of `key` at `scope`, replacing an earlier one there.
    /// Execution values are never stored.
    pub fn set(
        &mut self,
        key: &VariableKey,
        scope: ScopeKey,
        value: VariableValue,
    ) -> Result<(), String> {
        let definition = self
            .definitions
            .get(key)
            .ok_or_else(|| format!("`{key}` is not defined"))?;
        if scope.scope() == VariableScope::Execution {
            return Err("execution values exist only during a run".into());
        }
        definition
            .check_value(&scope, &value)
            .map_err(|issue| issue.to_string())?;
        let values = self.values.entry(key.clone()).or_default();
        values.retain(|(existing, _)| *existing != scope);
        values.push((scope, value));
        Ok(())
    }

    pub fn definitions(&self) -> impl Iterator<Item = &VariableDefinition> {
        self.definitions.values()
    }

    fn values_of<'a>(
        &'a self,
        key: &VariableKey,
    ) -> impl Iterator<Item = (&'a ScopeKey, &'a VariableValue)> + 'a {
        self.values
            .get(key)
            .into_iter()
            .flatten()
            .map(|(scope, value)| (scope, value))
    }

    /// Keys whose definition or values differ from `other`.
    pub fn changed_keys(&self, other: &Self) -> Vec<String> {
        let keys: BTreeSet<&VariableKey> = self
            .definitions
            .keys()
            .chain(other.definitions.keys())
            .collect();
        keys.into_iter()
            .filter(|key| {
                self.definitions.get(*key) != other.definitions.get(*key)
                    || self.values.get(*key) != other.values.get(*key)
            })
            .map(ToString::to_string)
            .collect()
    }
}

/// The agent's variable store: what runs and previews read. Values are read
/// when a reference is resolved, so a run after a change sees the new
/// value; nothing is cached.
#[derive(Debug, Default)]
pub struct VariableStore {
    variables: RwLock<SharedVariables>,
    /// Variables declared by these modules cannot be read while the module
    /// is disabled.
    disabled_modules: RwLock<BTreeSet<ModuleId>>,
}

impl VariableStore {
    pub fn snapshot(&self) -> SharedVariables {
        self.variables
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the set and returns the keys that changed.
    pub fn replace(&self, variables: SharedVariables) -> Vec<String> {
        let mut current = self
            .variables
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let changed = variables.changed_keys(&current);
        *current = variables;
        changed
    }

    pub fn set_disabled_modules(&self, modules: BTreeSet<ModuleId>) {
        *self
            .disabled_modules
            .write()
            .unwrap_or_else(PoisonError::into_inner) = modules;
    }

    fn denied(&self, definition: &VariableDefinition) -> bool {
        definition.module.as_ref().is_some_and(|module| {
            self.disabled_modules
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(module)
        })
    }
}

impl VariableResolver for VariableStore {
    /// Shared variables only; system variables and node outputs are
    /// resolved by [`ContextResolver`] from the context itself.
    fn resolve(&self, reference: &VariableRef, context: &ResolutionContext) -> Resolution {
        let VariableRef::Shared(key) = reference else {
            return Resolution::UnavailableUntilRuntime;
        };
        let variables = self
            .variables
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(definition) = variables.definitions.get(key) else {
            return Resolution::Missing(MissingReason::UnknownVariable);
        };
        if self.denied(definition) {
            return Resolution::PermissionDenied;
        }
        match select_value(definition, variables.values_of(key), context) {
            Some((value, source)) => Resolution::Value {
                value: value.clone(),
                source,
                sensitive: definition.sensitive,
            },
            None => Resolution::Missing(MissingReason::NoValue),
        }
    }
}

/// The application's active project of each profile, as the agent last
/// saw it.
#[derive(Debug, Default)]
pub struct ActiveProjects(RwLock<BTreeMap<ProfileId, ProjectId>>);

impl ActiveProjects {
    fn set(&self, profile: ProfileId, project: Option<ProjectId>) {
        let mut map = self.0.write().unwrap_or_else(PoisonError::into_inner);
        match project {
            Some(project) => map.insert(profile, project),
            None => map.remove(&profile),
        };
    }
}

impl ProjectResolver for ActiveProjects {
    fn active_project(&self, profile: ProfileId) -> Option<ProjectId> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&profile)
            .copied()
    }
}

/// What the agent last announced, to send context events only on changes.
#[derive(Debug, Default)]
pub(super) struct ContextWatch {
    profile: Option<ProfileId>,
    project: Option<ProjectId>,
}

/// Every reference available in `context`, metadata only. Node outputs are
/// listed by the app from the graph being edited.
pub fn catalog(variables: &SharedVariables, context: &ResolutionContext) -> Vec<CatalogEntry> {
    let mut entries: Vec<CatalogEntry> = SystemVariable::ALL
        .into_iter()
        .map(|variable| system_entry(variable, context))
        .collect();
    for definition in variables.definitions() {
        let selected = select_value(definition, variables.values_of(&definition.key), context);
        let scope = match &selected {
            Some((_, ValueSource::Scope { scope })) => Some(scope.scope()),
            _ => None,
        };
        let group = match scope {
            Some(VariableScope::Project) => VariableGroup::Project,
            Some(VariableScope::Profile) => VariableGroup::Profile,
            _ => VariableGroup::Shared,
        };
        let execution_only = definition
            .scopes
            .iter()
            .all(|s| *s == VariableScope::Execution);
        let availability = if execution_only {
            ReferenceAvailability::RuntimeOnly
        } else if selected.is_none() {
            ReferenceAvailability::NotInContext
        } else {
            ReferenceAvailability::Available
        };
        entries.push(CatalogEntry {
            reference: VariableRef::Shared(definition.key.clone()),
            label: if definition.label.is_empty() {
                definition.key.to_string()
            } else {
                definition.label.clone()
            },
            description: definition.description.clone(),
            group,
            value_type: definition.value_type,
            scope,
            module: definition.module.clone(),
            sensitive: definition.sensitive,
            dynamic: definition.scopes.len() > 1
                || definition.scopes.contains(&VariableScope::Project),
            availability,
        });
    }
    entries
}

fn system_entry(variable: SystemVariable, context: &ResolutionContext) -> CatalogEntry {
    let preview = context.phase == ResolutionPhase::Preview;
    let availability = match variable {
        _ if variable.runtime_only() => ReferenceAvailability::RuntimeOnly,
        SystemVariable::ProjectCurrentId
        | SystemVariable::ProjectCurrentKey
        | SystemVariable::ProjectCurrentName
            if preview && context.project_source == ProjectSource::Execution =>
        {
            ReferenceAvailability::RuntimeOnly
        }
        SystemVariable::ProjectCurrentId
        | SystemVariable::ProjectCurrentKey
        | SystemVariable::ProjectCurrentName
            if context.project.is_none() =>
        {
            ReferenceAvailability::NotInContext
        }
        SystemVariable::DeviceCurrentId if context.device.is_none() => {
            ReferenceAvailability::NotInContext
        }
        SystemVariable::WorkflowId if context.workflow.is_none() => {
            ReferenceAvailability::NotInContext
        }
        _ => ReferenceAvailability::Available,
    };
    let group = if variable.path().starts_with("workflow.") {
        VariableGroup::Workflow
    } else {
        VariableGroup::System
    };
    CatalogEntry {
        reference: VariableRef::System(variable),
        label: variable.path().to_owned(),
        description: variable.description().to_owned(),
        group,
        value_type: variable.value_type(),
        scope: None,
        module: None,
        sensitive: false,
        dynamic: variable.dynamic(),
        availability,
    }
}

/// A client's context is always a preview: it cannot claim to be a run.
fn as_preview(context: &ResolutionContext) -> ResolutionContext {
    ResolutionContext {
        phase: ResolutionPhase::Preview,
        ..context.clone()
    }
}

impl Agent {
    /// Uses `variables` as the shared variables, before the agent runs.
    /// A stand-in until the configuration database stores them.
    #[must_use]
    pub fn with_shared_variables(self, variables: SharedVariables) -> Self {
        let changed = self.variables.replace(variables);
        self.publish_variables(changed);
        self
    }

    /// The profile a client's context names: the active one, or one from
    /// the database.
    fn context_profile(&self, id: ProfileId) -> Result<Profile, IpcError> {
        if self.security != SecurityState::Unlocked {
            return Err(locked());
        }
        if let Some(profile) = self.runtime.profile().filter(|p| p.id == id) {
            return Ok(profile.clone());
        }
        self.store()?
            .load_profile(id)
            .map_err(|e| storage_error(&e))?
            .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "profile does not exist"))
    }

    pub(super) fn list_variables(
        &self,
        context: &ResolutionContext,
        filter: &CatalogFilter,
    ) -> Result<Response, IpcError> {
        // Asking for another profile than the active one is fine; one that
        // does not exist is not.
        self.context_profile(context.profile)?;
        let context = as_preview(context);
        let entries = catalog(&self.variables.snapshot(), &context)
            .into_iter()
            .filter(|entry| filter.matches(entry))
            .collect();
        Ok(Response::Variables { entries })
    }

    pub(super) fn preview_references(
        &self,
        context: &ResolutionContext,
        items: &[PreviewItem],
    ) -> Result<Response, IpcError> {
        let profile = self.context_profile(context.profile)?;
        let context = as_preview(context);
        let resolver = ContextResolver {
            profile: &profile,
            os: self.runtime.machine().os,
            shared: self.variables.as_ref(),
            outputs: None,
        };
        let previews: Vec<ReferencePreview> = items
            .iter()
            .map(|item| preview_reference(&resolver, item, &context))
            .collect();
        Ok(Response::ReferencePreviews { previews })
    }

    /// Announces a change of the active profile or project, once per
    /// change, and keeps the engine's view of active projects current.
    pub(super) fn publish_context(&mut self) {
        let profile = self.runtime.profile().map(|p| p.id);
        let project = self.runtime.active_project_id();
        if let Some(id) = profile {
            self.active_projects.set(id, project);
        }
        let watch = &mut self.context_watch;
        if profile == watch.profile && project == watch.project {
            return;
        }
        let profile_changed = profile != watch.profile;
        watch.profile = profile;
        watch.project = project;
        let Some(profile) = profile else {
            return;
        };
        if profile_changed {
            let _ = self
                .handle
                .events
                .send(AgentEvent::ProfileChanged { profile });
        }
        let _ = self.handle.events.send(AgentEvent::ProjectChanged {
            profile,
            project,
            source: ProjectSource::Application,
        });
    }

    /// Announces changed shared variables (keys only).
    pub(super) fn publish_variables(&self, keys: Vec<String>) {
        if !keys.is_empty() {
            let _ = self
                .handle
                .events
                .send(AgentEvent::SharedVariablesChanged { keys });
        }
    }

    pub(super) fn publish_device_context(&self, device: Option<deckpilot_device::DeviceId>) {
        let _ = self
            .handle
            .events
            .send(AgentEvent::DeviceContextChanged { device });
    }

    pub(super) fn publish_execution_context(
        &self,
        execution: deckpilot_core::ExecutionId,
        project: Option<ProjectId>,
    ) {
        let _ = self
            .handle
            .events
            .send(AgentEvent::ExecutionContextChanged { execution, project });
    }

    /// Variables of disabled modules cannot be read.
    pub(super) fn update_variable_permissions(&self) {
        let disabled = self
            .modules
            .states()
            .into_iter()
            .filter(|(_, settings)| !settings.enabled)
            .map(|(id, _)| id)
            .collect();
        self.variables.set_disabled_modules(disabled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::variables::VariableType;
    use deckpilot_core::{Project, ProjectKey};

    fn key(text: &str) -> VariableKey {
        VariableKey::new(text).unwrap()
    }

    fn profile() -> Profile {
        let mut profile = Profile::new("Work");
        profile
            .projects
            .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
        profile
    }

    #[test]
    fn a_change_is_seen_by_the_next_resolution() {
        let profile = profile();
        let store = VariableStore::default();
        let mut variables = SharedVariables::new();
        variables
            .define(VariableDefinition::new(
                key("team.name"),
                VariableType::String,
                &[VariableScope::Global, VariableScope::Profile],
            ))
            .unwrap();
        variables
            .set(
                &key("team.name"),
                ScopeKey::Global,
                VariableValue::String("one".into()),
            )
            .unwrap();
        assert_eq!(store.replace(variables.clone()), ["team.name"]);
        let reference = VariableRef::parse("shared.team.name").unwrap();
        let context = ResolutionContext::preview(profile.id);
        let value = |store: &VariableStore| match store.resolve(&reference, &context) {
            Resolution::Value {
                value: VariableValue::String(text),
                ..
            } => text,
            other => panic!("{other:?}"),
        };
        assert_eq!(value(&store), "one");
        variables
            .set(
                &key("team.name"),
                ScopeKey::Profile {
                    profile: profile.id,
                },
                VariableValue::String("two".into()),
            )
            .unwrap();
        assert_eq!(store.replace(variables.clone()), ["team.name"]);
        assert_eq!(value(&store), "two");
        assert_eq!(store.replace(variables), Vec::<String>::new());
    }

    #[test]
    fn values_are_checked_and_never_stored_for_a_run() {
        let mut variables = SharedVariables::new();
        variables
            .define(VariableDefinition::new(
                key("team.size"),
                VariableType::Integer,
                &[VariableScope::Global, VariableScope::Execution],
            ))
            .unwrap();
        assert!(
            variables
                .set(
                    &key("team.size"),
                    ScopeKey::Global,
                    VariableValue::String("x".into())
                )
                .is_err()
        );
        assert!(
            variables
                .set(
                    &key("team.size"),
                    ScopeKey::Execution {
                        execution: deckpilot_core::ExecutionId::new()
                    },
                    VariableValue::Integer(1)
                )
                .is_err()
        );
        assert!(
            variables
                .set(&key("nope.x"), ScopeKey::Global, VariableValue::Integer(1))
                .is_err()
        );
        assert!(
            variables
                .define(VariableDefinition::new(
                    key("team.empty"),
                    VariableType::Integer,
                    &[]
                ))
                .is_err()
        );
    }

    #[test]
    fn variables_of_disabled_modules_are_denied() {
        let store = VariableStore::default();
        let module = ModuleId::new("projects").unwrap();
        let mut variables = SharedVariables::new();
        variables
            .define(VariableDefinition {
                module: Some(module.clone()),
                default: Some(VariableValue::Boolean(true)),
                ..VariableDefinition::new(
                    key("projects.flag"),
                    VariableType::Boolean,
                    &[VariableScope::Global],
                )
            })
            .unwrap();
        store.replace(variables);
        let reference = VariableRef::parse("shared.projects.flag").unwrap();
        let context = ResolutionContext::preview(ProfileId::new());
        assert!(matches!(
            store.resolve(&reference, &context),
            Resolution::Value { .. }
        ));
        store.set_disabled_modules(BTreeSet::from([module]));
        assert_eq!(
            store.resolve(&reference, &context),
            Resolution::PermissionDenied
        );
    }

    #[test]
    fn the_catalog_lists_metadata_for_the_context() {
        let profile = profile();
        let shop = profile.projects[0].id;
        let mut variables = SharedVariables::new();
        variables
            .define(VariableDefinition {
                sensitive: true,
                ..VariableDefinition::new(
                    key("vault.pin"),
                    VariableType::String,
                    &[VariableScope::Project],
                )
            })
            .unwrap();
        variables
            .set(
                &key("vault.pin"),
                ScopeKey::Project {
                    profile: profile.id,
                    project: shop,
                },
                VariableValue::String("s3cr3t".into()),
            )
            .unwrap();
        let without = ResolutionContext::preview(profile.id);
        let with = without
            .clone()
            .with_project(Some(shop), ProjectSource::Application);
        let entry = |context: &ResolutionContext, reference: &str| {
            catalog(&variables, context)
                .into_iter()
                .find(|e| e.reference.to_string() == reference)
                .unwrap()
        };
        let pin = entry(&with, "shared.vault.pin");
        assert_eq!(pin.group, VariableGroup::Project);
        assert_eq!(pin.scope, Some(VariableScope::Project));
        assert!(pin.sensitive);
        assert_eq!(pin.availability, ReferenceAvailability::Available);
        let json = serde_json::to_string(&catalog(&variables, &with)).unwrap();
        assert!(!json.contains("s3cr3t"), "metadata only");
        assert_eq!(
            entry(&without, "shared.vault.pin").availability,
            ReferenceAvailability::NotInContext
        );
        assert_eq!(
            entry(&without, "system.project.current.key").availability,
            ReferenceAvailability::NotInContext
        );
        let run = without
            .clone()
            .with_project(Some(shop), ProjectSource::Execution);
        assert_eq!(
            entry(&run, "system.project.current.key").availability,
            ReferenceAvailability::RuntimeOnly
        );
        assert_eq!(
            entry(&with, "workflow.execution.id").group,
            VariableGroup::Workflow
        );
    }
}
