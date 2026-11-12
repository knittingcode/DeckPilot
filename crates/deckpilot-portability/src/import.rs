//! Validating, previewing and planning an import.
//!
//! The flow is: [`validate`] the decrypted payload (untrusted), show a
//! [`preview`], let the user choose [`ImportOptions`], [`plan`] the changes and
//! apply them with [`deckpilot_storage::SqliteStore::apply_changes`] in one
//! transaction. Nothing in an import is executed, imported profiles are not
//! activated, and machine-specific configuration is never touched.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use deckpilot_core::variables::{ScopeKey, VariableDefinition, VariableKey, VariableValue};
use deckpilot_core::{
    Availability, LayoutIssue, ModuleId, ModuleRegistry, ModuleSettings, Profile, ProfileId,
    ProfileIssue, ProjectId, ResourceKey, ResourceKind,
};
use deckpilot_device::ModelCatalog;
use deckpilot_storage::{ConfigChanges, ConfigStore, SqliteStore};

use crate::PortabilityError;
use crate::payload::{AppSettings, Payload, resource_keys};
use deckpilot_update::policy::{PREFERENCES_SETTING, UpdatePreferences};

// Limits for untrusted archives; far above any real configuration.
const MAX_PROFILES: usize = 200;
const MAX_ITEMS_PER_PROFILE: usize = 5000;
const MAX_NAME_CHARS: usize = 200;
const MAX_MODULES: usize = 100;
const MAX_VARIABLES: usize = 1000;
const MAX_VARIABLE_VALUES: usize = 20_000;

/// Checks an untrusted payload before anything is shown or written.
/// Returns warnings that do not block the import.
pub fn validate(
    payload: &Payload,
    catalog: &ModelCatalog,
) -> Result<Vec<String>, PortabilityError> {
    let invalid = |message: String| Err(PortabilityError::Invalid(message));
    if payload.profiles.len() > MAX_PROFILES {
        return invalid(format!("more than {MAX_PROFILES} profiles"));
    }
    if payload.modules.len() > MAX_MODULES {
        return invalid(format!("more than {MAX_MODULES} modules"));
    }
    let mut profile_ids = HashSet::new();
    let mut project_ids = HashSet::new();
    let mut warnings = Vec::new();
    for profile in &payload.profiles {
        let name = &profile.name;
        if name.chars().count() > MAX_NAME_CHARS {
            return invalid("a profile name is too long".into());
        }
        if !profile_ids.insert(profile.id) {
            return invalid(format!("profile `{name}` appears twice"));
        }
        let items = profile.actions.len()
            + profile.workflows.len()
            + profile.projects.len()
            + profile.layouts.len()
            + profile
                .layouts
                .iter()
                .map(|l| l.pages.iter().map(|p| p.bindings.len() + 1).sum::<usize>())
                .sum::<usize>();
        if items > MAX_ITEMS_PER_PROFILE {
            return invalid(format!("profile `{name}` is too large"));
        }
        for project in &profile.projects {
            if !project_ids.insert(project.id) {
                return invalid(format!("a project of `{name}` appears in two profiles"));
            }
        }
        let unique = |ids: Vec<String>| ids.len() == ids.iter().collect::<HashSet<_>>().len();
        if !unique(profile.actions.iter().map(|a| a.id.to_string()).collect())
            || !unique(profile.workflows.iter().map(|w| w.id.to_string()).collect())
            || !unique(profile.layouts.iter().map(|l| l.id.to_string()).collect())
        {
            return invalid(format!("profile `{name}` has duplicate identifiers"));
        }
        // Icons are decoded (with limits) before anything is shown, so a
        // crafted image is refused here.
        for icon in &profile.icons {
            if let Err(error) = deckpilot_render::icons::sanitize(icon) {
                return invalid(format!("profile `{name}`: icon “{}”: {error}", icon.name));
            }
        }
        for issue in profile.validate(catalog) {
            match issue {
                // Device capabilities may differ between versions; such
                // layouts are imported and shown with their problems.
                ProfileIssue::Layout {
                    issue:
                        LayoutIssue::UnknownModel(_)
                        | LayoutIssue::UnknownControl { .. }
                        | LayoutIssue::UnsupportedTrigger { .. }
                        | LayoutIssue::DuplicateBinding { .. },
                    ..
                } => warnings.push(format!("{name}: {issue}")),
                // Broken references would violate referential integrity.
                other => return invalid(format!("profile `{name}`: {other}")),
            }
        }
    }
    validate_variables(payload)?;
    Ok(warnings)
}

