//! What the migration engine needs from `deckpilot.db`, read on the
//! agent's service task (which owns the configuration store) and handed to
//! the plugin worker as a [`LegacySnapshot`]. Reading only: nothing here
//! writes `deckpilot.db`.
//!
//! For every module with an alias block the snapshot holds its stored
//! `modules` row, its effective settings and enabled state, the built-in
//! definitions of its uses action types, how many stored instances use
//! them, which of those a `ParamRule` cannot translate, and the secrets of
//! the credentials its credential settings name. The secrets are what an
//! import copies after the user's consent; they never leave the agent's
//! process and are never logged (`Secret`'s `Debug` hides them).

use std::collections::BTreeMap;

use deckpilot_core::alias::{ActionAlias, ParamRule};
use deckpilot_core::{
    ActionParameters, ActionType, CredentialId, CredentialSource, ModuleId, ModuleManifest,
    ModuleRegistry, ModuleSettings, ParameterKind, ParameterValue, Profile, Secret,
};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_storage::{ConfigStore, SqliteStore, StorageError};
use serde_json::Value;

use super::{MigrationPlan, plan_for};

/// One module that has an alias block.
#[derive(Debug, Clone)]
pub(crate) struct LegacyModule {
    pub module: ModuleId,
    pub plugin: PluginId,
    /// Its entries of the alias table.
    pub aliases: Vec<ActionAlias>,
    /// What the alias table does not say (credentials, origin…).
    pub plan: Option<&'static MigrationPlan>,
    /// The built-in's manifest; `None` when it is not compiled in.
    pub manifest: Option<ModuleManifest>,
    pub enabled: bool,
    /// The `modules` row as stored (`None`: never saved).
    pub row: Option<ModuleSettings>,
    /// The settings the built-in runs with (the row's values, defaults
    /// for the rest).
    pub config: ActionParameters,
    /// The built-in definitions of the uses legacy types.
    pub builtin_types: Vec<ActionType>,
    /// Stored action instances of the uses legacy types.
    pub instances: usize,
    /// Instances, bindings or nodes whose value a `ParamRule` cannot
    /// translate, by action name and parameter (never the value).
    pub untranslatable: Vec<String>,
    /// Instances, bindings or nodes that set a parameter the plugin cannot
    /// honour ([`ParamRule::Unsupported`]), by action name and parameter:
    /// they do not stop the migration, they are unavailable after it until
    /// the value is cleared, and the journal says which.
    pub unsupported: Vec<String>,
    /// Data of the module's beyond its row and instances is in use (a
    /// profile's projects, for `projects`; [`super::Extraction`]).
    pub data_in_use: bool,
    /// The credentials the built-in uses, per credential setting.
    pub credentials: Vec<LegacyCredential>,
}

/// A credential the built-in uses: its setting, the plugin's slot for it,
/// the credential id it names and, if stored, its secret.
#[derive(Debug, Clone)]
pub(crate) struct LegacyCredential {
    pub setting: String,
    pub slot: String,
    pub id: Option<CredentialId>,
    pub secret: Option<Secret>,
}

impl LegacyModule {
    /// The ids of the credentials the built-in uses, by setting: part of
    /// the source digest (never a secret).
    pub fn credential_ids(&self) -> BTreeMap<String, String> {
        self.credentials
            .iter()
            .map(|c| {
                (
                    c.setting.clone(),
                    c.id.as_ref().map(ToString::to_string).unwrap_or_default(),
                )
            })
            .collect()
    }
}

/// The modules with an alias block, as `deckpilot.db` has them now.
#[derive(Debug, Clone, Default)]
pub(crate) struct LegacySnapshot {
    pub modules: Vec<LegacyModule>,
    /// The encrypted backup `pre-plugin-extraction` of `deckpilot.db`
    /// (its file name), or why it could not be written, taken by the
    /// service task when a module of the extraction set is in use
    /// ([`super::Extraction`]). `None`: nothing to back up (no such module
    /// in use, or no encrypted database on disk).
    pub backup: Option<Result<String, String>>,
}

