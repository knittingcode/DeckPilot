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
    /// [`MockHost::new`] for a time or bytes of your choice).
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
            _ => CallResult::Ok(serde_json::Value::Null),
        })
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
}

/// Every difference between a plugin's definition and its `manifest.json`
/// (parsed as JSON) that the agent would refuse or that would make an
/// action behave differently from its declaration: the id, the version,
/// `engines.pluginApi` and `engines.sdk` against this SDK, the action ids
/// (in order) and, per action, every parameter's key, `required`, kind,
/// bounds, options and default. A [`Param::text`](crate::Param::text)
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
    }
    out
}

impl Transport for MockHost {
    fn call(&mut self, request: &CapabilityRequest) -> CallResult {
        self.requests.push(request.clone());
        (self.respond)(request)
    }
}
