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
//!
//! Core and module variables (and their legacy aliases) are resolved
//! through the module registry's variable registry. The agent supplies the
//! live state they may need: the work session's snapshot at the moment of
//! the resolution ([`VariableStore`] is the [`LiveState`]), from which the
//! work session module computes `workSession.…` (D5).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{PoisonError, RwLock};
use std::time::UNIX_EPOCH;

use deckpilot_core::resolution::ContextResolver;
use deckpilot_core::variables::{
    CatalogEntry, CatalogFilter, LiveState, MissingReason, PreviewItem, ProjectResolver,
    ProjectSource, ReferenceAvailability, ReferencePreview, Resolution, ResolutionContext,
    ResolutionPhase, ValueSource, VariableGroup, VariableKey, VariableOwner, VariableRef,
    VariableRegistry, VariableResolver, VariableScope, preview_reference, select_value,
};
use deckpilot_core::work_session::{WorkSessionSnapshot, WorkSessionVariable};
use deckpilot_core::{ModuleId, Profile, ProfileId, ProjectId};
use deckpilot_ipc::{AgentEvent, ErrorCode, IpcError, Response, SecurityState};
use deckpilot_storage::{ConfigStore, SharedVariables, SqliteStore, StorageError};
use tokio::sync::watch;

use super::{Agent, Clock, locked, storage_error};

/// The live work session for resolutions: the published snapshot and the
/// agent's wall clock.
#[derive(Clone)]
pub struct WorkSessionSource {
    snapshot: watch::Receiver<WorkSessionSnapshot>,
    clock: Clock,
}

impl std::fmt::Debug for WorkSessionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkSessionSource")
            .field("snapshot", &*self.snapshot.borrow())
            .finish_non_exhaustive()
    }
}

impl WorkSessionSource {
    pub fn new(snapshot: watch::Receiver<WorkSessionSnapshot>, clock: Clock) -> Self {
        Self { snapshot, clock }
    }

    fn now_ms(&self) -> i64 {
        (self.clock)()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// The value of a work session variable from `snapshot` at `now_ms`;
/// the work session module owns the variables and their resolution.
pub fn work_session_value(
    variable: WorkSessionVariable,
    snapshot: &WorkSessionSnapshot,
    now_ms: i64,
) -> Resolution {
    deckpilot_modules::work_session::variables::value(variable, snapshot, now_ms)
}

/// The work session variables whose value differs between two snapshots,
/// by canonical id and legacy id; elapsed time only when the timer starts
/// or stops, since clients count it on their own.
pub fn changed_work_session_variables(
    before: &WorkSessionSnapshot,
    after: &WorkSessionSnapshot,
) -> Vec<String> {
    let can_finish = |s: &WorkSessionSnapshot| {
        s.verification
            .as_ref()
            .and_then(|v| v.outcome.as_ref())
            .map(|o| o.can_finish)
    };
    let same_session = before.session == after.session;
    WorkSessionVariable::ALL
        .into_iter()
        .filter(|variable| {
            !same_session
                || match variable {
                    WorkSessionVariable::Id => false,
                    WorkSessionVariable::State => before.state != after.state,
                    WorkSessionVariable::StartedAt => before.started_at_ms != after.started_at_ms,
                    WorkSessionVariable::ElapsedSeconds => {
                        before.state.timer_running() != after.state.timer_running()
                    }
                    WorkSessionVariable::ProjectId => before.project != after.project,
                    WorkSessionVariable::CanFinish => can_finish(before) != can_finish(after),
                }
        })
        .flat_map(|variable| {
            [
                deckpilot_modules::work_session::variables::canonical(variable).to_owned(),
                variable.path().to_owned(),
            ]
        })
        .collect()
}

/// Resolves `shared.` references against one profile's snapshot.
/// Variables declared by a disabled module cannot be read.
struct SnapshotResolver<'a> {
    snapshot: Option<&'a SharedVariables>,
    disabled: &'a BTreeSet<ModuleId>,
}

impl VariableResolver for SnapshotResolver<'_> {
    /// Shared variables only; core and module variables and node outputs
    /// are resolved by [`ContextResolver`].
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
    /// The live work session; none in stores made for tests.
    work_session: Option<WorkSessionSource>,
}

