//! Capability requests a guest makes through `host_call`, and their replies.
//!
//! The set of operations is closed: an operation that is not a variant of
//! [`CapabilityRequest`] does not parse. There is deliberately no file
//! system, process, clipboard or device-frame operation (design §5.2).
//! No request carries a plugin id; a `plugin` field anywhere is an unknown
//! field and the request is rejected.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::limits::{
    MAX_BODY_LEN, MAX_CREDENTIAL_KEY, MAX_HEADER_LEN, MAX_HEADERS, MAX_LOG_LEN, MAX_NOTIFY_BODY,
    MAX_NOTIFY_TITLE, MAX_STORAGE_KEY, MAX_STORAGE_VALUE, MAX_URL_LEN, MAX_VARIABLE_ID,
    MAX_VARIABLE_VALUE,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", content = "args", deny_unknown_fields)]
pub enum CapabilityRequest {
    #[serde(rename = "http.fetch")]
    HttpFetch(HttpFetch),
    #[serde(rename = "notify")]
    Notify(Notify),
    #[serde(rename = "log")]
    Log(Log),
    #[serde(rename = "storage.get")]
    StorageGet(StorageKey),
    #[serde(rename = "storage.set")]
    StorageSet(StorageSet),
    #[serde(rename = "storage.delete")]
    StorageDelete(StorageKey),
    /// Publishes (or, with a `null` value, clears) one of the plugin's own
    /// declared variables (design §2.3 item 3, §5.2 "own variables").
    /// Added compatibly within ABI 1: a host or agent that predates it
    /// refuses the unknown op, and no other request changes.
    #[serde(rename = "variables.publish")]
    VariablesPublish(VariablePublish),
    /// Clears one published variable, or all of the plugin's.
    #[serde(rename = "variables.clear")]
    VariablesClear(VariableClear),
    /// The plugin's own storage keys, optionally only those starting with
    /// `prefix`, sorted. Added compatibly within ABI 1.
    #[serde(rename = "storage.keys")]
    StorageKeys(StoragePrefix),
    /// How much of its storage quota the plugin uses ([`StorageUsage`]).
    /// Added compatibly within ABI 1.
    #[serde(rename = "storage.usage")]
    StorageUsage(NoArgs),
    /// Whether the user stored a secret for one of the plugin's credential
    /// slots: `true` or `false`, never the secret (design §5.5). Added
    /// compatibly within ABI 1.
    #[serde(rename = "credentials.has")]
    CredentialHas(CredentialRef),
    /// The plugin's settings (`contributes.settings`): an object of the
    /// normalized values, defaults filled in. Added compatibly within
    /// ABI 1.
    #[serde(rename = "settings.get")]
    SettingsGet(NoArgs),
}

