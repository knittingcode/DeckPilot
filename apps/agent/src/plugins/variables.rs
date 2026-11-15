//! Plugin-owned variables (design §2.3 item 3, §3.7, §5.2 "own
//! variables").
//!
//! A plugin declares its variables in `contributes.variables` of its
//! manifest; they are registered under its namespace,
//! `plugins.<publisherCamel>.<nameCamel>.<id>`, as variables of its
//! module (`engine.rs`). Values come only from what the plugin published
//! with the `variables.publish` capability while it ran: the agent keeps
//! them per plugin, in memory ([`PublishedVariables`]), and
//! [`PluginVariableSource`] resolves from them. No plugin code runs during
//! a resolution.
//!
//! Values are not persisted. They are dropped when the plugin's host is
//! stopped for a lifecycle reason (disable, update, rollback, uninstall,
//! quarantine, crash loop, lock): references then resolve as unavailable
//! (the module is disabled or gone) or, once it runs again, as having no
//! value until it publishes again. Every drop starts a new generation, so
//! a publish from an invocation that began before it is refused.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, PoisonError, RwLock};

use deckpilot_core::variables::{
    DeclaredScope, MissingReason, Requirement, Resolution, ValueSource, VariableDeclaration,
    VariableEnvironment, VariableId, VariableSource, VariableType, VariableValue,
};
use deckpilot_plugin_manifest::{Variable, VariableRequirement, VariableScope, VariableValueKind};

/// The values one plugin published.
#[derive(Debug, Default)]
struct Values {
    generation: u64,
    /// By relative id (`temperature`).
    values: BTreeMap<String, VariableValue>,
}

/// Every plugin's published values. Cheap to clone; every clone is the
/// same store.
#[derive(Debug, Clone, Default)]
pub(crate) struct PublishedVariables(Arc<RwLock<HashMap<String, Values>>>);

impl PublishedVariables {
    /// The current generation of `plugin`'s values: an invocation captures
    /// it when it is prepared, and may publish only while it holds.
    pub fn generation(&self, plugin: &str) -> u64 {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)
            .map_or(0, |v| v.generation)
    }

    /// Sets (`Some`) or clears (`None`) one value. `false` when the
    /// generation is stale (the plugin was stopped since): nothing changes.
    pub fn set(
        &self,
        plugin: &str,
        generation: u64,
        id: &str,
        value: Option<VariableValue>,
    ) -> bool {
        let mut all = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let values = all.entry(plugin.to_owned()).or_default();
        if values.generation != generation {
            return false;
        }
        match value {
            Some(value) => {
                values.values.insert(id.to_owned(), value);
            }
            None => {
                values.values.remove(id);
            }
        }
        true
    }

    /// Clears every value of `plugin` within `generation`; the ids that
    /// had one. Nothing when the generation is stale.
    pub fn clear(&self, plugin: &str, generation: u64) -> Option<Vec<String>> {
        let mut all = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let values = all.entry(plugin.to_owned()).or_default();
        (values.generation == generation)
            .then(|| std::mem::take(&mut values.values).into_keys().collect())
    }

    /// Drops every value of `plugin` and starts a new generation (its host
    /// was stopped). The ids that had a value.
    pub fn forget(&self, plugin: &str) -> Vec<String> {
        let mut all = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let values = all.entry(plugin.to_owned()).or_default();
        values.generation += 1;
        std::mem::take(&mut values.values).into_keys().collect()
    }

    /// Drops every plugin's values (the store was closed).
    pub fn forget_all(&self) -> Vec<(String, Vec<String>)> {
        let plugins: Vec<String> = self
            .0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        plugins
            .into_iter()
            .map(|plugin| {
                let ids = self.forget(&plugin);
                (plugin, ids)
            })
            .collect()
    }

    fn get(&self, plugin: &str, id: &str) -> Option<VariableValue> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)?
            .values
            .get(id)
            .cloned()
    }
}

/// The declarations of a plugin's variables, in its namespace (read-only,
/// like every module variable). A declaration that does not make a valid
/// id is left out (the manifest's validation prevents it).
pub(crate) fn declarations(namespace: &str, variables: &[Variable]) -> Vec<VariableDeclaration> {
    variables
        .iter()
        .filter_map(|variable| {
            let id = VariableId::new(format!("{namespace}.{}", variable.id)).ok()?;
            Some(VariableDeclaration {
                id,
                name: variable.name.clone(),
                description: variable.description.clone().unwrap_or_default(),
                value_type: value_type(variable.value_type.kind),
                scope: match variable.scope {
                    VariableScope::Machine => DeclaredScope::Machine,
                    VariableScope::Profile => DeclaredScope::Profile,
                    VariableScope::Project => DeclaredScope::Project,
                },
                requires: variable
                    .requires
                    .iter()
                    .flatten()
                    .map(|requirement| match requirement {
                        VariableRequirement::Project => Requirement::Project,
                    })
                    .collect(),
                sensitive: variable.sensitive.unwrap_or(false),
                read_only: true,
            })
        })
        .collect()
}

fn value_type(kind: VariableValueKind) -> VariableType {
    match kind {
        VariableValueKind::String => VariableType::String,
        VariableValueKind::Integer => VariableType::Integer,
        VariableValueKind::Boolean => VariableType::Boolean,
        VariableValueKind::Json => VariableType::Json,
    }
}

