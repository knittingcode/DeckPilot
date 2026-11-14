//! Typed parameters and action outcomes.
//!
//! Parameters are normalized literals (design §6.2): the agent has already
//! resolved references and templates. JSON `null` and objects are not
//! values; a list nests at most [`MAX_VALUE_DEPTH`] levels.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::limits::{
    MAX_CODE_LEN, MAX_LIST_LEN, MAX_MAP_ENTRIES, MAX_MESSAGE_LEN, MAX_NAME_LEN, MAX_STRING_LEN,
    MAX_VALUE_DEPTH,
};

/// One parameter or result value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    /// Listed before `Number` so that JSON integers stay integers.
    Integer(i64),
    Number(f64),
    String(String),
    List(Vec<ParamValue>),
}

/// Named values: action parameters, or the data of a successful outcome.
pub type Params = BTreeMap<String, ParamValue>;

/// The result of one action invocation, as produced by the guest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum ActionOutcome {
    Ok {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        data: Params,
    },
    Skipped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Failed {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default)]
        retryable: bool,
    },
}

impl ActionOutcome {
    pub fn ok() -> Self {
        Self::Ok {
            message: None,
            data: Params::new(),
        }
    }

    pub fn ok_with(data: Params) -> Self {
        Self::Ok {
            message: None,
            data,
        }
    }

    pub fn failed(code: &str, message: impl Into<String>) -> Self {
        Self::Failed {
            code: code.to_owned(),
            message: Some(message.into()),
            retryable: false,
        }
    }

    /// Checks every bound of ABI 1. The host applies this to guest output
    /// and reports `invalid-result` when it fails.
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Ok { message, data } => {
                validate_message(message.as_deref())?;
                validate_params(data)
            }
            Self::Skipped { message } => validate_message(message.as_deref()),
            Self::Failed { code, message, .. } => {
                validate_code(code)?;
                validate_message(message.as_deref())
            }
        }
    }
}

/// Why a value broke a bound. Never contains the value itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    TooManyEntries,
    InvalidName,
    StringTooLong,
    ListTooLong,
    TooDeep,
    NonFiniteNumber,
    MessageTooLong,
    InvalidCode,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::TooManyEntries => "too many entries",
            Self::InvalidName => "invalid name",
            Self::StringTooLong => "string too long",
            Self::ListTooLong => "list too long",
            Self::TooDeep => "value nested too deeply",
            Self::NonFiniteNumber => "number is not finite",
            Self::MessageTooLong => "message too long",
            Self::InvalidCode => "invalid code",
        };
        f.write_str(s)
    }
}

impl std::error::Error for ValidationError {}

/// Action names and map keys: `[A-Za-z][A-Za-z0-9._-]{0,63}`.
pub fn validate_name(name: &str) -> Result<(), ValidationError> {
    let mut chars = name.chars();
    let ok = name.len() <= MAX_NAME_LEN
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(ValidationError::InvalidName)
    }
}

/// Error codes: `[a-z][a-z0-9-]{0,63}`.
pub fn validate_code(code: &str) -> Result<(), ValidationError> {
    let mut chars = code.chars();
    let ok = code.len() <= MAX_CODE_LEN
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(ValidationError::InvalidCode)
    }
}

fn validate_message(message: Option<&str>) -> Result<(), ValidationError> {
    match message {
        Some(m) if m.chars().count() > MAX_MESSAGE_LEN => Err(ValidationError::MessageTooLong),
        _ => Ok(()),
    }
}

/// Checks a parameter or data map.
pub fn validate_params(params: &Params) -> Result<(), ValidationError> {
    if params.len() > MAX_MAP_ENTRIES {
        return Err(ValidationError::TooManyEntries);
    }
    for (key, value) in params {
        validate_name(key)?;
        validate_value(value, 1)?;
    }
    Ok(())
}

