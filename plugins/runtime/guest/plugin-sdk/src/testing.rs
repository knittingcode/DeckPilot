//! Native unit tests for plugins: run actions without wasm, with a
//! scripted stand-in for the agent's broker and executors.
//!
//! ```
//! use deckpilot_plugin_sdk::prelude::*;
//! use deckpilot_plugin_sdk::testing::MockHost;
//! use deckpilot_plugin_sdk::{CallResult, CapabilityRequest};
//!
//! fn ping(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
//!     let r = ctx.http_get("https://api.example.com/ping")?;
//!     Ok(Output::ok().with("status", r.status))
//! }
//! let plugin = Plugin::new("acme.ping", "1.0.0").action(Action::new("ping", ping));
//!
//! let mut denied = MockHost::deny_all();
//! let out = denied.invoke(&plugin, "ping", serde_json::json!({}));
//! assert!(matches!(out, ActionOutcome::Failed { ref code, .. } if code == "not-permitted"));
//!
//! let mut host = MockHost::new(|_req: &CapabilityRequest| {
//!     CallResult::Ok(serde_json::json!({"status": 204}))
//! });
//! let out = host.invoke(&plugin, "ping", serde_json::json!({}));
//! assert!(matches!(out, ActionOutcome::Ok { .. }));
//! assert_eq!(host.requests().len(), 1);
//! ```

use std::fmt;

use deckpilot_plugin_abi::{ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, Params};

use crate::context::Transport;
use crate::{Context, Plugin};

type Responder = Box<dyn FnMut(&CapabilityRequest) -> CallResult>;

/// Answers capability requests like the agent would, and records them.
pub struct MockHost {
    respond: Responder,
    requests: Vec<CapabilityRequest>,
}

impl fmt::Debug for MockHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockHost")
            .field("requests", &self.requests)
            .finish_non_exhaustive()
    }
}

impl MockHost {
    pub fn new(respond: impl FnMut(&CapabilityRequest) -> CallResult + 'static) -> Self {
        Self {
            respond: Box::new(respond),
            requests: Vec::new(),
        }
    }

    /// Every request succeeds with `null` (an empty HTTP 200 for fetches,
    /// a run started from the app for `run.context`, the Unix epoch in
    /// UTC for `time.now`, zero bytes for `random.bytes`: use
    /// [`MockHost::new`] for a time or bytes of your choice; a dialog is
    /// confirmed with its defaults, a selection with its first option).
    pub fn allow_all() -> Self {
        Self::new(|req| match req {
            CapabilityRequest::HttpFetch(_) => CallResult::Ok(serde_json::json!({"status": 200})),
            CapabilityRequest::RunContext(_) => {
                CallResult::Ok(serde_json::json!({"trigger": "app"}))
            }
            CapabilityRequest::TimeNow(_) => {
                CallResult::Ok(serde_json::json!({"unixMs": 0, "utcOffsetMinutes": 0}))
            }
            CapabilityRequest::RandomBytes(r) => {
                let zeros = vec![0; r.length as usize];
                let reply = deckpilot_plugin_abi::capability::RandomReply::new(&zeros);
                CallResult::Ok(serde_json::to_value(reply).unwrap_or_default())
            }
            CapabilityRequest::DialogOpen(d) => CallResult::Ok(
                serde_json::to_value(crate::dialog::confirmed_with_defaults(d)).unwrap_or_default(),
            ),
            _ => CallResult::Ok(serde_json::Value::Null),
        })
    }

    /// [`MockHost::allow_all`] for a plugin shown in `locale` with the
    /// translations `messages` (flattened keys, plural forms as
    /// `key_one`…): `locale.get` answers `locale`, `locale.translate` looks
    /// the key up (the plural form for `count` by English or Polish rules),
    /// fills `{{name}}` and answers a label made from the key when it is
    /// missing, as the agent does.
    pub fn localized(locale: &str, messages: &[(&str, &str)]) -> Self {
        let locale = locale.to_owned();
        let messages: std::collections::BTreeMap<String, String> = messages
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let mut fallback = Self::allow_all();
        Self::new(move |req| match req {
            CapabilityRequest::LocaleGet(_) => {
                CallResult::Ok(serde_json::json!({"locale": locale}))
            }
            CapabilityRequest::LocaleTranslate(t) => {
                let count = t.params.get("count").and_then(|c| match c {
                    deckpilot_plugin_abi::locale::TranslateArg::Number(n) => n.as_i64(),
                    deckpilot_plugin_abi::locale::TranslateArg::Text(_) => None,
                });
                let language = locale.split(['-', '_']).next().unwrap_or_default();
                let found = count
                    .and_then(|n| {
                        messages
                            .get(&format!("{}_{}", t.key, plural(language, n)))
                            .or_else(|| messages.get(&format!("{}_other", t.key)))
                    })
                    .or_else(|| messages.get(&t.key));
                let (text, missing) = match found {
                    Some(template) => {
                        let mut text = template.clone();
                        for (name, value) in &t.params {
                            let value = match value {
                                deckpilot_plugin_abi::locale::TranslateArg::Text(s) => s.clone(),
                                deckpilot_plugin_abi::locale::TranslateArg::Number(n) => {
                                    n.to_string()
                                }
                            };
                            text = text.replace(&format!("{{{{{name}}}}}"), &value);
                        }
                        (text, false)
                    }
                    None => (humanize(&t.key), true),
                };
                CallResult::Ok(serde_json::json!({"text": text, "missing": missing}))
            }
            other => fallback.respond(other),
        })
    }

