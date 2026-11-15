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
            let (instances, untranslatable) = usage(&profiles, &aliases, &config);
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
                credentials,
            });
        }
        Ok(Self { modules })
    }
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

/// How many stored instances use the uses types, and every instance,
/// binding override or workflow node a rule cannot translate (a value
/// other than empty or the module's setting, or a mapping, which is only
/// known at run time).
fn usage(
    profiles: &[Profile],
    aliases: &[ActionAlias],
    config: &ActionParameters,
) -> (usize, Vec<String>) {
    let mut instances = 0;
    let mut problems = Vec::new();
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
            for ParamRule::DropIfDefault(key) in alias.params {
                if let Some(value) = instance.parameters.get(key)
                    && !translatable(value, key, config)
                {
                    problems.push(format!("`{}` sets `{key}`", instance.name));
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
                for ParamRule::DropIfDefault(key) in alias.params {
                    if overrides
                        .get(key)
                        .is_some_and(|v| !translatable(v, key, config))
                    {
                        problems.push(format!("a use of `{name}` overrides `{key}`"));
                    }
                    if mappings.is_some_and(|m| m.contains_key(*key)) {
                        problems.push(format!("a workflow node maps `{key}` of `{name}`"));
                    }
                }
            });
        }
    }
    problems.sort();
    problems.dedup();
    (instances, problems)
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
