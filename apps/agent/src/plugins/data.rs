//! Validation of what a plugin receives against its manifest (action
//! parameters, settings), and the IPC shapes of its data (credential
//! slots, settings, known plugins).
//!
//! Parameters and settings use the core's parameter schema: the manifest's
//! kinds mirror `deckpilot_core::ParameterKind` one to one, so a manifest
//! parameter or setting converts to a [`ParameterSpec`] through JSON and
//! [`ActionType::normalize`] checks types, ranges, choices and lengths and
//! fills defaults. Nothing here runs plugin code.

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionParameters, ActionType, ActionTypeId, ParameterKind, ParameterSpec, ParameterValue,
    ProjectId,
};
use deckpilot_ipc::plugins as ipc;
use deckpilot_plugin_abi::{ParamValue, Params};
use deckpilot_plugin_manifest::{Manifest, Portability, Setting};
use deckpilot_plugin_store::{CredentialStatus, KnownPlugin, StoredGrant};
use serde_json::Value;

/// A JSON value as the core value `spec` expects. Plugins never receive
/// references, templates, resources or credential names.
fn core_value(spec: &ParameterSpec, value: &Value) -> Result<ParameterValue, String> {
    let wrong = |expected: &str| format!("`{}` expects {expected}", spec.key);
    Ok(match (&spec.kind, value) {
        (
            ParameterKind::Text { .. }
            | ParameterKind::Lines { .. }
            | ParameterKind::Pairs { .. }
            | ParameterKind::Code { .. },
            Value::String(s),
        ) => ParameterValue::Text(s.clone()),
        (ParameterKind::Choice { .. }, Value::String(s)) => ParameterValue::Choice(s.clone()),
        (ParameterKind::Integer { .. }, Value::Number(n)) => {
            ParameterValue::Integer(n.as_i64().ok_or_else(|| wrong("a whole number"))?)
        }
        (ParameterKind::Boolean, Value::Bool(b)) => ParameterValue::Boolean(*b),
        (ParameterKind::Project, Value::String(s)) => {
            ParameterValue::Project(s.parse::<ProjectId>().map_err(|_| wrong("a project id"))?)
        }
        (
            ParameterKind::Text { .. }
            | ParameterKind::Lines { .. }
            | ParameterKind::Pairs { .. }
            | ParameterKind::Code { .. }
            | ParameterKind::Choice { .. }
            | ParameterKind::Project,
            _,
        ) => return Err(wrong("text")),
        (ParameterKind::Integer { .. }, _) => return Err(wrong("a whole number")),
        (ParameterKind::Boolean, _) => return Err(wrong("true or false")),
        (ParameterKind::Resource { .. } | ParameterKind::Credential, _) => {
            return Err(format!("`{}` is not a kind plugins can use", spec.key));
        }
    })
}

fn json_value(value: &ParameterValue) -> Value {
    match value {
        ParameterValue::Text(s) | ParameterValue::Choice(s) => Value::String(s.clone()),
        ParameterValue::Integer(i) => Value::from(*i),
        ParameterValue::Boolean(b) => Value::Bool(*b),
        ParameterValue::Project(p) => Value::String(p.to_string()),
        // Never produced by `core_value`.
        ParameterValue::Resource(_)
        | ParameterValue::Reference(_)
        | ParameterValue::Template(_) => Value::Null,
    }
}

fn abi_value(value: &ParameterValue) -> Option<ParamValue> {
    Some(match value {
        ParameterValue::Text(s) | ParameterValue::Choice(s) => ParamValue::String(s.clone()),
        ParameterValue::Integer(i) => ParamValue::Integer(*i),
        ParameterValue::Boolean(b) => ParamValue::Bool(*b),
        ParameterValue::Project(p) => ParamValue::String(p.to_string()),
        ParameterValue::Resource(_)
        | ParameterValue::Reference(_)
        | ParameterValue::Template(_) => {
            return None;
        }
    })
}