/// A value for one of the plugin's own variables. `id` is relative to the
/// plugin's namespace (`temperature`, not
/// `plugins.acme.helloWeather.temperature`): a plugin can only ever name
/// its own variables, and the agent adds the namespace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariablePublish {
    pub id: String,
    /// The value, of the declared type; `null` clears it.
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariableClear {
    /// One variable (relative id), or every published one when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Checks a relative variable id: 1 to 3 dot-separated segments, each
/// `[a-z][A-Za-z0-9_]{0,31}` (the manifest's `contributes.variables[].id`).
pub fn is_variable_id(id: &str) -> bool {
    let segment = |s: &str| {
        let mut chars = s.chars();
        s.len() <= 32
            && chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    id.len() <= MAX_VARIABLE_ID
        && (1..=3).contains(&id.split('.').count())
        && id.split('.').all(segment)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpFetch {
    pub method: HttpMethod,
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<Header>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// A credential **handle**: the agent injects the secret of this slot
    /// (never visible to the guest) if the broker allows it for the
    /// request's origin and scheme (design §5.5). Added compatibly within
    /// ABI 1: optional, and omitted from the JSON when absent, so requests
    /// without it are byte-identical to the original ABI 1 form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<HttpAuth>,
}

/// How a credential handle is attached to a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpAuth {
    /// The credential slot key from the manifest (`[a-z][A-Za-z0-9]{0,31}`).
    pub credential: String,
    pub scheme: AuthScheme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthScheme {
    /// `Authorization: Bearer <secret>`.
    Bearer,
    /// `Authorization: Basic <secret>`.
    Basic,
    /// The slot's declared header name.
    Header,
    /// The slot's declared query parameter.
    Query,
}

/// Checks a credential slot key: `[a-z][A-Za-z0-9]{0,31}` (the manifest's
/// `camelKey`).
pub fn is_credential_key(key: &str) -> bool {
    let mut chars = key.chars();
    key.len() <= MAX_CREDENTIAL_KEY
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// The `ok` value of an allowed `http.fetch`, as the agent's executor
/// returns it. Unknown fields are **ignored** here (host to guest only), so
/// the agent can add fields within ABI 1 without breaking built guests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<Header>,
    /// The body as UTF-8 text (bodies that are not UTF-8 are not delivered
    /// in ABI 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notify {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Log {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageKey {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageSet {
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePrefix {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

/// Arguments of an operation that takes none: `"args": {}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// A credential slot, by key (a handle, never a secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRef {
    pub credential: String,
}

/// The `ok` value of `storage.usage`. Unknown fields are ignored (host to
/// guest only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    /// Bytes in use: every key and its JSON value, as UTF-8.
    pub used_bytes: u64,
    pub keys: u64,
    pub quota_bytes: u64,
    pub max_keys: u64,
}

impl CapabilityRequest {
    /// Stable operation name, for audit records and rate classes.
    pub fn op(&self) -> &'static str {
        match self {
            Self::HttpFetch(_) => "http.fetch",
            Self::Notify(_) => "notify",
            Self::Log(_) => "log",
            Self::StorageGet(_) => "storage.get",
            Self::StorageSet(_) => "storage.set",
            Self::StorageDelete(_) => "storage.delete",
            Self::StorageKeys(_) => "storage.keys",
            Self::StorageUsage(_) => "storage.usage",
            Self::CredentialHas(_) => "credentials.has",
            Self::SettingsGet(_) => "settings.get",
            Self::VariablesPublish(_) => "variables.publish",
            Self::VariablesClear(_) => "variables.clear",
        }
    }

    /// Checks the size bounds of ABI 1 (not permissions: that is the
    /// broker's job).
    pub fn validate(&self) -> Result<(), RequestError> {
        let too_long = |s: &str, max: usize| s.chars().count() > max;
        match self {
            Self::HttpFetch(f) => {
                if f.url.len() > MAX_URL_LEN {
                    return Err(RequestError::TooLarge("url"));
                }
                if f.headers.len() > MAX_HEADERS {
                    return Err(RequestError::TooLarge("headers"));
                }
                if f.headers
                    .iter()
                    .any(|h| h.name.len() > MAX_HEADER_LEN || h.value.len() > MAX_HEADER_LEN)
                {
                    return Err(RequestError::TooLarge("header"));
                }
                if f.body.as_ref().is_some_and(|b| b.len() > MAX_BODY_LEN) {
                    return Err(RequestError::TooLarge("body"));
                }
                if f.auth
                    .as_ref()
                    .is_some_and(|a| !is_credential_key(&a.credential))
                {
                    return Err(RequestError::TooLarge("credential"));
                }
            }
            Self::Notify(n) => {
                if too_long(&n.title, MAX_NOTIFY_TITLE) || too_long(&n.body, MAX_NOTIFY_BODY) {
                    return Err(RequestError::TooLarge("notification"));
                }
            }
            Self::Log(l) => {
                if l.message.len() > MAX_LOG_LEN {
                    return Err(RequestError::TooLarge("message"));
                }
            }
            Self::StorageGet(k) | Self::StorageDelete(k) => validate_key(&k.key)?,
            Self::StorageSet(s) => {
                validate_key(&s.key)?;
                let len = serde_json::to_vec(&s.value)
                    .map(|v| v.len())
                    .unwrap_or(usize::MAX);
                if len > MAX_STORAGE_VALUE {
                    return Err(RequestError::TooLarge("value"));
                }
            }
            Self::StorageKeys(p) => {
                if p.prefix.as_deref().is_some_and(|prefix| {
                    prefix.chars().count() > MAX_STORAGE_KEY || prefix.chars().any(char::is_control)
                }) {
                    return Err(RequestError::TooLarge("prefix"));
                }
            }
            Self::CredentialHas(c) => {
                if !is_credential_key(&c.credential) {
                    return Err(RequestError::TooLarge("credential"));
                }
            }
            Self::StorageUsage(_) | Self::SettingsGet(_) => {}
            Self::VariablesPublish(p) => {
                if !is_variable_id(&p.id) {
                    return Err(RequestError::TooLarge("variable"));
                }
                let len = serde_json::to_vec(&p.value)
                    .map(|v| v.len())
                    .unwrap_or(usize::MAX);
                if len > MAX_VARIABLE_VALUE {
                    return Err(RequestError::TooLarge("value"));
                }
            }
            Self::VariablesClear(c) => {
                if c.id.as_deref().is_some_and(|id| !is_variable_id(id)) {
                    return Err(RequestError::TooLarge("variable"));
                }
            }
        }
        Ok(())
    }
}

fn validate_key(key: &str) -> Result<(), RequestError> {
    if key.is_empty() || key.chars().count() > MAX_STORAGE_KEY || key.chars().any(char::is_control)
    {
        Err(RequestError::TooLarge("key"))
    } else {
        Ok(())
    }
}

/// A request broke a size bound. Names the field, never its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    TooLarge(&'static str),
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(field) => write!(f, "{field} exceeds its limit"),
        }
    }
}