/// The shared variables of an untrusted payload: valid user definitions,
/// each key once, and values only for the exported profiles and their own
/// projects, of the exported type and allowed scope.
fn validate_variables(payload: &Payload) -> Result<(), PortabilityError> {
    let invalid = |message: String| Err(PortabilityError::Invalid(message));
    let variables = &payload.variables;
    if variables.definitions.len() > MAX_VARIABLES || variables.values.len() > MAX_VARIABLE_VALUES {
        return invalid("too many shared variables".into());
    }
    let mut definitions: BTreeMap<&VariableKey, &VariableDefinition> = BTreeMap::new();
    for definition in &variables.definitions {
        if definition.module.is_some() {
            return invalid(format!(
                "`{}` claims to be a module's variable",
                definition.key
            ));
        }
        if let Some(issue) = definition.issues().into_iter().next() {
            return invalid(issue.to_string());
        }
        if definitions.insert(&definition.key, definition).is_some() {
            return invalid(format!("the variable `{}` appears twice", definition.key));
        }
    }
    let mut slots = HashSet::new();
    for value in &variables.values {
        match value.scope {
            ScopeKey::Global => {}
            ScopeKey::Profile { profile } => {
                if !payload.profiles.iter().any(|p| p.id == profile) {
                    return invalid(format!(
                        "a value of `{}` belongs to a profile that is not in the export",
                        value.key
                    ));
                }
            }
            ScopeKey::Project { profile, project } => {
                let owned = payload
                    .profiles
                    .iter()
                    .find(|p| p.id == profile)
                    .is_some_and(|p| p.project(project).is_some());
                if !owned {
                    return invalid(format!(
                        "a value of `{}` belongs to a project that is not in its profile",
                        value.key
                    ));
                }
            }
            ScopeKey::Execution { .. } => {
                return invalid(format!("`{}` has a value of a run", value.key));
            }
        }
        if let Some(definition) = definitions.get(&value.key)
            && let Err(issue) = definition.check_value(&value.scope, &value.value)
        {
            return invalid(issue.to_string());
        }
        if !slots.insert((&value.key, value.scope)) {
            return invalid(format!("`{}` has two values at one scope", value.key));
        }
    }
    Ok(())
}