/// A published JSON value as a value of `variable`'s declared type, or why
/// not. `null` clears the value (`Ok(None)`).
pub(crate) fn typed(
    variable: &Variable,
    value: &serde_json::Value,
) -> Result<Option<VariableValue>, String> {
    use serde_json::Value;
    let typed = match (variable.value_type.kind, value) {
        (_, Value::Null) => return Ok(None),
        (VariableValueKind::String, Value::String(text)) => VariableValue::String(text.clone()),
        (VariableValueKind::Integer, Value::Number(number)) if number.is_i64() => {
            VariableValue::Integer(number.as_i64().unwrap_or_default())
        }
        (VariableValueKind::Boolean, Value::Bool(flag)) => VariableValue::Boolean(*flag),
        (VariableValueKind::Json, other) => VariableValue::Json(other.clone()),
        (kind, _) => {
            return Err(format!(
                "`{}` is declared as {}; the value is not one",
                variable.id,
                match kind {
                    VariableValueKind::String => "a string",
                    VariableValueKind::Integer => "an integer",
                    VariableValueKind::Boolean => "a boolean",
                    VariableValueKind::Json => "JSON",
                }
            ));
        }
    };
    Ok(Some(typed))
}

/// Resolves a plugin's variables from what it published (design §3.7):
/// never runs plugin code.
pub(crate) struct PluginVariableSource {
    plugin: String,
    namespace: String,
    values: PublishedVariables,
}

impl PluginVariableSource {
    pub fn new(plugin: String, namespace: String, values: PublishedVariables) -> Self {
        Self {
            plugin,
            namespace,
            values,
        }
    }
}

impl VariableSource for PluginVariableSource {
    fn resolve(
        &self,
        declaration: &VariableDeclaration,
        _environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        let relative = declaration
            .id
            .as_str()
            .strip_prefix(&self.namespace)
            .and_then(|rest| rest.strip_prefix('.'));
        match relative.and_then(|id| self.values.get(&self.plugin, id)) {
            Some(value) => Resolution::Value {
                value,
                source: ValueSource::System,
                sensitive: false,
            },
            None => Resolution::Missing(MissingReason::NoValue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(id: &str, kind: &str, scope: &str) -> Variable {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "valueType": {"type": kind}, "scope": scope
        }))
        .unwrap()
    }

    #[test]
    fn values_are_kept_per_plugin_and_generation() {
        let store = PublishedVariables::default();
        let generation = store.generation("acme.a");
        assert!(store.set("acme.a", generation, "x", Some(VariableValue::Integer(1))));
        assert_eq!(store.get("acme.a", "x"), Some(VariableValue::Integer(1)));
        assert_eq!(store.get("acme.b", "x"), None);
        assert_eq!(store.forget("acme.a"), vec!["x".to_owned()]);
        assert_eq!(store.get("acme.a", "x"), None);
        // An invocation from before the stop publishes nothing.
        assert!(!store.set("acme.a", generation, "x", Some(VariableValue::Integer(2))));
        assert_eq!(store.clear("acme.a", generation), None);
        let next = store.generation("acme.a");
        assert_ne!(next, generation);
        assert!(store.set("acme.a", next, "x", Some(VariableValue::Integer(3))));
        assert!(store.set("acme.a", next, "y", Some(VariableValue::Boolean(true))));
        assert!(store.set("acme.a", next, "y", None));
        assert_eq!(store.clear("acme.a", next), Some(vec!["x".to_owned()]));
        assert_eq!(store.forget_all().len(), 1);
    }

    #[test]
    fn values_must_have_the_declared_type() {
        use serde_json::json;
        let count = variable("count", "integer", "machine");
        assert_eq!(
            typed(&count, &json!(42)),
            Ok(Some(VariableValue::Integer(42)))
        );
        assert_eq!(typed(&count, &json!(null)), Ok(None));
        for wrong in [json!("42"), json!(4.2), json!(true), json!([1])] {
            assert!(typed(&count, &wrong).is_err(), "{wrong}");
        }
        let text = variable("status", "string", "machine");
        assert!(typed(&text, &json!(1)).is_err());
        let flag = variable("online", "boolean", "machine");
        assert_eq!(
            typed(&flag, &json!(false)),
            Ok(Some(VariableValue::Boolean(false)))
        );
        let any = variable("raw", "json", "machine");
        assert_eq!(
            typed(&any, &json!({"a": 1})),
            Ok(Some(VariableValue::Json(json!({"a": 1}))))
        );
    }

    #[test]
    fn declarations_are_read_only_in_the_plugins_namespace() {
        let declared = declarations(
            "plugins.acme.sdkTest",
            &[
                variable("status", "string", "machine"),
                variable("per.project", "json", "project"),
            ],
        );
        assert_eq!(declared.len(), 2);
        assert_eq!(declared[0].id.as_str(), "plugins.acme.sdkTest.status");
        assert_eq!(declared[0].scope, DeclaredScope::Machine);
        assert!(declared.iter().all(|d| d.read_only));
        assert_eq!(declared[1].value_type, VariableType::Json);
        assert_eq!(declared[1].scope, DeclaredScope::Project);
    }
}
