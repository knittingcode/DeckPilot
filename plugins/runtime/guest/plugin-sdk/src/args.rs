//! Checked, typed parameter access.

use deckpilot_plugin_abi::describe::ParamKind;
use deckpilot_plugin_abi::{ParamValue, Params};

use crate::context::SecretHandle;
use crate::definition::{Action, Param, is_camel_key};
use crate::output::{PluginError, codes};

/// The parameters of one invocation, already checked against the action's
/// declaration: every key is declared, every value has its declared kind
/// and bounds, defaults are filled in and required parameters are present.
#[derive(Debug)]
pub struct Args<'a> {
    action: &'a Action,
    values: Params,
}

impl Args<'_> {
    /// Reads a parameter as `T`: `String` (text, choice), `i64` (integer),
    /// `f64` (number, integer), `bool` (boolean), [`SecretHandle`]
    /// (secret), or `Option<T>` of those for optional parameters.
    ///
    /// Errors: [`codes::INVALID_PARAMETERS`] if a required value is absent
    /// (an optional parameter read as `T` rather than `Option<T>`);
    /// [`codes::PLUGIN_ERROR`] if `key` is not declared or `T` does not fit
    /// the declared kind (a bug in the plugin, not in the invocation).
    pub fn get<T: FromArg>(&self, key: &str) -> Result<T, PluginError> {
        let Some(param) = self.action.find(key) else {
            return Err(PluginError::new(
                codes::PLUGIN_ERROR,
                format!(
                    "action `{}` reads undeclared parameter `{key}`",
                    self.action.id()
                ),
            ));
        };
        if !T::fits(&param.kind) {
            return Err(PluginError::new(
                codes::PLUGIN_ERROR,
                format!(
                    "parameter `{key}` of action `{}` is {}, not {}",
                    self.action.id(),
                    kind_name(&param.kind),
                    T::EXPECTS
                ),
            ));
        }
        T::from_arg(key, self.values.get(key))
    }

    /// Whether the invocation (or a default) supplied `key`.
    pub fn has(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// The checked values, defaults included.
    pub fn values(&self) -> &Params {
        &self.values
    }
}

/// A Rust type a parameter can be read as. See [`Args::get`].
pub trait FromArg: Sized {
    /// For error messages.
    const EXPECTS: &'static str;
    fn fits(kind: &ParamKind) -> bool;
    fn from_value(value: &ParamValue) -> Option<Self>;

    fn from_arg(key: &str, value: Option<&ParamValue>) -> Result<Self, PluginError> {
        let value = value.ok_or_else(|| {
            PluginError::new(
                codes::INVALID_PARAMETERS,
                format!("parameter `{key}` is missing"),
            )
        })?;
        Self::from_value(value).ok_or_else(|| {
            PluginError::new(
                codes::INVALID_PARAMETERS,
                format!("parameter `{key}` is not {}", Self::EXPECTS),
            )
        })
    }
}