impl VariableStore {
    /// A store whose resolutions read the work session from `source`.
    pub fn with_work_session(source: WorkSessionSource) -> Self {
        Self {
            work_session: Some(source),
            ..Self::default()
        }
    }

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

/// The live state of core and module variables: the work session now.
impl LiveState for VariableStore {
    fn work_session(&self) -> Option<WorkSessionSnapshot> {
        self.work_session
            .as_ref()
            .map(|source| source.snapshot.borrow().clone())
    }

    fn now_ms(&self) -> i64 {
        self.work_session.as_ref().map_or_else(
            || deckpilot_core::variables::NoLiveState.now_ms(),
            WorkSessionSource::now_ms,
        )
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

/// Every reference available in `context`, metadata only: the core's and
/// the modules' variables from `registry` (legacy aliases flagged
/// deprecated), then the shared variables. Node outputs are listed by the
/// app from the graph being edited. `work_session`: whether a work session
/// is active now.
pub fn catalog(
    registry: &VariableRegistry,
    variables: &SharedVariables,
    context: &ResolutionContext,
    work_session: bool,
) -> Vec<CatalogEntry> {
    let mut entries = registry.catalog(context, work_session);
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
        // A module-declared shared variable belongs to its module and is
        // changed with it only.
        let owner = match &definition.module {
            Some(module) => VariableOwner::Module {
                module: module.clone(),
            },
            None => VariableOwner::User,
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
            read_only: definition.module.is_some(),
            owner: Some(owner),
            deprecated: None,
        });
    }
    entries
}

/// The ids of registered variables whose availability differs between
/// two registries, each followed by its legacy aliases.
pub fn changed_availability(before: &VariableRegistry, after: &VariableRegistry) -> Vec<String> {
    let state = |registry: &VariableRegistry| -> BTreeMap<String, bool> {
        registry
            .variables()
            .map(|v| (v.declaration.id.to_string(), v.unavailable.is_none()))
            .collect()
    };
    let (old, new) = (state(before), state(after));
    let changed: BTreeSet<&String> = old
        .keys()
        .chain(new.keys())
        .filter(|id| old.get(*id) != new.get(*id))
        .collect();
    let mut keys = Vec::new();
    for id in changed {
        keys.push(id.clone());
        keys.extend(
            deckpilot_core::variables::ALIASES
                .iter()
                .filter(|alias| alias.canonical == id.as_str())
                .map(|alias| alias.alias.path().to_owned()),
        );
    }
    keys
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
        let active = self.work_session_snapshot().session.is_some();
        let entries = catalog(
            self.modules.variables(),
            &self.current_variables(context.profile)?,
            &context,
            active,
        )
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
            registry: self.modules.variables(),
            live: self.variables.as_ref(),
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

    /// Announces the work session variables whose value changed, as
    /// `sharedVariablesChanged` with their canonical and legacy ids:
    /// pickers and previews refresh on it, with no polling (§8.1).
    pub(super) fn publish_work_session_variables(
        &self,
        before: &WorkSessionSnapshot,
        after: &WorkSessionSnapshot,
    ) {
        self.publish_variables(changed_work_session_variables(before, after));
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

    /// Announces the core and module variables whose availability differs
    /// between two module registries (a module was enabled or disabled), by
    /// canonical id and by every legacy alias of it.
    pub(super) fn publish_module_variables(&self, before: &VariableRegistry) {
        self.publish_variables(changed_availability(before, self.modules.variables()));
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

    fn builtin() -> deckpilot_core::ModuleRegistry {
        deckpilot_modules::builtin(deckpilot_core::Os::Linux)
    }

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
            catalog(builtin().variables(), &variables, context, false)
                .into_iter()
                .find(|e| e.reference.to_string() == reference)
                .unwrap()
        };
        let pin = entry(&with, "shared.vault.pin");
        assert_eq!(pin.group, VariableGroup::Project);
        assert_eq!(pin.scope, Some(VariableScope::Project));
        assert!(pin.sensitive);
        assert_eq!(pin.availability, ReferenceAvailability::Available);
        let json = serde_json::to_string(&catalog(builtin().variables(), &variables, &with, false))
            .unwrap();
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

    fn session(state: deckpilot_core::work_session::WorkSessionState) -> WorkSessionSnapshot {
        use deckpilot_core::work_session::{VerificationOutcome, VerificationRecord};
        WorkSessionSnapshot {
            state,
            session: Some(deckpilot_core::WorkSessionId::new()),
            profile: Some(ProfileId::new()),
            project: Some(ProjectId::new()),
            // 2026-11-12T08:00:00Z
            started_at_ms: Some(1_794_470_400_000),
            elapsed_ms: 125_400,
            as_of_ms: 10_000,
            verification: Some(VerificationRecord {
                id: deckpilot_core::VerificationId::new(),
                outcome: Some(VerificationOutcome {
                    can_finish: true,
                    details: Some("token=abc".into()),
                }),
                at_ms: 9_000,
            }),
            approval: None,
            notices: Vec::new(),
        }
    }

    fn value(
        variable: WorkSessionVariable,
        snapshot: &WorkSessionSnapshot,
        now: i64,
    ) -> VariableValue {
        match work_session_value(variable, snapshot, now) {
            Resolution::Value {
                value,
                source: ValueSource::System,
                sensitive: false,
            } => value,
            other => panic!("{variable:?}: {other:?}"),
        }
    }

    #[test]
    fn work_session_values_follow_the_snapshot() {
        use deckpilot_core::work_session::WorkSessionState as S;
        // Without a session every variable is missing (§8.1).
        let idle = WorkSessionSnapshot::idle(5);
        for variable in WorkSessionVariable::ALL {
            assert_eq!(
                work_session_value(variable, &idle, 5),
                Resolution::Missing(MissingReason::NoValue),
                "{variable:?}"
            );
        }
        let working = session(S::Working);
        for variable in WorkSessionVariable::ALL {
            // Every value has its declared type.
            assert_eq!(
                value(variable, &working, 10_000).value_type(),
                variable.value_type(),
                "{variable:?}"
            );
        }
        assert_eq!(
            value(WorkSessionVariable::Id, &working, 0),
            VariableValue::String(working.session.unwrap().to_string())
        );
        assert_eq!(
            value(WorkSessionVariable::State, &working, 0),
            VariableValue::String("working".into())
        );
        assert_eq!(
            value(WorkSessionVariable::StartedAt, &working, 0),
            VariableValue::String("2026-11-12T08:00:00Z".into())
        );
        assert_eq!(
            value(WorkSessionVariable::ProjectId, &working, 0),
            VariableValue::ProjectId(working.project.unwrap())
        );
        assert_eq!(
            value(WorkSessionVariable::CanFinish, &working, 0),
            VariableValue::Boolean(true)
        );
        // Elapsed time is computed when resolved: the timer runs on from
        // the snapshot, never backwards.
        let elapsed = |snapshot: &WorkSessionSnapshot, now| {
            value(WorkSessionVariable::ElapsedSeconds, snapshot, now)
        };
        assert_eq!(elapsed(&working, 10_000), VariableValue::Integer(125));
        assert_eq!(elapsed(&working, 70_000), VariableValue::Integer(185));
        assert_eq!(elapsed(&working, 0), VariableValue::Integer(125));
        for state in [S::VerifyingEnd, S::ReadyToFinish] {
            assert_eq!(
                elapsed(&session(state), 70_000),
                VariableValue::Integer(185)
            );
        }
        // Not counted before the start succeeded or once stopping.
        for state in [S::Starting, S::Stopping] {
            assert_eq!(
                elapsed(&session(state), 70_000),
                VariableValue::Integer(125)
            );
        }
        // Not verified yet, no project, no start time: no value.
        let bare = WorkSessionSnapshot {
            verification: None,
            project: None,
            started_at_ms: None,
            ..session(S::Starting)
        };
        assert_eq!(
            work_session_value(WorkSessionVariable::CanFinish, &bare, 0),
            Resolution::Missing(MissingReason::NoValue)
        );
        assert_eq!(
            work_session_value(WorkSessionVariable::StartedAt, &bare, 0),
            Resolution::Missing(MissingReason::NoValue)
        );
        assert_eq!(
            work_session_value(WorkSessionVariable::ProjectId, &bare, 0),
            Resolution::Missing(MissingReason::NoProject)
        );
        // Verification details are never a value.
        let all: Vec<_> = WorkSessionVariable::ALL
            .map(|v| format!("{:?}", work_session_value(v, &working, 0)))
            .to_vec();
        assert!(!all.join(" ").contains("token=abc"));
    }

    #[test]
    fn only_changed_work_session_variables_are_announced() {
        use deckpilot_core::work_session::WorkSessionState as S;
        let working = session(S::Working);
        let paths = |before: &WorkSessionSnapshot, after: &WorkSessionSnapshot| {
            changed_work_session_variables(before, after)
        };
        // Time passing is not a change clients are told about.
        let later = WorkSessionSnapshot {
            elapsed_ms: 999_000,
            as_of_ms: 999_000,
            ..working.clone()
        };
        assert_eq!(paths(&working, &later), Vec::<String>::new());
        // A new or ended session changes everything, named by canonical
        // and by legacy id.
        assert_eq!(
            paths(&WorkSessionSnapshot::idle(0), &working).len(),
            2 * WorkSessionVariable::ALL.len()
        );
        assert_eq!(
            paths(&working, &WorkSessionSnapshot::idle(0)).len(),
            2 * WorkSessionVariable::ALL.len()
        );
        let verifying = WorkSessionSnapshot {
            verification: None,
            ..session(S::VerifyingEnd)
        };
        let verifying = WorkSessionSnapshot {
            session: working.session,
            project: working.project,
            ..verifying
        };
        assert_eq!(
            paths(&working, &verifying),
            [
                "workSession.state",
                "system.work.session.state",
                "workSession.canFinish",
                "system.work.session.canFinish"
            ]
        );
        let stopping = WorkSessionSnapshot {
            state: S::Stopping,
            ..working.clone()
        };
        assert_eq!(
            paths(&working, &stopping),
            [
                "workSession.state",
                "system.work.session.state",
                "workSession.elapsedSeconds",
                "system.work.session.elapsedSeconds"
            ]
        );
    }

    #[test]
    fn the_catalog_lists_the_work_session_with_or_without_one() {
        let (db, profile) = store();
        let variables = db.shared_variables_for(profile.id).unwrap();
        let context = ResolutionContext::preview(profile.id);
        for active in [false, true] {
            let entries: Vec<CatalogEntry> =
                catalog(builtin().variables(), &variables, &context, active)
                    .into_iter()
                    .filter(|e| e.reference.to_string().starts_with("system.work.session."))
                    .collect();
            assert_eq!(entries.len(), WorkSessionVariable::ALL.len());
            for (entry, variable) in entries.iter().zip(WorkSessionVariable::ALL) {
                assert_eq!(entry.reference.to_string(), variable.path());
                assert_eq!(entry.value_type, variable.value_type());
                assert_eq!(entry.description, variable.description());
                assert_eq!(entry.group, VariableGroup::System);
                assert!(entry.dynamic && !entry.sensitive);
                // A deprecated alias of the work session module's variable.
                assert_eq!(
                    entry.deprecated.as_ref().unwrap().canonical.as_str(),
                    deckpilot_modules::work_session::variables::canonical(variable)
                );
                assert_eq!(
                    entry.module.as_ref().map(ModuleId::as_str),
                    Some("work-session")
                );
                assert!(entry.read_only);
                let canonical = catalog(builtin().variables(), &variables, &context, active)
                    .into_iter()
                    .find(|e| {
                        e.reference.to_string()
                            == deckpilot_modules::work_session::variables::canonical(variable)
                    })
                    .unwrap();
                assert_eq!(canonical.group, VariableGroup::Module);
                assert_eq!(canonical.availability, entry.availability);
                assert!(canonical.deprecated.is_none());
                assert_eq!(
                    entry.availability,
                    if active {
                        ReferenceAvailability::Available
                    } else {
                        ReferenceAvailability::RuntimeOnly
                    }
                );
                // Listed by default, so a workflow can be written while idle.
                assert!(CatalogFilter::default().matches(entry));
            }
        }
    }
}