/// How an imported profile relates to the local configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Conflict {
    /// A local profile has the same identifier (it was exported from here
    /// or imported before).
    SameId { name: String },
    /// A different local profile has the same name.
    SameName { id: ProfileId },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePreview {
    pub id: ProfileId,
    pub name: String,
    pub actions: usize,
    pub workflows: usize,
    pub projects: usize,
    pub layouts: usize,
    pub conflict: Option<Conflict>,
    /// Modules whose action types this profile uses.
    pub modules: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModuleStatus {
    /// Settings can be applied.
    Ready,
    /// The module is not part of this DeckPilot; its data is not imported.
    Missing,
    /// The export is newer than this module understands; not imported.
    Incompatible,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModulePreview {
    pub id: String,
    pub name: Option<String>,
    pub status: ModuleStatus,
    pub enabled: bool,
    /// Settings that are not portable for the local module and are skipped.
    pub skipped_settings: Vec<String>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingResource {
    pub key: ResourceKey,
    pub kind: Option<ResourceKind>,
    /// Profiles that use it.
    pub profiles: Vec<String>,
}

/// What an import would bring, shown before the user decides.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub exported_at: i64,
    pub app_version: String,
    pub profiles: Vec<ProfilePreview>,
    pub modules: Vec<ModulePreview>,
    /// Action types no installed module provides; such actions are kept and
    /// shown as unavailable.
    pub unknown_action_types: Vec<String>,
    /// Resources the profiles use that are not set up on this computer.
    pub missing_resources: Vec<MissingResource>,
    pub warnings: Vec<String>,
    /// Local profiles that a replacing import would remove.
    pub local_profiles: Vec<(ProfileId, String)>,
    /// Portable application settings in the export.
    pub settings: AppSettings,
    pub variables: VariablesPreview,
}

/// The shared variables an import brings.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesPreview {
    /// Variables that do not exist here and would be created.
    pub new: Vec<VariableKey>,
    /// Variables that exist here with the same type; the local definition
    /// is kept.
    pub existing: Vec<VariableKey>,
    /// Exported variables whose type differs from the local one; their
    /// values are not imported.
    pub incompatible: Vec<VariableKey>,
    /// Values of profiles and projects.
    pub values: usize,
    /// Global values, applied only with the application settings.
    pub global_values: usize,
}

fn variables_preview(
    payload: &Payload,
    store: &SqliteStore,
) -> Result<VariablesPreview, PortabilityError> {
    let mut preview = VariablesPreview::default();
    for definition in &payload.variables.definitions {
        match store.shared_variable(&definition.key)? {
            None => preview.new.push(definition.key.clone()),
            Some(local) if local.definition.value_type != definition.value_type => {
                preview.incompatible.push(definition.key.clone());
            }
            Some(_) => preview.existing.push(definition.key.clone()),
        }
    }
    for value in &payload.variables.values {
        if value.scope == ScopeKey::Global {
            preview.global_values += 1;
        } else {
            preview.values += 1;
        }
    }
    Ok(preview)
}

/// Describes `payload` relative to the local configuration.
pub fn preview(
    payload: &Payload,
    store: &SqliteStore,
    modules: &ModuleRegistry,
    warnings: Vec<String>,
) -> Result<ImportPreview, PortabilityError> {
    let local = store.list_profiles()?;
    let machine = store.load_machine_config()?;
    let mut unknown = BTreeSet::new();
    let mut profiles = Vec::new();
    let mut missing: BTreeMap<ResourceKey, MissingResource> = BTreeMap::new();
    for profile in &payload.profiles {
        let conflict = if let Some(same) = local.iter().find(|l| l.id == profile.id) {
            Some(Conflict::SameId {
                name: same.name.clone(),
            })
        } else {
            local
                .iter()
                .find(|l| l.name.eq_ignore_ascii_case(&profile.name))
                .map(|same| Conflict::SameName { id: same.id })
        };
        let mut used_modules = BTreeSet::new();
        for action in &profile.actions {
            match modules.availability(&action.action_type) {
                Availability::Unknown => {
                    unknown.insert(action.action_type.to_string());
                }
                Availability::Available | Availability::ModuleDisabled { .. } => {}
            }
            if let Some(module) = ModuleId::owner_of(&action.action_type) {
                used_modules.insert(module.to_string());
            }
        }
        for key in resource_keys(profile) {
            let bound = machine.as_ref().is_some_and(|m| m.binding(&key).is_some());
            if !bound {
                missing
                    .entry(key.clone())
                    .or_insert_with(|| MissingResource {
                        kind: payload.resources.get(&key).copied().flatten(),
                        key: key.clone(),
                        profiles: Vec::new(),
                    })
                    .profiles
                    .push(profile.name.clone());
            }
        }
        profiles.push(ProfilePreview {
            id: profile.id,
            name: profile.name.clone(),
            actions: profile.actions.len(),
            workflows: profile.workflows.len(),
            projects: profile.projects.len(),
            layouts: profile.layouts.len(),
            conflict,
            modules: used_modules.into_iter().collect(),
        });
    }
    let module_previews = payload
        .modules
        .iter()
        .map(|(id, export)| module_preview(modules, id, export))
        .collect();
    Ok(ImportPreview {
        exported_at: payload.exported_at,
        app_version: payload.app_version.clone(),
        profiles,
        modules: module_previews,
        unknown_action_types: unknown.into_iter().collect(),
        missing_resources: missing.into_values().collect(),
        warnings,
        local_profiles: local.into_iter().map(|p| (p.id, p.name)).collect(),
        settings: payload.settings.clone(),
        variables: variables_preview(payload, store)?,
    })
}

fn module_preview(
    modules: &ModuleRegistry,
    id: &str,
    export: &crate::ModuleExport,
) -> ModulePreview {
    let missing = |problem: &str| ModulePreview {
        id: id.to_owned(),
        name: None,
        status: ModuleStatus::Missing,
        enabled: export.enabled,
        skipped_settings: Vec::new(),
        problem: Some(problem.to_owned()),
    };
    let Ok(module) = ModuleId::new(id) else {
        return missing("invalid module identifier");
    };
    let Some(manifest) = modules.manifest(&module) else {
        return missing("this DeckPilot does not include the module");
    };
    match modules.validate_import(&module, export.version, &export.settings) {
        Ok((_, skipped)) => ModulePreview {
            id: id.to_owned(),
            name: Some(manifest.name.clone()),
            status: ModuleStatus::Ready,
            enabled: export.enabled,
            skipped_settings: skipped,
            problem: None,
        },
        Err(error) => ModulePreview {
            id: id.to_owned(),
            name: Some(manifest.name.clone()),
            status: ModuleStatus::Incompatible,
            enabled: export.enabled,
            skipped_settings: Vec::new(),
            problem: Some(error.to_string()),
        },
    }
}

/// Merge into or replace the local configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportMode {
    /// Add the selected profiles; conflicts are resolved per profile.
    #[default]
    Merge,
    /// Remove all local profiles and import the selected ones. Machine
    /// resources stay as they are.
    Replace,
}