impl LegacySnapshot {
    /// Reads the snapshot for the aliases of `registry` (its built-in
    /// modules, before plugins are registered). Empty, and without reading
    /// anything, when there are no aliases (every build until a cut-over).
    pub fn read(store: &SqliteStore, registry: &ModuleRegistry) -> Result<Self, StorageError> {
        let aliases = registry.alias_table();
        if aliases.is_empty() {
            return Ok(Self::default());
        }
        let mut by_module: BTreeMap<String, Vec<ActionAlias>> = BTreeMap::new();
        for alias in aliases {
            by_module
                .entry(alias.module.to_owned())
                .or_default()
                .push(*alias);
        }
        let rows = store.module_states()?;
        let profiles = load_profiles(store)?;
        let secrets = store.load_credentials()?;
        let infos = registry.modules();
        let mut modules = Vec::new();
        for (module, aliases) in by_module {
            let module_id = aliases[0].module_id();
            let Ok(plugin) = PluginId::parse(aliases[0].plugin) else {
                continue;
            };
            let info = infos.iter().find(|i| i.manifest.id == module_id);
            let config = info.map(|i| i.config_values.clone()).unwrap_or_default();
            let builtin_types: Vec<ActionType> = info
                .map(|i| {
                    i.action_types
                        .iter()
                        .filter(|t| aliases.iter().any(|a| a.legacy == t.id.as_str()))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let plan = plan_for(&module);
            let Usage {
                instances,
                untranslatable,
                unsupported,
            } = usage(&profiles, &aliases, &config);
            let data_in_use = plan
                .and_then(|plan| plan.extraction)
                .is_some_and(|extraction| extraction.projects_data && uses_projects(&profiles));
            let credentials = plan
                .map(|plan| {
                    plan.credentials
                        .iter()
                        .map(|(setting, slot)| {
                            let id = credential_id(info.map(|i| &i.manifest), &config, setting);
                            let secret = id.as_ref().and_then(|id| secrets.secret(id));
                            LegacyCredential {
                                setting: (*setting).to_owned(),
                                slot: (*slot).to_owned(),
                                id,
                                secret,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            modules.push(LegacyModule {
                row: rows.get(&module_id).cloned(),
                enabled: info.is_some_and(|i| i.enabled),
                manifest: info.map(|i| i.manifest.clone()),
                module: module_id,
                plugin,
                aliases,
                plan,
                config,
                builtin_types,
                instances,
                untranslatable,
                unsupported,
                data_in_use,
                credentials,
            });
        }
        Ok(Self {
            modules,
            backup: None,
        })
    }

    /// Whether a module of the extraction set is in use here, so the
    /// backup before its first cut-over is due ([`super::Extraction`]).
    pub fn needs_backup(&self) -> bool {
        self.modules.iter().any(|m| {
            m.plan.is_some_and(|plan| plan.extraction.is_some())
                && m.manifest.is_some()
                && m.enabled
                && (m.row.is_some() || m.instances > 0 || m.data_in_use)
        })
    }
}

/// Whether a profile has projects, or a document refers to the
/// `projects` module's variables by their canonical ids.
fn uses_projects(profiles: &[Profile]) -> bool {
    profiles.iter().any(|profile| {
        !profile.projects.is_empty()
            || serde_json::to_string(profile).is_ok_and(|doc| doc.contains("projects.current."))
    })
}

fn load_profiles(store: &SqliteStore) -> Result<Vec<Profile>, StorageError> {
    let mut profiles = Vec::new();
    for summary in store.list_profiles()? {
        if let Some(profile) = store.load_profile(summary.id)? {
            profiles.push(profile);
        }
    }
    Ok(profiles)
}

/// The credential a credential setting names: its value, else its
/// default.
fn credential_id(
    manifest: Option<&ModuleManifest>,
    config: &ActionParameters,
    setting: &str,
) -> Option<CredentialId> {
    let spec = manifest?.config.iter().find(|s| s.key == setting)?;
    if spec.kind != ParameterKind::Credential {
        return None;
    }
    let value = config
        .text(setting)
        .filter(|v| !v.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| match &spec.default {
            Some(ParameterValue::Text(text)) => Some(text.clone()),
            _ => None,
        })?;
    CredentialId::new(value.trim()).ok()
}

/// Whether `value` of the translated parameter `key` is dropped by
/// [`ParamRule::DropIfDefault`]: empty, or the module's own setting.
fn translatable(value: &ParameterValue, key: &str, config: &ActionParameters) -> bool {
    matches!(value, ParameterValue::Text(text) if text.trim().is_empty())
        || config.get(key) == Some(value)
}

/// What the stored documents do with a module's legacy types.
struct Usage {
    instances: usize,
    untranslatable: Vec<String>,
    unsupported: Vec<String>,
}

/// How many stored instances use the uses types, and every instance,
/// binding override or workflow node a rule cannot translate (a value
/// other than empty or the module's setting, or a mapping, which is only
/// known at run time) or that sets a parameter the plugin cannot honour.
fn usage(profiles: &[Profile], aliases: &[ActionAlias], config: &ActionParameters) -> Usage {
    let mut instances = 0;
    let mut problems = Vec::new();
    let mut unsupported = Vec::new();
    let set =
        |value: &ParameterValue| !matches!(value, ParameterValue::Text(t) if t.trim().is_empty());
    for profile in profiles {
        let mut uses = BTreeMap::new();
        for instance in &profile.actions {
            let Some(alias) = aliases
                .iter()
                .find(|a| a.legacy == instance.action_type.as_str())
            else {
                continue;
            };
            instances += 1;
            // As the documents store it.
            if let Ok(Value::String(id)) = serde_json::to_value(instance.id) {
                uses.insert(id, (instance.name.clone(), alias));
            }
            for rule in alias.params {
                let Some(value) = instance.parameters.get(rule.key()) else {
                    continue;
                };
                match *rule {
                    ParamRule::DropIfDefault(key) if !translatable(value, key, config) => {
                        problems.push(format!("`{}` sets `{key}`", instance.name));
                    }
                    ParamRule::Unsupported(key) if set(value) => {
                        unsupported.push(format!("`{}` sets `{key}`", instance.name));
                    }
                    _ => {}
                }
            }
        }
        if uses.is_empty() {
            continue;
        }
        // Bindings and workflow nodes refer to instances by id, with
        // `overrides` and (nodes) `mappings` keyed by parameter.
        if let Ok(document) = serde_json::to_value(profile) {
            walk(&document, &mut |object| {
                let Some((name, alias)) = object
                    .get("action")
                    .and_then(Value::as_str)
                    .and_then(|id| uses.get(id))
                else {
                    return;
                };
                let overrides = object
                    .get("overrides")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<ActionParameters>(v).ok())
                    .unwrap_or_default();
                let mappings = object.get("mappings").and_then(Value::as_object);
                for rule in alias.params {
                    let (list, value_problem) = match *rule {
                        ParamRule::DropIfDefault(key) => (
                            &mut problems,
                            overrides
                                .get(key)
                                .is_some_and(|v| !translatable(v, key, config)),
                        ),
                        ParamRule::Unsupported(key) => {
                            (&mut unsupported, overrides.get(key).is_some_and(set))
                        }
                        ParamRule::Keep(_) | ParamRule::Rename(..) => continue,
                    };
                    let key = rule.key();
                    if value_problem {
                        list.push(format!("a use of `{name}` overrides `{key}`"));
                    }
                    if mappings.is_some_and(|m| m.contains_key(key)) {
                        list.push(format!("a workflow node maps `{key}` of `{name}`"));
                    }
                }
            });
        }
    }
    problems.sort();
    problems.dedup();
    unsupported.sort();
    unsupported.dedup();
    Usage {
        instances,
        untranslatable: problems,
        unsupported,
    }
}

/// Calls `visit` for every JSON object in `value`.
fn walk(value: &Value, visit: &mut dyn FnMut(&serde_json::Map<String, Value>)) {
    match value {
        Value::Object(map) => {
            visit(map);
            for child in map.values() {
                walk(child, visit);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, visit);
            }
        }
        _ => {}
    }
}
