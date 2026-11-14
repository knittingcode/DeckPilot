//! Declaring a plugin: metadata, actions, parameters, lifecycle hooks.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::OnceLock;

use deckpilot_plugin_abi::capability::is_credential_key;
use deckpilot_plugin_abi::describe::{
    ActionDescriptor, MAX_ACTIONS, MAX_OPTION_LEN, MAX_OPTIONS, MAX_PARAMS, ParamDescriptor,
    ParamKind, PluginDescriptor,
};
use deckpilot_plugin_abi::{ActionOutcome, ParamValue, Params};

use crate::args::{Args, normalize};
use crate::context::{Context, Transport};
use crate::output::{ActionResult, IntoValue, PluginError, codes};
use crate::{API_LEVEL, SDK_NAME, SDK_VERSION};

/// An action handler. Plain functions and non-capturing closures coerce to
/// it.
pub type Handler = fn(&mut Context<'_>, &Args<'_>) -> ActionResult;

/// The init hook: no capabilities (there is no invocation yet).
pub type InitHook = fn() -> Result<(), PluginError>;

/// The shutdown hook: no capabilities, best effort.
pub type ShutdownHook = fn();

/// A plugin: metadata plus its actions and hooks.
///
/// `id` and `version` must equal `manifest.json`'s `id` and `version`, and
/// the action ids and parameters must equal its `contributes.actions`; the
/// agent compares the module's descriptor with the manifest and refuses a
/// mismatch. Check it in a unit test with [`Plugin::validate`] and
/// [`Plugin::descriptor`].
pub struct Plugin {
    id: String,
    version: String,
    actions: Vec<Action>,
    init: Option<InitHook>,
    shutdown: Option<ShutdownHook>,
    init_state: OnceLock<Result<(), PluginError>>,
}

impl fmt::Debug for Plugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Plugin")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("actions", &self.actions)
            .finish_non_exhaustive()
    }
}

impl Plugin {
    /// `id` is `<publisher>.<name>` and `version` a semver, both exactly as
    /// in `manifest.json`.
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            actions: Vec::new(),
            init: None,
            shutdown: None,
            init_state: OnceLock::new(),
        }
    }

    pub fn action(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    /// Runs once before the first invocation. An error refuses the load.
    pub fn on_init(mut self, hook: InitHook) -> Self {
        self.init = Some(hook);
        self
    }

    /// Runs once when the host shuts down cleanly (not after a crash or a
    /// kill).
    pub fn on_shutdown(mut self, hook: ShutdownHook) -> Self {
        self.shutdown = Some(hook);
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    pub fn find(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    /// What `dp_describe` reports: SDK, API level, actions and parameters.
    /// No plugin id (identity comes from the channel).
    pub fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            sdk: format!("{SDK_NAME}/{SDK_VERSION}"),
            api_level: API_LEVEL.to_string(),
            actions: self.actions.iter().map(Action::descriptor).collect(),
        }
    }

    /// Checks the definition against the manifest's rules (ids, keys,
    /// uniqueness, kinds, defaults). Call it in a unit test.
    pub fn validate(&self) -> Result<(), DefinitionError> {
        if !is_plugin_id(&self.id) {
            return Err(DefinitionError::PluginId(self.id.clone()));
        }
        if !is_semver(&self.version) {
            return Err(DefinitionError::Version(self.version.clone()));
        }
        if self.actions.len() > MAX_ACTIONS {
            return Err(DefinitionError::TooManyActions);
        }
        let mut ids = BTreeSet::new();
        for a in &self.actions {
            if !is_local_id(&a.id) {
                return Err(DefinitionError::ActionId(a.id.clone()));
            }
            if !ids.insert(a.id.as_str()) {
                return Err(DefinitionError::DuplicateAction(a.id.clone()));
            }
            a.validate()?;
        }
        // The wire form must pass the host's check too.
        self.descriptor()
            .validate()
            .map_err(|e| DefinitionError::Descriptor(e.to_string()))
    }

    /// Runs the init hook once; later calls return the first result.
    pub(crate) fn ensure_init(&self) -> Result<(), PluginError> {
        self.init_state
            .get_or_init(|| self.init.map_or(Ok(()), |hook| hook()))
            .clone()
    }

    pub(crate) fn run_shutdown(&self) {
        if let Some(hook) = self.shutdown {
            hook();
        }
    }

    /// One invocation: init (if not done), find the action, check the
    /// parameters, run the handler, check the result.
    pub(crate) fn run(
        &self,
        transport: &mut dyn Transport,
        action: &str,
        params: &Params,
    ) -> ActionOutcome {
        if let Err(e) = self.validate() {
            return ActionOutcome::failed(codes::PLUGIN_ERROR, format!("invalid plugin: {e}"));
        }
        if let Err(e) = self.ensure_init() {
            return ActionOutcome::failed(
                codes::PLUGIN_ERROR,
                format!("init failed: {} ({})", e.message(), e.code()),
            );
        }
        let Some(a) = self.find(action) else {
            return ActionOutcome::failed(
                codes::UNSUPPORTED,
                format!("this plugin has no action `{}`", truncate(action, 64)),
            );
        };
        let args = match normalize(a, params) {
            Ok(args) => args,
            Err(e) => return e.into_outcome(),
        };
        let mut ctx = Context::new(transport);
        let outcome = match (a.handler)(&mut ctx, &args) {
            Ok(out) => out.into_outcome(),
            Err(e) => e.into_outcome(),
        };
        match outcome.validate() {
            Ok(()) => outcome,
            Err(e) => ActionOutcome::failed(
                codes::INVALID_RESULT,
                format!("action `{}` returned an invalid result: {e}", a.id),
            ),
        }
    }
}