/// What to do with a profile whose identifier exists locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictResolution {
    /// Import as a new, independent profile with new identifiers.
    #[default]
    KeepBoth,
    /// Overwrite the local profile.
    Replace,
    /// Do not import this profile.
    Skip,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    #[serde(default)]
    pub mode: ImportMode,
    /// Profiles to import; must be part of the export.
    pub profiles: Vec<ProfileId>,
    #[serde(default)]
    pub conflicts: BTreeMap<ProfileId, ConflictResolution>,
    /// Apply the exported module settings (enabled state and portable
    /// settings).
    #[serde(default)]
    pub module_settings: bool,
    /// Apply the exported application settings.
    #[serde(default)]
    pub app_settings: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    /// `(id, name)` of profiles written.
    pub imported: Vec<(ProfileId, String)>,
    pub removed: Vec<(ProfileId, String)>,
    pub skipped: Vec<String>,
    /// Profiles imported with new identifiers to avoid a clash.
    pub renamed: Vec<String>,
    pub modules: Vec<String>,
    /// Application settings that were applied.
    pub settings: Vec<String>,
    /// Shared variables that were created.
    pub variables: Vec<String>,
    /// Number of shared variable values that were set.
    pub variable_values: usize,
    /// Values that were not imported, and why.
    pub skipped_values: Vec<String>,
}