fn validate_value(value: &ParamValue, depth: usize) -> Result<(), ValidationError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(ValidationError::TooDeep);
    }
    match value {
        ParamValue::Bool(_) | ParamValue::Integer(_) => Ok(()),
        ParamValue::Number(n) if n.is_finite() => Ok(()),
        ParamValue::Number(_) => Err(ValidationError::NonFiniteNumber),
        ParamValue::String(s) if s.len() > MAX_STRING_LEN => Err(ValidationError::StringTooLong),
        ParamValue::String(_) => Ok(()),
        ParamValue::List(items) if items.len() > MAX_LIST_LEN => Err(ValidationError::ListTooLong),
        ParamValue::List(items) => items.iter().try_for_each(|v| validate_value(v, depth + 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(json: &str) -> Result<Params, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn values_keep_their_types() {
        let p = params(r#"{"a":1,"b":1.5,"c":"x","d":true,"e":[1,["y"]]}"#).unwrap();
        assert_eq!(p["a"], ParamValue::Integer(1));
        assert_eq!(p["b"], ParamValue::Number(1.5));
        assert_eq!(p["c"], ParamValue::String("x".into()));
        assert_eq!(p["d"], ParamValue::Bool(true));
        validate_params(&p).unwrap();
        let back = serde_json::to_string(&p).unwrap();
        assert_eq!(params(&back).unwrap(), p);
    }

    #[test]
    fn null_and_objects_are_not_values() {
        assert!(params(r#"{"a":null}"#).is_err());
        assert!(params(r#"{"a":{"b":1}}"#).is_err());
        assert!(params(r#"{"a":[{"b":1}]}"#).is_err());
    }

    #[test]
    fn bounds_are_enforced() {
        let deep = params(r#"{"a":[[[[1]]]]}"#).unwrap();
        assert_eq!(validate_params(&deep), Err(ValidationError::TooDeep));
        let long = Params::from([(
            "a".into(),
            ParamValue::String("x".repeat(MAX_STRING_LEN + 1)),
        )]);
        assert_eq!(validate_params(&long), Err(ValidationError::StringTooLong));
        let list = Params::from([(
            "a".into(),
            ParamValue::List(vec![ParamValue::Bool(true); 257]),
        )]);
        assert_eq!(validate_params(&list), Err(ValidationError::ListTooLong));
        let many: Params = (0..65)
            .map(|i| (format!("k{i}"), ParamValue::Integer(i)))
            .collect();
        assert_eq!(validate_params(&many), Err(ValidationError::TooManyEntries));
        let nan = Params::from([("a".into(), ParamValue::Number(f64::NAN))]);
        assert_eq!(validate_params(&nan), Err(ValidationError::NonFiniteNumber));
        for bad in ["", "1a", "a b", "a/b", "ä", &"a".repeat(65)] {
            assert_eq!(
                validate_name(bad),
                Err(ValidationError::InvalidName),
                "{bad:?}"
            );
        }
        validate_name("weather.current_temp-2").unwrap();
    }

    #[test]
    fn outcomes_are_strict() {
        let ok: ActionOutcome = serde_json::from_str(r#"{"status":"ok","data":{"t":21}}"#).unwrap();
        ok.validate().unwrap();
        let failed: ActionOutcome =
            serde_json::from_str(r#"{"status":"failed","code":"rate-limited"}"#).unwrap();
        failed.validate().unwrap();
        for bad in [
            r#"{"status":"ok","extra":1}"#,
            r#"{"status":"done"}"#,
            r#"{"status":"failed"}"#,
            r#"{"data":{}}"#,
        ] {
            assert!(serde_json::from_str::<ActionOutcome>(bad).is_err(), "{bad}");
        }
        for code in ["", "Bad", "-x", "a_b", "über"] {
            let o = ActionOutcome::failed(code, "m");
            assert_eq!(o.validate(), Err(ValidationError::InvalidCode), "{code:?}");
        }
        let long = ActionOutcome::Skipped {
            message: Some("x".repeat(MAX_MESSAGE_LEN + 1)),
        };
        assert_eq!(long.validate(), Err(ValidationError::MessageTooLong));
    }
}