impl std::error::Error for RequestError {}

/// Reply to one capability request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum CallResult {
    Ok(Value),
    Error(CallError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallError {
    pub code: CallErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CallErrorCode {
    /// Not granted, revoked, or never grantable.
    NotPermitted,
    /// A rate limit was hit.
    Throttled,
    /// Malformed request (bad JSON, unknown field, size bound).
    BadRequest,
    /// The operation does not exist in this ABI.
    UnsupportedOp,
    /// The operation was allowed but failed while being carried out.
    Failed,
    /// The capability is not available right now.
    Unavailable,
}

impl CallResult {
    pub fn error(code: CallErrorCode, message: impl Into<String>) -> Self {
        Self::Error(CallError {
            code,
            message: message.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<CapabilityRequest, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn requests_round_trip() {
        let r = parse(
            r#"{"op":"http.fetch","args":{"method":"GET","url":"https://api.example.com/x"}}"#,
        )
        .unwrap();
        assert_eq!(r.op(), "http.fetch");
        r.validate().unwrap();
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(parse(&s).unwrap(), r);
    }

    #[test]
    fn unknown_ops_do_not_parse() {
        for op in [
            "fs.read",
            "fs.write",
            "process.exec",
            "clipboard.read",
            "keystore.getDatabaseKey",
            "HTTP.FETCH",
            "",
        ] {
            let json = format!(r#"{{"op":"{op}","args":{{"path":"/etc/passwd"}}}}"#);
            assert!(parse(&json).is_err(), "{op}");
        }
    }

    #[test]
    fn plugin_ids_in_requests_are_rejected() {
        assert!(
            parse(r#"{"op":"log","plugin":"other","args":{"level":"info","message":"m"}}"#)
                .is_err()
        );
        assert!(
            parse(r#"{"op":"log","args":{"level":"info","message":"m","plugin":"other"}}"#)
                .is_err()
        );
    }

    #[test]
    fn sizes_are_bounded() {
        let big = CapabilityRequest::Log(Log {
            level: LogLevel::Info,
            message: "x".repeat(MAX_LOG_LEN + 1),
        });
        assert_eq!(big.validate(), Err(RequestError::TooLarge("message")));
        let key = CapabilityRequest::StorageGet(StorageKey { key: String::new() });
        assert!(key.validate().is_err());
        let notify = CapabilityRequest::Notify(Notify {
            title: "t".repeat(65),
            body: String::new(),
        });
        assert!(notify.validate().is_err());
        let value = CapabilityRequest::StorageSet(StorageSet {
            key: "k".into(),
            value: Value::String("v".repeat(MAX_STORAGE_VALUE)),
        });
        assert!(value.validate().is_err());
        let method =
            parse(r#"{"op":"http.fetch","args":{"method":"CONNECT","url":"https://a.b"}}"#);
        assert!(method.is_err());
    }

    #[test]
    fn auth_is_an_optional_compatible_addition() {
        // The original ABI 1 form parses and serializes byte-identically.
        let old = r#"{"op":"http.fetch","args":{"method":"GET","url":"https://a.example.com/"}}"#;
        let r = parse(old).unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), old);
        let CapabilityRequest::HttpFetch(f) = &r else {
            panic!()
        };
        assert_eq!(f.auth, None);

        let new = r#"{"op":"http.fetch","args":{"method":"GET","url":"https://a.example.com/","auth":{"credential":"apiKey","scheme":"bearer"}}}"#;
        let r = parse(new).unwrap();
        r.validate().unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), new);

        for bad in [
            r#"{"credential":"apiKey","scheme":"digest"}"#,
            r#"{"credential":"apiKey"}"#,
            r#"{"credential":"apiKey","scheme":"bearer","value":"s3cret"}"#,
        ] {
            let json = format!(
                r#"{{"op":"http.fetch","args":{{"method":"GET","url":"https://a.example.com/","auth":{bad}}}}}"#
            );
            assert!(parse(&json).is_err(), "{bad}");
        }
        for key in ["", "ApiKey", "api-key", "api_key", &"k".repeat(33)] {
            let json = format!(
                r#"{{"op":"http.fetch","args":{{"method":"GET","url":"https://a.example.com/","auth":{{"credential":"{key}","scheme":"bearer"}}}}}}"#
            );
            assert_eq!(
                parse(&json).unwrap().validate(),
                Err(RequestError::TooLarge("credential")),
                "{key:?}"
            );
        }
        assert!(is_credential_key("token"));
        assert!(is_credential_key("apiKey2"));
    }

    #[test]
    fn storage_credential_and_settings_ops_parse_strictly() {
        for (json, op) in [
            (r#"{"op":"storage.keys","args":{}}"#, "storage.keys"),
            (
                r#"{"op":"storage.keys","args":{"prefix":"a."}}"#,
                "storage.keys",
            ),
            (r#"{"op":"storage.usage","args":{}}"#, "storage.usage"),
            (
                r#"{"op":"credentials.has","args":{"credential":"apiKey"}}"#,
                "credentials.has",
            ),
            (r#"{"op":"settings.get","args":{}}"#, "settings.get"),
        ] {
            let r = parse(json).unwrap();
            assert_eq!(r.op(), op);
            r.validate().unwrap();
            assert_eq!(serde_json::to_string(&r).unwrap(), json);
        }
        for bad in [
            r#"{"op":"settings.get","args":{"plugin":"other.plugin"}}"#,
            r#"{"op":"storage.usage","args":{"quota":1}}"#,
            r#"{"op":"credentials.has","args":{"credential":"apiKey","secret":"x"}}"#,
            r#"{"op":"credentials.get","args":{"credential":"apiKey"}}"#,
            r#"{"op":"credentials.reveal","args":{"credential":"apiKey"}}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let bad_key = parse(r#"{"op":"credentials.has","args":{"credential":"api-key"}}"#).unwrap();
        assert_eq!(
            bad_key.validate(),
            Err(RequestError::TooLarge("credential"))
        );
        let long = CapabilityRequest::StorageKeys(StoragePrefix {
            prefix: Some("p".repeat(MAX_STORAGE_KEY + 1)),
        });
        assert_eq!(long.validate(), Err(RequestError::TooLarge("prefix")));
        let usage: StorageUsage = serde_json::from_str(
            r#"{"usedBytes":10,"keys":1,"quotaBytes":262144,"maxKeys":1000,"later":true}"#,
        )
        .unwrap();
        assert_eq!(usage.keys, 1);
    }

    #[test]
    fn variables_name_only_relative_ids_and_are_bounded() {
        let publish =
            parse(r#"{"op":"variables.publish","args":{"id":"current.temperature","value":21.5}}"#)
                .unwrap();
        assert_eq!(publish.op(), "variables.publish");
        publish.validate().unwrap();
        let clear = parse(r#"{"op":"variables.clear","args":{}}"#).unwrap();
        assert_eq!(clear.op(), "variables.clear");
        assert_eq!(
            serde_json::to_string(&clear).unwrap(),
            r#"{"op":"variables.clear","args":{}}"#
        );
        parse(r#"{"op":"variables.clear","args":{"id":"x"}}"#)
            .unwrap()
            .validate()
            .unwrap();
        // An absolute id, another plugin's, a bad segment, too deep: refused.
        for id in [
            "plugins.other.prs.count",
            "a.b.c.d",
            "Temperature",
            "a-b",
            "",
            "a..b",
            &"x".repeat(33),
        ] {
            let json = format!(r#"{{"op":"variables.publish","args":{{"id":"{id}","value":1}}}}"#);
            assert_eq!(
                parse(&json).unwrap().validate(),
                Err(RequestError::TooLarge("variable")),
                "{id}"
            );
        }
        let big = CapabilityRequest::VariablesPublish(VariablePublish {
            id: "x".into(),
            value: Value::String("v".repeat(MAX_VARIABLE_VALUE)),
        });
        assert_eq!(big.validate(), Err(RequestError::TooLarge("value")));
        assert!(
            parse(r#"{"op":"variables.publish","args":{"id":"x","value":1,"plugin":"o"}}"#)
                .is_err()
        );
    }

    #[test]
    fn http_responses_tolerate_additions() {
        let r: HttpResponse =
            serde_json::from_str(r#"{"status":200,"body":"{}","elapsedMs":12}"#).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body.as_deref(), Some("{}"));
        assert!(r.headers.is_empty());
        assert!(serde_json::from_str::<HttpResponse>(r#"{"body":"x"}"#).is_err());
    }

    #[test]
    fn call_results_are_tagged() {
        let ok = serde_json::to_string(&CallResult::Ok(Value::from(1))).unwrap();
        assert_eq!(ok, r#"{"ok":1}"#);
        let err = CallResult::error(CallErrorCode::NotPermitted, "no");
        let s = serde_json::to_string(&err).unwrap();
        assert_eq!(s, r#"{"error":{"code":"not-permitted","message":"no"}}"#);
        assert!(serde_json::from_str::<CallResult>(r#"{"ok":1,"error":2}"#).is_err());
    }
}