fn unique_name(name: &str, taken: &BTreeSet<String>) -> String {
    let base = format!("{name} (imported)");
    if !taken.contains(&base.to_lowercase()) {
        return base;
    }
    // One of the next `taken.len() + 1` numbers is free.
    for n in 2..=taken.len() + 2 {
        let candidate = format!("{name} (imported {n})");
        if !taken.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!("a free name exists")
}

/// Validated module settings to apply. Declarative validation only: no
/// module code runs, and missing or incompatible modules are skipped.
fn plan_modules(
    payload: &Payload,
    modules: &ModuleRegistry,
    summary: &mut ImportSummary,
) -> Vec<(ModuleId, ModuleSettings)> {
    let mut states = Vec::new();
    for (id, export) in &payload.modules {
        let Ok(module) = ModuleId::new(id.clone()) else {
            continue;
        };
        if !modules.contains(&module) {
            continue;
        }
        if let Ok((config, _)) = modules.validate_import(&module, export.version, &export.settings)
        {
            states.push((
                module,
                ModuleSettings {
                    enabled: export.enabled,
                    config,
                },
            ));
            summary.modules.push(id.clone());
        }
    }
    states
}

/// The export has global shared variable values (applied with the
/// application settings).
fn has_global_values(payload: &Payload) -> bool {
    payload
        .variables
        .values
        .iter()
        .any(|v| v.scope == ScopeKey::Global)
}

/// Local profiles the import removes: all of them when replacing, else
/// those the user chose to replace.
fn removed_profiles(
    options: &ImportOptions,
    local: &[deckpilot_storage::ProfileSummary],
    selected: &[&Profile],
) -> BTreeSet<ProfileId> {
    if options.mode == ImportMode::Replace {
        return local.iter().map(|p| p.id).collect();
    }
    selected
        .iter()
        .filter(|profile| {
            local.iter().any(|l| l.id == profile.id)
                && options
                    .conflicts
                    .get(&profile.id)
                    .copied()
                    .unwrap_or_default()
                    == ConflictResolution::Replace
        })
        .map(|profile| profile.id)
        .collect()
}

/// Turns the user's choices into one set of changes. Nothing is written.
pub fn plan(
    payload: &Payload,
    options: &ImportOptions,
    store: &SqliteStore,
    modules: &ModuleRegistry,
) -> Result<(ConfigChanges, ImportSummary), PortabilityError> {
    let mut changes = ConfigChanges::default();
    let mut summary = ImportSummary::default();
    let local = store.list_profiles()?;
    let selected: Vec<&Profile> = options
        .profiles
        .iter()
        .map(|id| {
            payload
                .profiles
                .iter()
                .find(|p| p.id == *id)
                .ok_or_else(|| {
                    PortabilityError::Invalid("a selected profile is not in the export".into())
                })
        })
        .collect::<Result<_, _>>()?;
    let apply_settings = options.app_settings && !payload.settings.is_empty();
    let settings_selected = options.app_settings && (apply_settings || has_global_values(payload));
    if selected.is_empty() && !options.module_settings && !settings_selected {
        return Err(PortabilityError::Invalid(
            "nothing selected to import".into(),
        ));
    }

    // Local profiles that will remain, and their projects: an imported
    // project must never end up in two profiles.
    let removed = removed_profiles(options, &local, &selected);
    // Where the exported profiles and projects ended up, for their values.
    let mut moved_profiles: BTreeMap<ProfileId, ProfileId> = BTreeMap::new();
    let mut moved_projects: BTreeMap<ProjectId, (ProfileId, ProjectId)> = BTreeMap::new();
    let mut remaining_projects: HashSet<ProjectId> = HashSet::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for summary_entry in &local {
        if removed.contains(&summary_entry.id) {
            continue;
        }
        names.insert(summary_entry.name.to_lowercase());
        if let Some(profile) = store.load_profile(summary_entry.id)? {
            remaining_projects.extend(profile.projects.iter().map(|p| p.id));
        }
    }

    for profile in selected {
        let exists = local.iter().any(|l| l.id == profile.id) && !removed.contains(&profile.id);
        let resolution = options
            .conflicts
            .get(&profile.id)
            .copied()
            .unwrap_or_default();
        if exists && resolution == ConflictResolution::Skip {
            summary.skipped.push(profile.name.clone());
            continue;
        }
        let clashes = profile
            .projects
            .iter()
            .any(|p| remaining_projects.contains(&p.id));
        let imported = if exists || clashes {
            // New identifiers for the profile and everything in it.
            let name = unique_name(&profile.name, &names);
            summary.renamed.push(name.clone());
            profile.duplicate(&name)
        } else {
            profile.clone()
        };
        // Icons are stored as DeckPilot encoded them, never as they came.
        let mut imported = imported;
        for icon in &mut imported.icons {
            *icon = deckpilot_render::icons::sanitize(icon)
                .map_err(|e| PortabilityError::Invalid(format!("icon “{}”: {e}", icon.name)))?;
        }
        names.insert(imported.name.to_lowercase());
        remaining_projects.extend(imported.projects.iter().map(|p| p.id));
        moved_profiles.insert(profile.id, imported.id);
        // `duplicate` keeps the order of the projects.
        for (old, new) in profile.projects.iter().zip(&imported.projects) {
            moved_projects.insert(old.id, (imported.id, new.id));
        }
        summary.imported.push((imported.id, imported.name.clone()));
        changes.save_profiles.push(imported);
    }
    for id in &removed {
        changes.delete_profiles.push(*id);
        if let Some(entry) = local.iter().find(|l| l.id == *id) {
            summary.removed.push((entry.id, entry.name.clone()));
        }
    }

    if options.module_settings {
        changes.module_states = plan_modules(payload, modules, &mut summary);
    }
    let moved = Moved {
        profiles: &moved_profiles,
        projects: &moved_projects,
    };
    plan_variables(
        payload,
        store,
        options.app_settings,
        &moved,
        &mut changes,
        &mut summary,
    )?;
    if apply_settings && let Some(updates) = payload.settings.updates {
        changes.settings.push(plan_update_settings(updates, store)?);
        summary.settings.push("updates".into());
    }
    Ok((changes, summary))
}

/// Where exported profiles and projects were imported to.
struct Moved<'a> {
    profiles: &'a BTreeMap<ProfileId, ProfileId>,
    projects: &'a BTreeMap<ProjectId, (ProfileId, ProjectId)>,
}