impl FromArg for String {
    const EXPECTS: &'static str = "text";
    fn fits(kind: &ParamKind) -> bool {
        matches!(kind, ParamKind::Text { .. } | ParamKind::Choice { .. })
    }
    fn from_value(value: &ParamValue) -> Option<Self> {
        match value {
            ParamValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }
}

impl FromArg for i64 {
    const EXPECTS: &'static str = "an integer";
    fn fits(kind: &ParamKind) -> bool {
        matches!(kind, ParamKind::Integer { .. })
    }
    fn from_value(value: &ParamValue) -> Option<Self> {
        match value {
            ParamValue::Integer(i) => Some(*i),
            _ => None,
        }
    }
}

impl FromArg for f64 {
    const EXPECTS: &'static str = "a number";
    fn fits(kind: &ParamKind) -> bool {
        matches!(kind, ParamKind::Number { .. } | ParamKind::Integer { .. })
    }
    fn from_value(value: &ParamValue) -> Option<Self> {
        match value {
            ParamValue::Number(n) => Some(*n),
            ParamValue::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl FromArg for bool {
    const EXPECTS: &'static str = "a boolean";
    fn fits(kind: &ParamKind) -> bool {
        matches!(kind, ParamKind::Boolean {})
    }
    fn from_value(value: &ParamValue) -> Option<Self> {
        match value {
            ParamValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

impl FromArg for SecretHandle {
    const EXPECTS: &'static str = "a secret handle";
    fn fits(kind: &ParamKind) -> bool {
        matches!(kind, ParamKind::Secret {})
    }
    fn from_value(value: &ParamValue) -> Option<Self> {
        match value {
            ParamValue::String(s) if is_camel_key(s) => Some(SecretHandle::new_unchecked(s)),
            _ => None,
        }
    }
}

impl<T: FromArg> FromArg for Option<T> {
    const EXPECTS: &'static str = T::EXPECTS;
    fn fits(kind: &ParamKind) -> bool {
        T::fits(kind)
    }
    fn from_value(value: &ParamValue) -> Option<Self> {
        T::from_value(value).map(Some)
    }
    fn from_arg(key: &str, value: Option<&ParamValue>) -> Result<Self, PluginError> {
        match value {
            None => Ok(None),
            Some(_) => T::from_arg(key, value).map(Some),
        }
    }
}

fn kind_name(kind: &ParamKind) -> &'static str {
    match kind {
        ParamKind::Text { .. } => "text",
        ParamKind::Integer { .. } => "an integer",
        ParamKind::Number { .. } => "a number",
        ParamKind::Boolean {} => "a boolean",
        ParamKind::Choice { .. } => "a choice",
        ParamKind::Secret {} => "a secret handle",
    }
}

/// Checks one value against its declaration. The message names the
/// parameter and the rule, and quotes no value (it may be sensitive).
pub(crate) fn check_value(p: &Param, v: &ParamValue) -> Result<(), String> {
    let key = &p.key;
    let fail = |why: String| Err(format!("parameter `{key}` {why}"));
    match (&p.kind, v) {
        (ParamKind::Text { max_length }, ParamValue::String(s)) => match max_length {
            Some(max) if s.chars().count() > *max as usize => {
                fail(format!("is longer than {max} characters"))
            }
            _ => Ok(()),
        },
        (ParamKind::Integer { min, max }, ParamValue::Integer(i)) => {
            if min.is_some_and(|lo| *i < lo) || max.is_some_and(|hi| *i > hi) {
                fail(format!("must be within {}", range(min, max)))
            } else {
                Ok(())
            }
        }
        (ParamKind::Number { min, max }, ParamValue::Number(_) | ParamValue::Integer(_)) => {
            let n = match v {
                ParamValue::Number(n) => *n,
                ParamValue::Integer(i) => *i as f64,
                _ => unreachable!("matched above"),
            };
            if !n.is_finite() {
                fail("must be a finite number".into())
            } else if min.is_some_and(|lo| n < lo) || max.is_some_and(|hi| n > hi) {
                fail(format!("must be within {}", range(min, max)))
            } else {
                Ok(())
            }
        }
        (ParamKind::Boolean {}, ParamValue::Bool(_)) => Ok(()),
        (ParamKind::Choice { options }, ParamValue::String(s)) => {
            if options.iter().any(|o| o == s) {
                Ok(())
            } else {
                fail(format!("must be one of: {}", options.join(", ")))
            }
        }
        (ParamKind::Secret {}, ParamValue::String(s)) => {
            if is_camel_key(s) {
                Ok(())
            } else {
                fail("must name a credential slot".into())
            }
        }
        (kind, _) => fail(format!("must be {}", kind_name(kind))),
    }
}

fn range<T: std::fmt::Display>(min: &Option<T>, max: &Option<T>) -> String {
    let show = |b: &Option<T>, inf: &str| b.as_ref().map_or(inf.to_owned(), T::to_string);
    format!("[{}, {}]", show(min, "-inf"), show(max, "inf"))
}

/// Checks the raw parameters of one invocation against `action` and fills
/// defaults.
pub(crate) fn normalize<'a>(action: &'a Action, params: &Params) -> Result<Args<'a>, PluginError> {
    let invalid = |m: String| PluginError::new(codes::INVALID_PARAMETERS, m);
    if let Some(unknown) = params.keys().find(|k| action.find(k).is_none()) {
        let shown: String = unknown.chars().take(64).collect();
        return Err(invalid(format!(
            "action `{}` has no parameter `{shown}`",
            action.id()
        )));
    }
    let mut values = Params::new();
    for p in action.params() {
        match params.get(&p.key).or(p.default.as_ref()) {
            Some(v) => {
                check_value(p, v).map_err(invalid)?;
                values.insert(p.key.clone(), v.clone());
            }
            None if p.required => {
                return Err(invalid(format!("parameter `{}` is required", p.key)));
            }
            None => {}
        }
    }
    Ok(Args { action, values })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionResult, Context, Output};
    use serde_json::json;

    fn noop(_: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
        Ok(Output::ok())
    }

    fn action() -> Action {
        Action::new("a", noop)
            .param(Param::text("name").required().max_length(5))
            .param(Param::integer("count").min(1.0).max(3.0).default(1))
            .param(Param::number("ratio").min(0.0).max(1.0))
            .param(Param::boolean("loud"))
            .param(Param::choice("units", &["metric", "imperial"]).default("metric"))
            .param(Param::secret("token"))
    }

    fn params(v: serde_json::Value) -> Params {
        serde_json::from_value(v).unwrap()
    }

    fn message(a: &Action, v: serde_json::Value) -> String {
        let e = normalize(a, &params(v)).unwrap_err();
        assert_eq!(e.code(), codes::INVALID_PARAMETERS);
        e.message().to_owned()
    }

    #[test]
    fn values_are_typed_and_defaults_filled() {
        let a = action();
        let args = normalize(
            &a,
            &params(json!({"name": "Ada", "ratio": 1, "token": "apiKey"})),
        )
        .unwrap();
        assert_eq!(args.get::<String>("name").unwrap(), "Ada");
        assert_eq!(args.get::<i64>("count").unwrap(), 1);
        assert_eq!(args.get::<f64>("count").unwrap(), 1.0);
        assert_eq!(args.get::<f64>("ratio").unwrap(), 1.0);
        assert_eq!(args.get::<Option<bool>>("loud").unwrap(), None);
        assert_eq!(args.get::<String>("units").unwrap(), "metric");
        assert_eq!(args.get::<SecretHandle>("token").unwrap().key(), "apiKey");
        assert!(!args.has("loud"));
        // An optional parameter read as T: a typed error, not a panic.
        assert_eq!(
            args.get::<bool>("loud").unwrap_err().code(),
            codes::INVALID_PARAMETERS
        );
        // Plugin bugs are plugin errors.
        assert_eq!(
            args.get::<i64>("name").unwrap_err().code(),
            codes::PLUGIN_ERROR
        );
        assert_eq!(
            args.get::<String>("nope").unwrap_err().code(),
            codes::PLUGIN_ERROR
        );
    }

    #[test]
    fn malformed_params_are_typed_errors() {
        let a = action();
        assert!(message(&a, json!({})).contains("`name` is required"));
        assert!(message(&a, json!({"name": 5})).contains("must be text"));
        assert!(message(&a, json!({"name": "toolong"})).contains("longer than 5"));
        assert!(message(&a, json!({"name": "a", "count": 9})).contains("[1, 3]"));
        assert!(message(&a, json!({"name": "a", "count": 1.5})).contains("integer"));
        assert!(message(&a, json!({"name": "a", "ratio": 2.5})).contains("[0, 1]"));
        assert!(message(&a, json!({"name": "a", "loud": "yes"})).contains("boolean"));
        assert!(
            message(&a, json!({"name": "a", "units": "kelvin"}))
                .contains("one of: metric, imperial")
        );
        assert!(
            message(&a, json!({"name": "a", "token": "not a key"})).contains("credential slot")
        );
        assert!(message(&a, json!({"name": "a", "extra": 1})).contains("no parameter `extra`"));
        assert!(message(&a, json!({"name": ["a"]})).contains("must be text"));
        // Messages never quote the value.
        assert!(!message(&a, json!({"name": "s3cr3t-value"})).contains("s3cr3t"));
    }
}