/// One action: an id from the manifest, its parameters and a handler.
#[derive(Clone)]
pub struct Action {
    id: String,
    params: Vec<Param>,
    handler: Handler,
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Action")
            .field("id", &self.id)
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

impl Action {
    /// `id` is the manifest's action id (`[a-z][a-z0-9]*(-[a-z0-9]+)*`).
    pub fn new(id: impl Into<String>, handler: Handler) -> Self {
        Self {
            id: id.into(),
            params: Vec::new(),
            handler,
        }
    }

    pub fn param(mut self, p: Param) -> Self {
        self.params.push(p);
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn params(&self) -> &[Param] {
        &self.params
    }

    pub(crate) fn find(&self, key: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.key == key)
    }

    fn descriptor(&self) -> ActionDescriptor {
        ActionDescriptor {
            id: self.id.clone(),
            params: self.params.iter().map(Param::descriptor).collect(),
        }
    }

    fn validate(&self) -> Result<(), DefinitionError> {
        if self.params.len() > MAX_PARAMS {
            return Err(DefinitionError::TooManyParams(self.id.clone()));
        }
        let mut keys = BTreeSet::new();
        for p in &self.params {
            let at = || format!("{}.{}", self.id, p.key);
            if !is_camel_key(&p.key) {
                return Err(DefinitionError::ParamKey(at()));
            }
            if !keys.insert(p.key.as_str()) {
                return Err(DefinitionError::DuplicateParam(at()));
            }
            if let Some(misuse) = p.misuse {
                return Err(DefinitionError::Misuse(at(), misuse));
            }
            p.validate_kind()
                .map_err(|why| DefinitionError::Kind(at(), why))?;
            if let Some(d) = &p.default {
                if p.required {
                    return Err(DefinitionError::Default(at(), "a required parameter"));
                }
                crate::args::check_value(p, d)
                    .map_err(|_| DefinitionError::Default(at(), "does not match the kind"))?;
            }
        }
        Ok(())
    }
}

/// One typed parameter. Optional unless [`Param::required`].
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub(crate) key: String,
    pub(crate) kind: ParamKind,
    pub(crate) required: bool,
    pub(crate) default: Option<ParamValue>,
    misuse: Option<&'static str>,
}

impl Param {
    fn of(key: impl Into<String>, kind: ParamKind) -> Self {
        Self {
            key: key.into(),
            kind,
            required: false,
            default: None,
            misuse: None,
        }
    }