    fn respond(&mut self, request: &CapabilityRequest) -> CallResult {
        (self.respond)(request)
    }

    /// Every request is denied like an ungranted permission (logs too).
    pub fn deny_all() -> Self {
        Self::new(|_| CallResult::error(CallErrorCode::NotPermitted, "not granted"))
    }

    /// Requests made so far, in order.
    pub fn requests(&self) -> &[CapabilityRequest] {
        &self.requests
    }

    /// Runs one action as the host would. `params` is a JSON object.
    ///
    /// # Panics
    /// If `params` is not a valid parameter object.
    pub fn invoke(
        &mut self,
        plugin: &Plugin,
        action: &str,
        params: serde_json::Value,
    ) -> ActionOutcome {
        let params: Params =
            serde_json::from_value(params).expect("params must be a JSON object of values");
        self.invoke_params(plugin, action, &params)
    }

    /// A [`Context`] whose capability calls go to this mock, to test a
    /// helper that takes `&mut Context` without declaring an action.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CallResult, CapabilityRequest};
    ///
    /// let mut host = MockHost::new(|req| match req {
    ///     CapabilityRequest::StorageGet(_) => CallResult::Ok(serde_json::json!(7)),
    ///     _ => CallResult::Ok(serde_json::Value::Null),
    /// });
    /// let mut ctx = host.context();
    /// assert_eq!(ctx.storage_get("count").unwrap(), Some(serde_json::json!(7)));
    /// assert_eq!(host.requests().len(), 1);
    /// ```
    pub fn context(&mut self) -> Context<'_> {
        Context::new(self)
    }

    pub fn invoke_params(
        &mut self,
        plugin: &Plugin,
        action: &str,
        params: &Params,
    ) -> ActionOutcome {
        plugin.run(self, action, params)
    }

    /// Asks for the state of a key bound to `action` with `params` (a
    /// JSON object) as the host would ([`Action::state`](crate::Action::state)).
    /// The mock answers whatever the handler asks; the real agent answers
    /// only reads inside a state call, so answer the others with
    /// `not-permitted` to test that a handler copes.
    ///
    /// # Panics
    /// If `params` is not a valid parameter object.
    pub fn key_state(
        &mut self,
        plugin: &Plugin,
        action: &str,
        params: serde_json::Value,
    ) -> crate::StateOutcome {
        let params: Params =
            serde_json::from_value(params).expect("params must be a JSON object of values");
        plugin.key_state(self, action, &params)
    }
}