/// Checks `values` against `schema` (all problems at once) and returns
/// them with defaults filled.
fn normalize(
    schema: &ActionType,
    values: &BTreeMap<String, Value>,
) -> Result<ActionParameters, Vec<String>> {
    let mut errors = Vec::new();
    let mut parameters = ActionParameters::new();
    for (key, value) in values {
        match schema.parameters.iter().find(|s| &s.key == key) {
            Some(spec) => match core_value(spec, value) {
                Ok(v) => parameters = parameters.with(key.clone(), v),
                Err(e) => errors.push(e),
            },
            None => errors.push(format!("unknown parameter `{key}`")),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    schema
        .normalize(&parameters)
        .map_err(|problems| problems.iter().map(ToString::to_string).collect())
}

/// Validates an invocation's parameters against the action's manifest
/// schema before anything is sent to the host: unknown parameters, wrong
/// types, values out of range, choices not offered and missing required
/// parameters are refused, and defaults are filled in.
pub fn normalize_params(definition: &ActionType, params: &Params) -> Result<Params, String> {
    let values: BTreeMap<String, Value> = params
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(Value::Null)))
        .collect();
    let normalized = normalize(definition, &values).map_err(|problems| {
        format!(
            "the parameters of {} do not match its declaration: {}",
            definition.id,
            problems.join("; ")
        )
    })?;
    normalized
        .iter()
        .map(|(key, value)| {
            abi_value(value)
                .map(|v| (key.clone(), v))
                .ok_or_else(|| format!("parameter `{key}` is not a value a plugin can receive"))
        })
        .collect()
}

// ------------------------------------------------------------- settings

fn snake_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A manifest parameter or setting (JSON) as the core's [`ParameterSpec`].
/// The manifest writes the kind's constraints in camel case
/// (`maxLength`), the core's `ParameterKind` in snake case
/// (`max_length`); serde would silently drop the camel-case ones, so they
/// are renamed first, and anything the core does not know is refused.
pub fn core_spec(mut json: Value) -> Result<ParameterSpec, String> {
    let key = json
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if let Some(Value::Object(kind)) = json.get_mut("kind") {
        let renamed: serde_json::Map<String, Value> = std::mem::take(kind)
            .into_iter()
            .map(|(k, v)| (snake_case(&k), v))
            .collect();
        *kind = renamed;
    }
    let spec: ParameterSpec =
        serde_json::from_value(json.clone()).map_err(|e| format!("`{key}`: {e}"))?;
    // Every constraint must have survived the conversion.
    let back = serde_json::to_value(&spec.kind).map_err(|e| e.to_string())?;
    if let (Some(Value::Object(given)), Value::Object(kept)) = (json.get("kind"), &back) {
        let lost = given
            .iter()
            .find(|(k, v)| !kept.contains_key(*k) && !matches!(v, Value::Bool(false)));
        if let Some((name, _)) = lost {
            return Err(format!("`{key}`: the constraint `{name}` is not supported"));
        }
    }
    Ok(spec)
}

fn setting_spec(setting: &Setting) -> Result<ParameterSpec, String> {
    let mut json = serde_json::to_value(setting).map_err(|e| e.to_string())?;
    if let Value::Object(map) = &mut json {
        map.remove("portability");
    }
    core_spec(json).map_err(|e| format!("setting {e}"))
}

/// The settings of `manifest` as one parameter schema.
fn settings_schema(manifest: &Manifest) -> Result<ActionType, String> {
    let parameters = manifest
        .contributes
        .settings()
        .iter()
        .map(setting_spec)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ActionType {
        id: ActionTypeId::new(format!("{}.settings", manifest.id)).map_err(|e| e.to_string())?,
        name: manifest.name.clone(),
        description: String::new(),
        category: String::new(),
        parameters,
        requires_project: false,
        node: deckpilot_core::graph::NodeSpec::default(),
        deprecated: false,
        controls: None,
    })
}

/// What the plugin sees: every saved value that still fits its field (an
/// update may have changed the field), otherwise the default; and the
/// required settings that have neither.
pub fn effective_settings(
    manifest: &Manifest,
    saved: &BTreeMap<String, Value>,
) -> (BTreeMap<String, Value>, Vec<String>) {
    let Ok(schema) = settings_schema(manifest) else {
        return (BTreeMap::new(), Vec::new());
    };
    let mut effective = BTreeMap::new();
    let mut missing = Vec::new();
    for spec in &schema.parameters {
        let one = ActionType {
            parameters: vec![spec.clone()],
            ..schema.clone()
        };
        let value = saved
            .get(&spec.key)
            .map(|v| BTreeMap::from([(spec.key.clone(), v.clone())]))
            .and_then(|v| normalize(&one, &v).ok())
            .or_else(|| normalize(&one, &BTreeMap::new()).ok())
            .and_then(|n| n.get(&spec.key).map(json_value));
        match value {
            Some(v) => {
                effective.insert(spec.key.clone(), v);
            }
            None if spec.required => missing.push(spec.key.clone()),
            None => {}
        }
    }
    (effective, missing)
}

