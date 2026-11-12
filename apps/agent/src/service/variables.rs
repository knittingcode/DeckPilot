//! Shared variables in the agent: the authoritative resolver for runs and
//! previews, the metadata the variable picker lists, and the events that
//! tell clients a context changed.
//!
//! The agent is the only place that resolves references against real data;
//! the configuration app asks it (`listVariables`, `previewReferences`).
//! Shared variables come from the configuration database: per profile, a
//! snapshot of every definition with the global values and those of the
//! profile and its projects ([`SqliteStore::shared_variables_for`]). Runs
//! read the snapshots taken at the last reload; previews read the database
//! directly.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{PoisonError, RwLock};

use deckpilot_core::resolution::ContextResolver;
use deckpilot_core::variables::{
    CatalogEntry, CatalogFilter, MissingReason, PreviewItem, ProjectResolver, ProjectSource,
    ReferenceAvailability, ReferencePreview, Resolution, ResolutionContext, ResolutionPhase,
    SystemVariable, ValueSource, VariableGroup, VariableKey, VariableRef, VariableResolver,
    VariableScope, preview_reference, select_value,
};
use deckpilot_core::{ModuleId, Profile, ProfileId, ProjectId};
use deckpilot_ipc::{AgentEvent, ErrorCode, IpcError, Response, SecurityState};
use deckpilot_storage::{ConfigStore, SharedVariables, SqliteStore, StorageError};

use super::{Agent, locked, storage_error};

/// Resolves `shared.` references against one profile's snapshot.
/// Variables declared by a disabled module cannot be read.
struct SnapshotResolver<'a> {
    snapshot: Option<&'a SharedVariables>,
    disabled: &'a BTreeSet<ModuleId>,
}

impl VariableResolver for SnapshotResolver<'_> {
    /// Shared variables only; system variables and node outputs are
    /// resolved by [`ContextResolver`] from the context itself.
    fn resolve(&self, reference: &VariableRef, context: &ResolutionContext) -> Resolution {
        let VariableRef::Shared(key) = reference else {
            return Resolution::UnavailableUntilRuntime;
        };
        let Some(snapshot) = self.snapshot else {
            return Resolution::Missing(MissingReason::UnknownVariable);
        };
        let declared_by_disabled = snapshot
            .definition(key)
            .and_then(|definition| definition.module.as_ref())
            .is_some_and(|module| self.disabled.contains(module));
        if declared_by_disabled {
            return Resolution::PermissionDenied;
        }
        // Execution values are never stored; runs do not set any yet (D7).
        snapshot.resolve(key, context, &[])
    }
}

/// The agent's shared variables for runs: one snapshot per profile, taken
/// at the last reload. A run started after a reload reads the new values.
#[derive(Debug, Default)]
pub struct VariableStore {
    snapshots: RwLock<BTreeMap<ProfileId, SharedVariables>>,
    /// Variables declared by these modules cannot be read while the module
    /// is disabled.
    disabled_modules: RwLock<BTreeSet<ModuleId>>,
}

impl VariableStore {
    /// Takes new snapshots of every profile and returns the keys whose
    /// definition or values changed.
    pub fn reload(&self, store: &SqliteStore) -> Result<Vec<String>, StorageError> {
        let mut snapshots = BTreeMap::new();
        for profile in store.list_profiles()? {
            snapshots.insert(profile.id, store.shared_variables_for(profile.id)?);
        }
        let mut current = self
            .snapshots
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let changed = changed_keys(&current, &snapshots);
        *current = snapshots;
        Ok(changed)
    }

    pub fn set_disabled_modules(&self, modules: BTreeSet<ModuleId>) {
        *self
            .disabled_modules
            .write()
            .unwrap_or_else(PoisonError::into_inner) = modules;
    }