impl Moved<'_> {
    /// The scope a value lands at; `None`: its profile is not imported.
    fn scope(&self, scope: ScopeKey, app_settings: bool) -> Option<ScopeKey> {
        match scope {
            ScopeKey::Global if app_settings => Some(ScopeKey::Global),
            ScopeKey::Profile { profile } => self
                .profiles
                .get(&profile)
                .map(|profile| ScopeKey::Profile { profile: *profile }),
            ScopeKey::Project { project, .. } => {
                self.projects
                    .get(&project)
                    .map(|(profile, project)| ScopeKey::Project {
                        profile: *profile,
                        project: *project,
                    })
            }
            // Global values without the application settings; values of a
            // run are refused by validation.
            ScopeKey::Global | ScopeKey::Execution { .. } => None,
        }
    }

    /// The value with project and profile ids moved along; `Err`: why it
    /// cannot be imported at `scope`.
    fn value(&self, value: &VariableValue, scope: ScopeKey) -> Result<VariableValue, &'static str> {
        let owner = match scope {
            ScopeKey::Profile { profile } | ScopeKey::Project { profile, .. } => Some(profile),
            ScopeKey::Global | ScopeKey::Execution { .. } => None,
        };
        Ok(match value {
            VariableValue::ProjectId(project) => match self.projects.get(project) {
                Some((profile, id)) if owner.is_none_or(|owner| owner == *profile) => {
                    VariableValue::ProjectId(*id)
                }
                Some(_) => return Err("it names a project of another profile"),
                None if owner.is_some() => return Err("it names a project that is not imported"),
                None => value.clone(),
            },
            VariableValue::ProfileId(profile) => match self.profiles.get(profile) {
                Some(id) if owner.is_none_or(|owner| owner == *id) => VariableValue::ProfileId(*id),
                _ if owner.is_some() => return Err("it names another profile"),
                _ => value.clone(),
            },
            other => other.clone(),
        })
    }
}

/// The shared variables to create and the values to set. Local
/// definitions are never replaced; values follow their profile or project
/// to where it was imported; global values only with the application
/// settings. A value that does not fit is skipped and reported, never
/// forced.
fn plan_variables(
    payload: &Payload,
    store: &SqliteStore,
    app_settings: bool,
    moved: &Moved<'_>,
    changes: &mut ConfigChanges,
    summary: &mut ImportSummary,
) -> Result<(), PortabilityError> {
    let mut effective: BTreeMap<VariableKey, VariableDefinition> = BTreeMap::new();
    for definition in &payload.variables.definitions {
        if let Some(local) = store.shared_variable(&definition.key)? {
            effective.insert(definition.key.clone(), local.definition);
        } else {
            effective.insert(definition.key.clone(), definition.clone());
            changes.variables.push(definition.clone());
            summary.variables.push(definition.key.to_string());
        }
    }
    for exported in &payload.variables.values {
        let Some(scope) = moved.scope(exported.scope, app_settings) else {
            continue;
        };
        let skip = |reason: &str| format!("`{}`: {reason}", exported.key);
        let definition = match effective.get(&exported.key) {
            Some(definition) => Some(definition.clone()),
            // A module's variable, declared here by the module.
            None => store.shared_variable(&exported.key)?.map(|s| s.definition),
        };
        let Some(definition) = definition else {
            summary
                .skipped_values
                .push(skip("the variable does not exist here"));
            continue;
        };
        if definition.sensitive {
            summary
                .skipped_values
                .push(skip("the variable is sensitive here"));
            continue;
        }
        let value = match moved.value(&exported.value, scope) {
            Ok(value) => value,
            Err(reason) => {
                summary.skipped_values.push(skip(reason));
                continue;
            }
        };
        if let Err(issue) = definition.check_value(&scope, &value) {
            summary.skipped_values.push(skip(&issue.to_string()));
            continue;
        }
        changes
            .variable_values
            .push((exported.key.clone(), scope, value));
        summary.variable_values += 1;
    }
    Ok(())
}

/// The update preferences to store: the exported ones, with this
/// computer's time of the last check.
fn plan_update_settings(
    updates: crate::payload::UpdateSettings,
    store: &SqliteStore,
) -> Result<(String, String), PortabilityError> {
    let local: UpdatePreferences = store
        .setting(PREFERENCES_SETTING)?
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let preferences = UpdatePreferences {
        mode: updates.mode,
        automatic_checks: updates.automatic_checks,
        check_interval_hours: updates.check_interval_hours,
        last_check: local.last_check,
        // The update channel is this computer's choice; it is not exported.
        channel: local.channel,
    }
    .sanitized();
    let text = serde_json::to_string(&preferences)
        .map_err(|e| PortabilityError::Invalid(e.to_string()))?;
    Ok((PREFERENCES_SETTING.to_owned(), text))
}

#[cfg(test)]
mod tests;
