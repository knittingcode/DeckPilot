//! Structured results and typed errors.

use std::fmt;

use deckpilot_plugin_abi::{ActionOutcome, ParamValue, Params};

/// What a handler returns: [`Output`] on success (or skip), a
/// [`PluginError`] on failure.
pub type ActionResult = Result<Output, PluginError>;

/// Shared error codes (design §6.3 `SharedCode`): the agent produces them,
/// and plugins may return them without declaring them in `errorCodes`.
/// Plugin-specific codes must be declared in the manifest's
/// `contributes.errorCodes`.
pub mod codes {
    pub const NOT_PERMITTED: &str = "not-permitted";
    pub const INVALID_PARAMETERS: &str = "invalid-parameters";
    pub const CREDENTIAL_MISSING: &str = "credential-missing";
    pub const REQUEST_FAILED: &str = "request-failed";
    pub const INVALID_URL: &str = "invalid-url";
    pub const HOST_NOT_ALLOWED: &str = "host-not-allowed";
    pub const THROTTLED: &str = "throttled";
    pub const STORAGE_QUOTA: &str = "storage-quota";
    pub const TIMEOUT: &str = "timeout";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const PLUGIN_ERROR: &str = "plugin-error";
    pub const PLUGIN_UNAVAILABLE: &str = "plugin-unavailable";
    pub const INVALID_RESULT: &str = "invalid-result";

    /// Every shared code of plugin API 1.0.
    pub const SHARED: &[&str] = &[
        "not-permitted",
        "invalid-parameters",
        "no-active-project",
        "credential-missing",
        "reference-missing",
        "reference-type",
        "reference-denied",
        "blocked-address",
        "dns-failed",
        "connection-failed",
        "tls-failed",
        "too-many-redirects",
        "insecure-redirect",
        "insecure-credentials",
        "request-failed",
        "sensitive-header",
        "invalid-url",
        "host-not-allowed",
        "throttled",
        "storage-quota",
        "timeout",
        "unsupported",
        "plugin-error",
        "plugin-unavailable",
        "invalid-result",
    ];

    /// Whether `code` is a shared code.
    pub fn is_shared(code: &str) -> bool {
        SHARED.contains(&code)
    }
}

/// A successful (or skipped) result.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    skipped: bool,
    message: Option<String>,
    data: Params,
}

impl Output {
    /// Success without message or data.
    pub fn ok() -> Self {
        Self {
            skipped: false,
            message: None,
            data: Params::new(),
        }
    }

    /// Success with a message (at most 1,024 characters; never put secrets
    /// in it).
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            ..Self::ok()
        }
    }

    /// Nothing to do (the workflow step is skipped).
    pub fn skipped(message: impl Into<String>) -> Self {
        Self {
            skipped: true,
            message: Some(message.into()),
            data: Params::new(),
        }
    }

    /// Adds one output value. Keys should be the manifest's
    /// `node.outputs[].key`; the agent keeps only declared outputs.
    pub fn with(mut self, key: impl Into<String>, value: impl IntoValue) -> Self {
        self.data.insert(key.into(), value.into_value());
        self
    }

    pub fn data(&self) -> &Params {
        &self.data
    }

    pub(crate) fn into_outcome(self) -> ActionOutcome {
        if self.skipped {
            ActionOutcome::Skipped {
                message: self.message,
            }
        } else {
            ActionOutcome::Ok {
                message: self.message,
                data: self.data,
            }
        }
    }
}

/// A typed failure: a code (shared, or declared in the manifest's
/// `errorCodes`), a message for the user, and whether a retry may help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginError {
    code: String,
    message: String,
    retryable: bool,
}

impl PluginError {
    /// `code` matches `[a-z][a-z0-9]*(-[a-z0-9]+)*`; anything else is
    /// reported as `invalid-result`.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    /// Marks the failure as transient (honoured only for actions declared
    /// `retrySafe`).
    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    pub(crate) fn into_outcome(self) -> ActionOutcome {
        ActionOutcome::Failed {
            code: self.code,
            message: Some(self.message),
            retryable: self.retryable,
        }
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PluginError {}

/// Values an [`Output`] can carry.
pub trait IntoValue {
    fn into_value(self) -> ParamValue;
}

impl IntoValue for ParamValue {
    fn into_value(self) -> ParamValue {
        self
    }
}

impl IntoValue for String {
    fn into_value(self) -> ParamValue {
        ParamValue::String(self)
    }
}

impl IntoValue for &str {
    fn into_value(self) -> ParamValue {
        ParamValue::String(self.to_owned())
    }
}

impl IntoValue for &String {
    fn into_value(self) -> ParamValue {
        ParamValue::String(self.clone())
    }
}

impl IntoValue for bool {
    fn into_value(self) -> ParamValue {
        ParamValue::Bool(self)
    }
}

impl IntoValue for f64 {
    fn into_value(self) -> ParamValue {
        ParamValue::Number(self)
    }
}

impl IntoValue for f32 {
    fn into_value(self) -> ParamValue {
        ParamValue::Number(f64::from(self))
    }
}

macro_rules! int_values {
    ($($t:ty),*) => {$(
        impl IntoValue for $t {
            fn into_value(self) -> ParamValue {
                ParamValue::Integer(i64::from(self))
            }
        }
    )*};
}
int_values!(i8, i16, i32, i64, u8, u16, u32);

impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> ParamValue {
        ParamValue::List(self.into_iter().map(IntoValue::into_value).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_become_outcomes() {
        let o = Output::message("hi")
            .with("n", 3)
            .with("t", "x")
            .with("ok", true);
        let ActionOutcome::Ok { message, data } = o.into_outcome() else {
            panic!()
        };
        assert_eq!(message.as_deref(), Some("hi"));
        assert_eq!(data["n"], ParamValue::Integer(3));
        assert_eq!(data["t"], ParamValue::String("x".into()));
        assert_eq!(
            Output::skipped("later").into_outcome(),
            ActionOutcome::Skipped {
                message: Some("later".into())
            }
        );
        let e = PluginError::new("city-not-found", "no such city").retryable();
        assert_eq!(e.to_string(), "city-not-found: no such city");
        assert_eq!(
            e.into_outcome(),
            ActionOutcome::Failed {
                code: "city-not-found".into(),
                message: Some("no such city".into()),
                retryable: true
            }
        );
    }

    #[test]
    fn shared_codes_are_valid_codes() {
        for c in codes::SHARED {
            deckpilot_plugin_abi::value::validate_code(c).unwrap();
        }
        assert!(codes::is_shared(codes::NOT_PERMITTED));
        assert!(!codes::is_shared("city-not-found"));
    }
}