/// Every difference between a plugin's definition and its `manifest.json`
/// (parsed as JSON) that the agent would refuse or that would make an
/// action behave differently from its declaration: the id, the version,
/// `engines.pluginApi` and `engines.sdk` against this SDK, the action ids
/// (in order) and, per action, every parameter's key, `required`, kind,
/// bounds, options and default, and its `controls`
/// ([`Action::controls`](crate::Action::controls), compared per kind as
/// sets). A [`Param::text`](crate::Param::text)
/// matches the manifest kinds `text` (with `maxLength`), `project`,
/// `lines`, `pairs` and `code`, which all arrive as text; `number` and
/// `secret` parameters have no manifest kind in manifest version 1 and
/// are always reported. Empty means they agree. Use it in a unit test of
/// your plugin:
///
/// ```
/// use deckpilot_plugin_sdk::prelude::*;
/// use deckpilot_plugin_sdk::testing::manifest_mismatches;
///
/// fn greet(_: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
///     let name: String = args.get("name")?;
///     Ok(Output::message(format!("Hello, {name}.")))
/// }
/// let plugin = Plugin::new("acme.hello", "1.0.0")
///     .action(Action::new("greet", greet).param(Param::text("name").required().max_length(64)));
/// let manifest = serde_json::json!({
///     "id": "acme.hello", "version": "1.0.0",
///     "engines": {"deckpilot": ">=0.5.0 <1.0.0", "pluginApi": "^1.0"},
///     "contributes": {"actions": [{"id": "greet", "parameters": [
///         {"key": "name", "label": "Name", "required": true,
///          "kind": {"type": "text", "maxLength": 64}}
///     ]}]}
/// });
/// assert_eq!(manifest_mismatches(&plugin, &manifest), Vec::<String>::new());
///
/// let mut wrong = manifest.clone();
/// wrong["contributes"]["actions"][0]["parameters"][0]["required"] = false.into();
/// assert_eq!(manifest_mismatches(&plugin, &wrong), ["greet.name: required is false in the manifest, true in the plugin"]);
/// ```
pub fn manifest_mismatches(plugin: &Plugin, manifest: &serde_json::Value) -> Vec<String> {
    use deckpilot_plugin_abi::describe::ParamKind;
    use serde_json::{Value, json};

    let mut out = Vec::new();
    let mut differ = |what: String, manifest: &Value, plugin: &Value| {
        if manifest != plugin {
            out.push(format!(
                "{what} is {manifest} in the manifest, {plugin} in the plugin"
            ));
        }
    };
    differ("id".into(), &manifest["id"], &json!(plugin.id()));
    differ(
        "version".into(),
        &manifest["version"],
        &json!(plugin.version()),
    );
    differ(
        "engines.pluginApi".into(),
        &manifest["engines"]["pluginApi"],
        &json!(format!("^{}", plugin.api_level())),
    );
    if let Some(sdk) = manifest["engines"].get("sdk") {
        differ("engines.sdk".into(), sdk, &json!(crate::SDK_VERSION));
    }
    let declared = manifest["contributes"]["actions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let ids = |actions: &[Value]| -> Value { actions.iter().map(|a| a["id"].clone()).collect() };
    let plugin_ids: Value = plugin.actions().iter().map(|a| json!(a.id())).collect();
    differ("action ids".into(), &ids(&declared), &plugin_ids);
    for action in plugin.actions() {
        let Some(m) = declared.iter().find(|m| m["id"] == action.id()) else {
            continue;
        };
        let params = m["parameters"].as_array().cloned().unwrap_or_default();
        let keys: Value = params.iter().map(|p| p["key"].clone()).collect();
        let plugin_keys: Value = action.params().iter().map(|p| json!(p.key())).collect();
        differ(
            format!("{}: parameter keys", action.id()),
            &keys,
            &plugin_keys,
        );
        for p in action.params() {
            let Some(mp) = params.iter().find(|mp| mp["key"] == p.key()) else {
                continue;
            };
            let at = format!("{}.{}", action.id(), p.key());
            let kind = &mp["kind"];
            differ(
                format!("{at}: required"),
                &mp["required"],
                &json!(p.is_required()),
            );
            let (type_name, bounds): (&str, Vec<(&str, Value)>) = match p.kind() {
                // The manifest kinds the agent passes as text: `project`
                // (the project's id), `lines`, `pairs` and `code` (their
                // text). Their own bounds are the agent's to check.
                ParamKind::Text { .. }
                    if matches!(
                        kind["type"].as_str(),
                        Some("project" | "lines" | "pairs" | "code")
                    ) =>
                {
                    (kind["type"].as_str().unwrap_or_default(), vec![])
                }
                ParamKind::Text { max_length } => ("text", vec![("maxLength", json!(max_length))]),
                ParamKind::Integer { min, max } => {
                    ("integer", vec![("min", json!(min)), ("max", json!(max))])
                }
                ParamKind::Boolean {} => ("boolean", vec![]),
                ParamKind::Choice { options } => ("choice", vec![("options", json!(options))]),
                ParamKind::Number { .. } => ("number (no manifest kind in version 1)", vec![]),
                ParamKind::Secret {} => ("secret (no manifest kind in version 1)", vec![]),
            };
            differ(format!("{at}: kind.type"), &kind["type"], &json!(type_name));
            for (field, value) in bounds {
                differ(format!("{at}: kind.{field}"), &kind[field], &value);
            }
            let default = p
                .default_value()
                .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            differ(
                format!("{at}: default.value"),
                &mp["default"]["value"],
                &default,
            );
        }
        // Controls compare as sets per kind: the order in the manifest
        // does not matter. Absent on both sides is equal.
        let declared_controls = m.get("controls").filter(|c| !c.is_null());
        let normalized = declared_controls
            .map(|c| crate::Controls::from_json(c).map_or_else(|| c.clone(), |c| c.to_json()));
        let plugin_controls = action.declared_controls().map(crate::Controls::to_json);
        differ(
            format!("{}: controls", action.id()),
            normalized.as_ref().unwrap_or(&Value::Null),
            plugin_controls.as_ref().unwrap_or(&Value::Null),
        );
    }
    out
}

impl Transport for MockHost {
    fn call(&mut self, request: &CapabilityRequest) -> CallResult {
        self.requests.push(request.clone());
        (self.respond)(request)
    }
}

/// The plural category of `n` (English and Polish rules, as the agent).
fn plural(language: &str, n: i64) -> &'static str {
    let i = n.unsigned_abs();
    match (language, i) {
        (_, 1) => "one",
        ("pl", i) if (2..=4).contains(&(i % 10)) && !(12..=14).contains(&(i % 100)) => "few",
        ("pl", _) => "many",
        _ => "other",
    }
}

/// A label from a key's last segment (`settings.apiToken` → `Api token`),
/// as the agent shows a missing key.
fn humanize(key: &str) -> String {
    let segment = key.rsplit('.').next().unwrap_or(key);
    let mut words = Vec::new();
    let mut word = String::new();
    let mut previous_lower = false;
    for c in segment.chars() {
        if c == '_' || c == '-' {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            previous_lower = false;
            continue;
        }
        if c.is_uppercase() && previous_lower && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        previous_lower = c.is_lowercase() || c.is_ascii_digit();
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    let text = words.join(" ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "?".to_owned(),
    }
}