    /// Text (`ParamValue::String`).
    pub fn text(key: impl Into<String>) -> Self {
        Self::of(key, ParamKind::Text { max_length: None })
    }

    /// A whole number (`ParamValue::Integer`).
    pub fn integer(key: impl Into<String>) -> Self {
        Self::of(
            key,
            ParamKind::Integer {
                min: None,
                max: None,
            },
        )
    }

    /// A finite number; integers are accepted and widened.
    pub fn number(key: impl Into<String>) -> Self {
        Self::of(
            key,
            ParamKind::Number {
                min: None,
                max: None,
            },
        )
    }

    pub fn boolean(key: impl Into<String>) -> Self {
        Self::of(key, ParamKind::Boolean {})
    }

    /// One of `options` (text).
    pub fn choice(key: impl Into<String>, options: &[&str]) -> Self {
        Self::of(
            key,
            ParamKind::Choice {
                options: options.iter().map(|o| (*o).to_owned()).collect(),
            },
        )
    }

    /// A credential slot key (read with [`Args::get`] as a
    /// [`SecretHandle`](crate::SecretHandle)). The value is the slot's
    /// key, never the secret.
    pub fn secret(key: impl Into<String>) -> Self {
        Self::of(key, ParamKind::Secret {})
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// The value used when the parameter is absent (optional parameters
    /// only; checked against the kind by [`Plugin::validate`]).
    pub fn default(mut self, value: impl IntoValue) -> Self {
        self.default = Some(value.into_value());
        self
    }

    /// Longest text, in characters (`text` only).
    pub fn max_length(mut self, n: u32) -> Self {
        match &mut self.kind {
            ParamKind::Text { max_length } => *max_length = Some(n),
            _ => self.misuse = Some("max_length applies to text"),
        }
        self
    }

    /// Smallest value (`integer`: must be whole; `number`).
    pub fn min(mut self, v: f64) -> Self {
        self.set_bound(v, true);
        self
    }

    /// Largest value (`integer`: must be whole; `number`).
    pub fn max(mut self, v: f64) -> Self {
        self.set_bound(v, false);
        self
    }

    fn set_bound(&mut self, v: f64, is_min: bool) {
        match &mut self.kind {
            ParamKind::Integer { min, max } => {
                if v.fract() != 0.0 || !v.is_finite() || v.abs() > 9.0e15 {
                    self.misuse = Some("integer bounds must be whole numbers");
                } else if is_min {
                    *min = Some(v as i64);
                } else {
                    *max = Some(v as i64);
                }
            }
            ParamKind::Number { min, max } => {
                if is_min {
                    *min = Some(v);
                } else {
                    *max = Some(v);
                }
            }
            _ => self.misuse = Some("min and max apply to integer and number"),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn kind(&self) -> &ParamKind {
        &self.kind
    }

    pub fn is_required(&self) -> bool {
        self.required
    }

    pub fn default_value(&self) -> Option<&ParamValue> {
        self.default.as_ref()
    }

    fn descriptor(&self) -> ParamDescriptor {
        ParamDescriptor {
            key: self.key.clone(),
            required: self.required,
            kind: self.kind.clone(),
            default: self.default.clone(),
        }
    }

    fn validate_kind(&self) -> Result<(), &'static str> {
        match &self.kind {
            ParamKind::Text {
                max_length: Some(0),
            } => Err("max_length must be at least 1"),
            ParamKind::Integer {
                min: Some(lo),
                max: Some(hi),
            } if lo > hi => Err("min is greater than max"),
            ParamKind::Number { min, max } => {
                if min.is_some_and(|v| !v.is_finite()) || max.is_some_and(|v| !v.is_finite()) {
                    Err("bounds must be finite")
                } else if matches!((min, max), (Some(lo), Some(hi)) if lo > hi) {
                    Err("min is greater than max")
                } else {
                    Ok(())
                }
            }
            ParamKind::Choice { options } => {
                let unique: BTreeSet<&String> = options.iter().collect();
                if options.is_empty() || options.len() > MAX_OPTIONS {
                    Err("a choice needs 1 to 64 options")
                } else if unique.len() != options.len() {
                    Err("choice options must be unique")
                } else if options
                    .iter()
                    .any(|o| o.is_empty() || o.chars().count() > MAX_OPTION_LEN)
                {
                    Err("choice options must be 1 to 64 characters")
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
}

/// A plugin definition breaks a manifest rule. Names what and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionError {
    PluginId(String),
    Version(String),
    TooManyActions,
    ActionId(String),
    DuplicateAction(String),
    TooManyParams(String),
    ParamKey(String),
    DuplicateParam(String),
    Misuse(String, &'static str),
    Kind(String, &'static str),
    Default(String, &'static str),
    Descriptor(String),
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PluginId(id) => write!(f, "plugin id `{id}` is not <publisher>.<name>"),
            Self::Version(v) => write!(f, "version `{v}` is not a release semver"),
            Self::TooManyActions => write!(f, "more than {MAX_ACTIONS} actions"),
            Self::ActionId(id) => write!(f, "action id `{id}` is not a manifest action id"),
            Self::DuplicateAction(id) => write!(f, "action `{id}` is declared twice"),
            Self::TooManyParams(id) => {
                write!(f, "action `{id}` has more than {MAX_PARAMS} parameters")
            }
            Self::ParamKey(at) => write!(f, "parameter `{at}`: key is not camelCase"),
            Self::DuplicateParam(at) => write!(f, "parameter `{at}` is declared twice"),
            Self::Misuse(at, why) | Self::Kind(at, why) => write!(f, "parameter `{at}`: {why}"),
            Self::Default(at, why) => write!(f, "parameter `{at}`: default for {why}"),
            Self::Descriptor(why) => write!(f, "descriptor: {why}"),
        }
    }
}

impl std::error::Error for DefinitionError {}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `[a-z][a-z0-9]*(-[a-z][a-z0-9]*)*`, 2..=max characters.
fn is_kebab(s: &str, min: usize, max: usize, letter_after_hyphen: bool) -> bool {
    let len_ok = (min..=max).contains(&s.len());
    let mut prev_hyphen = true;
    let chars_ok = s.chars().enumerate().all(|(i, c)| {
        let ok = match c {
            'a'..='z' => true,
            '0'..='9' => i > 0 && !(prev_hyphen && letter_after_hyphen),
            '-' => i > 0 && !prev_hyphen,
            _ => false,
        };
        prev_hyphen = c == '-';
        ok
    });
    len_ok && chars_ok && !s.ends_with('-')
}

/// The manifest's `pluginId`: `<publisher>.<name>`.
pub(crate) fn is_plugin_id(id: &str) -> bool {
    id.len() <= 57
        && id
            .split_once('.')
            .is_some_and(|(p, n)| is_kebab(p, 2, 24, true) && is_kebab(n, 2, 32, true))
}

/// The manifest's `localId` (action ids).
pub(crate) fn is_local_id(id: &str) -> bool {
    is_kebab(id, 1, 48, false)
}

/// The manifest's `camelKey` (parameter keys).
pub(crate) fn is_camel_key(key: &str) -> bool {
    is_credential_key(key)
}

/// The manifest's `semver`: `X.Y.Z` with an optional `-alpha.N`,
/// `-beta.N` or `-rc.N`.
pub(crate) fn is_semver(v: &str) -> bool {
    let num = |s: &str| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))
    };
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let pre_ok = pre.is_none_or(|p| {
        p.split_once('.')
            .is_some_and(|(tag, n)| matches!(tag, "alpha" | "beta" | "rc") && num(n) && n != "0")
    });
    v.len() <= 32 && parts.len() == 3 && parts.iter().all(|p| num(p)) && pre_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop(_: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
        Ok(crate::Output::ok())
    }

    #[test]
    fn manifest_patterns() {
        for ok in [
            "acme.weather",
            "ab.cd",
            "acme-tools.hello-weather2",
            "a1.b2",
        ] {
            assert!(is_plugin_id(ok), "{ok}");
        }
        for bad in [
            "acme",
            "a.weather",
            "Acme.weather",
            "acme.-x",
            "acme.x-",
            "acme.a--b",
            "acme.a-1",
        ] {
            assert!(!is_plugin_id(bad), "{bad}");
        }
        for ok in ["greet", "current-weather", "v2-api", "a"] {
            assert!(is_local_id(ok), "{ok}");
        }
        for bad in ["", "Greet", "-a", "a-", "a_b", "a.b", "1a"] {
            assert!(!is_local_id(bad), "{bad}");
        }
        for ok in ["0.1.0", "1.20.3", "1.0.0-rc.1", "2.0.0-alpha.12"] {
            assert!(is_semver(ok), "{ok}");
        }
        for bad in [
            "1.0",
            "01.0.0",
            "1.0.0-rc.0",
            "1.0.0-dev.1",
            "1.0.0+b",
            "v1.0.0",
        ] {
            assert!(!is_semver(bad), "{bad}");
        }
    }

    #[test]
    fn definitions_are_checked() {
        let p = |a: Action| Plugin::new("acme.xy", "1.0.0").action(a);
        let err = |a: Action| p(a).validate().unwrap_err().to_string();
        assert!(
            p(Action::new("a", noop).param(Param::text("name").required()))
                .validate()
                .is_ok()
        );
        assert!(err(Action::new("A", noop)).contains("action id"));
        assert!(err(Action::new("a", noop).param(Param::text("Name"))).contains("camelCase"));
        assert!(
            err(Action::new("a", noop)
                .param(Param::text("n"))
                .param(Param::integer("n")))
            .contains("twice")
        );
        assert!(
            err(Action::new("a", noop).param(Param::boolean("b").max_length(3))).contains("text")
        );
        assert!(err(Action::new("a", noop).param(Param::integer("i").min(0.5))).contains("whole"));
        assert!(
            err(Action::new("a", noop).param(Param::number("n").min(2.0).max(1.0)))
                .contains("greater")
        );
        assert!(err(Action::new("a", noop).param(Param::choice("c", &[]))).contains("options"));
        assert!(
            err(Action::new("a", noop).param(Param::choice("c", &["x"]).default("y")))
                .contains("default")
        );
        assert!(
            err(Action::new("a", noop).param(Param::integer("i").max(3.0).default(4)))
                .contains("default")
        );
        assert!(
            err(Action::new("a", noop).param(Param::text("t").required().default("x")))
                .contains("required")
        );
        assert!(
            Plugin::new("acme", "1.0.0")
                .validate()
                .unwrap_err()
                .to_string()
                .contains("plugin id")
        );
        assert!(Plugin::new("acme.xy", "1.0").validate().is_err());
        let twice = Plugin::new("acme.xy", "1.0.0")
            .action(Action::new("a", noop))
            .action(Action::new("a", noop));
        assert_eq!(
            twice.validate(),
            Err(DefinitionError::DuplicateAction("a".into()))
        );
    }

    #[test]
    fn descriptor_lists_actions_and_params() {
        let plugin = Plugin::new("acme.xy", "1.0.0").action(
            Action::new("greet", noop)
                .param(Param::text("name").required().max_length(10))
                .param(Param::choice("style", &["a", "b"]).default("a"))
                .param(Param::secret("token")),
        );
        let d = plugin.descriptor();
        assert_eq!(d.sdk, format!("deckpilot-plugin-sdk/{SDK_VERSION}"));
        assert_eq!(d.api_level, "1.0");
        d.validate().unwrap();
        let params = &d.actions[0].params;
        assert_eq!(
            params[0].kind,
            ParamKind::Text {
                max_length: Some(10)
            }
        );
        assert!(params[0].required);
        assert_eq!(params[1].default, Some(ParamValue::String("a".into())));
        assert_eq!(params[2].kind, ParamKind::Secret {});
        let json = serde_json::to_string(&d).unwrap();
        assert!(
            !json.contains("acme.xy"),
            "no plugin id on the wire: {json}"
        );
    }
}