    fn disabled(&self) -> BTreeSet<ModuleId> {
        self.disabled_modules
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl VariableResolver for VariableStore {
    fn resolve(&self, reference: &VariableRef, context: &ResolutionContext) -> Resolution {
        let snapshots = self
            .snapshots
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let disabled = self.disabled();
        SnapshotResolver {
            snapshot: snapshots.get(&context.profile),
            disabled: &disabled,
        }
        .resolve(reference, context)
    }
}

/// Keys whose definition, or values in any profile, differ.
fn changed_keys(
    old: &BTreeMap<ProfileId, SharedVariables>,
    new: &BTreeMap<ProfileId, SharedVariables>,
) -> Vec<String> {
    let profiles: BTreeSet<&ProfileId> = old.keys().chain(new.keys()).collect();
    let mut keys = BTreeSet::new();
    for profile in profiles {
        let (before, after) = (old.get(profile), new.get(profile));
        let candidates: BTreeSet<&VariableKey> = before
            .into_iter()
            .chain(after)
            .flat_map(|snapshot| snapshot.definitions().map(|d| &d.key))
            .collect();
        for key in candidates {
            let definition =
                |s: Option<&SharedVariables>| s.and_then(|s| s.definition(key)).cloned();
            let values =
                |s: Option<&SharedVariables>| s.map(|s| s.values(key).to_vec()).unwrap_or_default();
            // A profile that appears or disappears changes no definition.
            let redefined =
                before.is_some() && after.is_some() && definition(before) != definition(after);
            if redefined || values(before) != values(after) {
                keys.insert(key.to_string());
            }
        }
    }
    keys.into_iter().collect()
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
        let values = variables.values(&definition.key);
        let selected = select_value(
            definition,
            values.iter().map(|(scope, value)| (scope, value)),
            context,
        );
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
    /// Takes new snapshots of the shared variables after the configuration
    /// changed, and announces the keys that changed.
    pub(super) fn reload_variables(&self) -> Result<(), StorageError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let changed = self.variables.reload(store)?;
        self.publish_variables(changed);
        Ok(())
    }

    /// The shared variables of `profile` as the database holds them now.
    fn current_variables(&self, profile: ProfileId) -> Result<SharedVariables, IpcError> {
        self.store()?
            .shared_variables_for(profile)
            .map_err(|e| storage_error(&e))
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
        let entries = catalog(&self.current_variables(context.profile)?, &context)
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
        let variables = self.current_variables(profile.id)?;
        let disabled = self.variables.disabled();
        let shared = SnapshotResolver {
            snapshot: Some(&variables),
            disabled: &disabled,
        };
        let resolver = ContextResolver {
            profile: &profile,
            os: self.runtime.machine().os,
            shared: &shared,
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
    use deckpilot_core::variables::{ScopeKey, VariableDefinition, VariableType, VariableValue};
    use deckpilot_core::{Project, ProjectKey};

    fn key(text: &str) -> VariableKey {
        VariableKey::new(text).unwrap()
    }

    fn store() -> (SqliteStore, Profile) {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut profile = Profile::new("Work");
        profile
            .projects
            .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
        store.save_profile(&profile).unwrap();
        (store, profile)
    }

    fn name(store: &VariableStore, context: &ResolutionContext) -> Resolution {
        store.resolve(&VariableRef::parse("shared.team.name").unwrap(), context)
    }

    #[test]
    fn a_reload_brings_the_new_value_and_names_the_changed_keys() {
        let (db, profile) = store();
        db.save_shared_variable(&VariableDefinition::new(
            key("team.name"),
            VariableType::String,
            &[VariableScope::Global, VariableScope::Profile],
        ))
        .unwrap();
        db.set_shared_variable_value(
            &key("team.name"),
            &ScopeKey::Global,
            &VariableValue::String("one".into()),
        )
        .unwrap();
        let variables = VariableStore::default();
        assert_eq!(variables.reload(&db).unwrap(), ["team.name"]);
        let context = ResolutionContext::preview(profile.id);
        assert!(matches!(
            name(&variables, &context),
            Resolution::Value { value: VariableValue::String(text), .. } if text == "one"
        ));
        db.set_shared_variable_value(
            &key("team.name"),
            &ScopeKey::Profile {
                profile: profile.id,
            },
            &VariableValue::String("two".into()),
        )
        .unwrap();
        // Until the reload, runs keep the snapshot they had.
        assert!(matches!(
            name(&variables, &context),
            Resolution::Value { value: VariableValue::String(text), .. } if text == "one"
        ));
        assert_eq!(variables.reload(&db).unwrap(), ["team.name"]);
        assert!(matches!(
            name(&variables, &context),
            Resolution::Value { value: VariableValue::String(text), .. } if text == "two"
        ));
        assert_eq!(variables.reload(&db).unwrap(), Vec::<String>::new());
        // A profile the agent does not know resolves nothing.
        assert_eq!(
            name(&variables, &ResolutionContext::preview(ProfileId::new())),
            Resolution::Missing(MissingReason::UnknownVariable)
        );
    }

    #[test]
    fn variables_of_disabled_modules_are_denied() {
        let (db, profile) = store();
        let module = ModuleId::new("projects").unwrap();
        let conflicts = db
            .declare_module_variables(
                &module,
                &[VariableDefinition {
                    default: Some(VariableValue::Boolean(true)),
                    ..VariableDefinition::new(
                        key("projects.flag"),
                        VariableType::Boolean,
                        &[VariableScope::Global],
                    )
                }],
            )
            .unwrap();
        assert!(conflicts.is_empty(), "{conflicts:?}");
        let variables = VariableStore::default();
        variables.reload(&db).unwrap();
        let reference = VariableRef::parse("shared.projects.flag").unwrap();
        let context = ResolutionContext::preview(profile.id);
        assert!(matches!(
            variables.resolve(&reference, &context),
            Resolution::Value { .. }
        ));
        variables.set_disabled_modules(BTreeSet::from([module]));
        assert_eq!(
            variables.resolve(&reference, &context),
            Resolution::PermissionDenied
        );
    }

    #[test]
    fn the_catalog_lists_metadata_for_the_context() {
        let (db, profile) = store();
        let shop = profile.projects[0].id;
        db.save_shared_variable(&VariableDefinition {
            sensitive: true,
            ..VariableDefinition::new(
                key("vault.pin"),
                VariableType::String,
                &[VariableScope::Project],
            )
        })
        .unwrap();
        db.set_shared_variable_value(
            &key("vault.pin"),
            &ScopeKey::Project {
                profile: profile.id,
                project: shop,
            },
            &VariableValue::String("s3cr3t".into()),
        )
        .unwrap();
        let variables = db.shared_variables_for(profile.id).unwrap();
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