/// `changes` merged into `saved` (`null` removes a value), validated
/// against the manifest. The result holds only values the user set, never
/// defaults, so a changed default in an update applies.
pub fn merge_settings(
    manifest: &Manifest,
    saved: &BTreeMap<String, Value>,
    changes: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, String> {
    let schema = settings_schema(manifest)?;
    let mut merged: BTreeMap<String, Value> = saved
        .iter()
        .filter(|(k, _)| schema.parameters.iter().any(|s| &s.key == *k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (key, value) in changes {
        if value.is_null() {
            merged.remove(key);
        } else {
            merged.insert(key.clone(), value.clone());
        }
    }
    // Required settings may stay unset while they have no value: the
    // plugin sees them as missing. Every value given must fit.
    let lenient = ActionType {
        parameters: schema
            .parameters
            .iter()
            .map(|s| ParameterSpec {
                required: s.required && merged.contains_key(&s.key),
                ..s.clone()
            })
            .collect(),
        ..schema
    };
    let normalized = normalize(&lenient, &merged).map_err(|problems| {
        format!(
            "the settings of {} do not match their declaration: {}",
            manifest.id,
            problems.join("; ")
        )
    })?;
    Ok(merged
        .keys()
        .filter_map(|k| normalized.get(k).map(|v| (k.clone(), json_value(v))))
        .collect())
}

fn portability(p: Option<Portability>) -> &'static str {
    match p.unwrap_or_default() {
        Portability::Portable => "portable",
        Portability::MachineSpecific => "machineSpecific",
        Portability::Sensitive => "sensitive",
    }
}

pub fn settings(manifest: &Manifest, saved: BTreeMap<String, Value>) -> ipc::PluginSettings {
    let (effective, missing) = effective_settings(manifest, &saved);
    ipc::PluginSettings {
        plugin: manifest.id.to_string(),
        fields: manifest
            .contributes
            .settings()
            .iter()
            .map(|s| ipc::PluginSettingField {
                key: s.key.clone(),
                label: s.label.clone(),
                description: s.description.clone(),
                kind: serde_json::to_value(&s.kind).unwrap_or(Value::Null),
                required: s.required,
                default: s
                    .default
                    .as_ref()
                    .and_then(|d| serde_json::to_value(d).ok())
                    .and_then(|d| d.get("value").cloned()),
                portability: portability(s.portability).to_owned(),
            })
            .collect(),
        values: saved,
        effective,
        missing,
        origins: Vec::new(),
    }
}

// ---------------------------------------------------------- credentials

pub fn credential_slots(
    manifest: &Manifest,
    stored: &[CredentialStatus],
    grants: &[StoredGrant],
) -> Vec<ipc::PluginCredentialSlot> {
    let permissions = super::convert::permissions(manifest, grants);
    manifest
        .contributes
        .credentials()
        .iter()
        .map(|slot| {
            let status = stored.iter().find(|s| s.key == slot.key);
            let grant = format!("credentials:{}", slot.key);
            ipc::PluginCredentialSlot {
                key: slot.key.clone(),
                label: slot.label.clone(),
                description: slot.description.clone(),
                hosts: slot.hosts.clone(),
                user_origin: slot.user_origin.clone(),
                schemes: slot
                    .schemes
                    .iter()
                    .filter_map(|s| serde_json::to_value(s).ok())
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                setup_url: slot.setup_url.clone(),
                set: status.is_some(),
                updated_at: status.map(|s| s.updated_at),
                granted: permissions
                    .iter()
                    .any(|p| p.key == grant && p.status == ipc::PermissionStatus::Granted),
            }
        })
        .collect()
}

pub fn known(plugin: &KnownPlugin, installed: bool) -> ipc::KnownPlugin {
    ipc::KnownPlugin {
        id: plugin.id.to_string(),
        name: plugin.display_name.clone(),
        publisher: plugin.publisher.clone(),
        namespace: plugin.namespace.clone(),
        last_version: plugin.last_version.clone(),
        installed,
        first_seen_at: plugin.first_seen_at,
        last_seen_at: plugin.last_seen_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/examples/hello-weather/manifest.json"
        ))
        .unwrap();
        let mut manifest: Value = serde_json::from_str(&text).unwrap();
        manifest["contributes"]["settings"] = serde_json::json!({
            "fields": [
                {"key": "city", "label": "City", "required": true,
                 "kind": {"type": "choice", "options": ["berlin", "tokyo"]},
                 "default": {"type": "choice", "value": "berlin"}},
                {"key": "retries", "label": "Retries", "required": false,
                 "kind": {"type": "integer", "min": 0, "max": 5}},
                {"key": "label", "label": "Label", "required": true,
                 "kind": {"type": "text", "maxLength": 8}, "portability": "portable"}
            ]
        });
        serde_json::from_value(manifest).unwrap()
    }

    fn greet() -> ActionType {
        let manifest = sample();
        super::super::engine::action_type(manifest.id.as_str(), &manifest.contributes.actions()[0])
            .unwrap()
    }

    fn params(json: Value) -> Params {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn parameters_are_checked_and_completed_before_the_host_sees_them() {
        let ok = normalize_params(&greet(), &params(serde_json::json!({"name": "Ada"}))).unwrap();
        assert_eq!(ok["name"], ParamValue::String("Ada".into()));
        assert_eq!(ok["style"], ParamValue::String("plain".into()));
        for (bad, says) in [
            (serde_json::json!({}), "`name` is required"),
            (
                serde_json::json!({"name": "Ada", "x": 1}),
                "unknown parameter `x`",
            ),
            (serde_json::json!({"name": 5}), "`name` expects text"),
            (
                serde_json::json!({"name": "Ada", "style": "shouting"}),
                "`shouting` is not one of plain, excited",
            ),
            (
                serde_json::json!({"name": "A".repeat(65)}),
                "at most 64 characters",
            ),
        ] {
            let problem = normalize_params(&greet(), &params(bad.clone())).unwrap_err();
            assert!(problem.contains(says), "{bad}: {problem}");
            assert!(problem.contains("acme.hello-weather.greet"), "{problem}");
        }
    }

    /// The app validates saved plugin actions with the definition the
    /// agent reports (`PluginActionInfo::definition`); it must refuse what
    /// an invocation refuses here, so every constraint of the manifest has
    /// to survive the trip as it is.
    #[test]
    fn the_app_saves_with_the_schema_invocations_are_checked_against() {
        let action: deckpilot_plugin_manifest::Action = serde_json::from_value(serde_json::json!({
            "id": "run", "name": "Run", "description": "Checks.", "category": "Test",
            "parameters": [
                {"key": "name", "label": "Name", "required": true,
                 "kind": {"type": "text", "maxLength": 8}},
                {"key": "count", "label": "Count", "required": false,
                 "kind": {"type": "integer", "min": 1, "max": 10},
                 "default": {"type": "integer", "value": 3}},
                {"key": "style", "label": "Style", "required": false,
                 "kind": {"type": "choice", "options": ["plain", "fancy"]}},
                {"key": "tags", "label": "Tags", "required": false,
                 "kind": {"type": "lines", "maxItems": 2, "maxLength": 4}},
                {"key": "headers", "label": "Headers", "required": false,
                 "kind": {"type": "pairs", "separator": ":", "maxItems": 1, "noSecrets": true}},
                {"key": "body", "label": "Body", "required": false,
                 "kind": {"type": "code", "language": "json", "maxLength": 16}},
                {"key": "on", "label": "On", "required": false, "kind": {"type": "boolean"}}
            ]
        }))
        .unwrap();
        let definition = super::super::engine::action_type("acme.checks", &action).unwrap();
        let info = ipc::PluginActionInfo {
            action: "run".into(),
            action_type: "acme.checks.run".into(),
            name: "Run".into(),
            description: None,
            available: true,
            unavailable_reason: None,
            definition: Some(definition.clone()),
        };
        let shipped: ipc::PluginActionInfo =
            serde_json::from_value(serde_json::to_value(&info).unwrap()).unwrap();
        let saved_with = shipped.definition.unwrap();
        assert_eq!(saved_with, definition);
        for (key, expected) in [
            (
                "name",
                serde_json::json!({"type": "text", "multiline": false, "max_length": 8}),
            ),
            (
                "count",
                serde_json::json!({"type": "integer", "min": 1, "max": 10}),
            ),
            (
                "tags",
                serde_json::json!({"type": "lines", "max_items": 2, "max_length": 4}),
            ),
            (
                "body",
                serde_json::json!({"type": "code", "language": "json", "max_length": 16}),
            ),
        ] {
            let spec = saved_with.parameters.iter().find(|p| p.key == key).unwrap();
            assert_eq!(serde_json::to_value(&spec.kind).unwrap(), expected, "{key}");
        }

        // The same answer for the same values, saved or invoked.
        for (values, valid) in [
            (serde_json::json!({"name": "Ada"}), true),
            (
                serde_json::json!({"name": "Ada", "count": 10, "tags": "a\nbcd"}),
                true,
            ),
            (serde_json::json!({}), false),
            (serde_json::json!({"name": "  "}), false),
            (serde_json::json!({"name": "Augusta Ada"}), false),
            (serde_json::json!({"name": "Ada", "count": 11}), false),
            (serde_json::json!({"name": "Ada", "count": 0}), false),
            (serde_json::json!({"name": "Ada", "style": "loud"}), false),
            (serde_json::json!({"name": "Ada", "tags": "a\nb\nc"}), false),
            (serde_json::json!({"name": "Ada", "tags": "abcde"}), false),
            (
                serde_json::json!({"name": "Ada", "headers": "a: 1\nb: 2"}),
                false,
            ),
            (
                serde_json::json!({"name": "Ada", "headers": "Authorization: x"}),
                false,
            ),
            (
                serde_json::json!({"name": "Ada", "body": "{\"much\": \"too long\"}"}),
                false,
            ),
            (serde_json::json!({"name": "Ada", "extra": 1}), false),
        ] {
            let invoked = normalize_params(&definition, &params(values.clone()));
            let typed: BTreeMap<String, Value> = serde_json::from_value(values.clone()).unwrap();
            let saved = normalize(&saved_with, &typed);
            assert_eq!(invoked.is_ok(), valid, "{values}: {invoked:?}");
            assert_eq!(saved.is_ok(), valid, "{values}: {saved:?}");
        }
    }

    #[test]
    fn settings_merge_validate_and_fall_back_to_defaults() {
        let manifest = sample();
        let saved = BTreeMap::new();
        let (effective, missing) = effective_settings(&manifest, &saved);
        assert_eq!(effective["city"], "berlin");
        assert!(!effective.contains_key("retries"));
        assert_eq!(missing, ["label"]);

        let changes = BTreeMap::from([
            ("retries".to_owned(), serde_json::json!(3)),
            ("label".to_owned(), serde_json::json!("home")),
        ]);
        let merged = merge_settings(&manifest, &saved, &changes).unwrap();
        assert_eq!(merged.len(), 2);
        let (effective, missing) = effective_settings(&manifest, &merged);
        assert_eq!(effective["retries"], 3);
        assert_eq!(effective["city"], "berlin");
        assert_eq!(missing, Vec::<String>::new());

        for (key, bad) in [
            ("retries", serde_json::json!(9)),
            ("retries", serde_json::json!("3")),
            ("city", serde_json::json!("paris")),
            ("label", serde_json::json!("much too long")),
            ("unknown", serde_json::json!(1)),
        ] {
            let changes = BTreeMap::from([(key.to_owned(), bad.clone())]);
            assert!(
                merge_settings(&manifest, &merged, &changes).is_err(),
                "{key} = {bad}"
            );
        }
        // `null` resets to the default.
        let reset = merge_settings(
            &manifest,
            &merged,
            &BTreeMap::from([("retries".to_owned(), Value::Null)]),
        )
        .unwrap();
        assert!(!reset.contains_key("retries"));
        // A saved value that no longer fits (after an update) falls back.
        let stale = BTreeMap::from([("city".to_owned(), serde_json::json!("paris"))]);
        assert_eq!(effective_settings(&manifest, &stale).0["city"], "berlin");

        let shown = settings(&manifest, merged);
        assert_eq!(shown.fields.len(), 3);
        assert_eq!(shown.fields[0].default, Some(serde_json::json!("berlin")));
        assert_eq!(shown.fields[2].portability, "portable");
        assert_eq!(shown.fields[1].portability, "machineSpecific");
    }
}
