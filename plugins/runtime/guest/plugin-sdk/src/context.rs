//! Brokered capabilities for one invocation.

use std::fmt;

use deckpilot_plugin_abi::capability::{
    AuthScheme, Header, HttpAuth, HttpFetch, HttpResponse, Log, LogLevel, Notify, StorageKey,
    StorageSet,
};
use deckpilot_plugin_abi::limits::MAX_HOST_CALLS_PER_INVOCATION;
use deckpilot_plugin_abi::{CallError, CallErrorCode, CallResult, CapabilityRequest, HttpMethod};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::definition::is_camel_key;
use crate::output::{PluginError, codes};

/// Longest log line the host accepts; longer lines are cut (at a character
/// boundary) by [`Context::log`].
pub const MAX_LOG_LEN: usize = deckpilot_plugin_abi::limits::MAX_LOG_LEN;

/// Where capability requests go: the host (wasm) or a test double.
pub(crate) trait Transport {
    fn call(&mut self, request: &CapabilityRequest) -> CallResult;
}

/// `deckpilot_v1.host_call`. Off wasm there is no host: `unavailable`.
pub(crate) struct WasmTransport;

impl Transport for WasmTransport {
    fn call(&mut self, request: &CapabilityRequest) -> CallResult {
        let Ok(bytes) = serde_json::to_vec(request) else {
            return CallResult::error(CallErrorCode::BadRequest, "unserializable request");
        };
        match deckpilot_plugin_guest::__rt::host_call_bytes(&bytes) {
            Some(reply) => serde_json::from_slice(&reply).unwrap_or_else(|_| {
                CallResult::error(CallErrorCode::Failed, "malformed reply from the host")
            }),
            None => CallResult::error(
                CallErrorCode::Unavailable,
                "not running in a plugin host (use testing::MockHost in native tests)",
            ),
        }
    }
}

/// Capability access during one invocation. Every call is a message to the
/// agent, which checks it against the consent stored for this plugin; a
/// call outside the consent fails with [`CapabilityErrorKind::NotPermitted`].
pub struct Context<'a> {
    transport: &'a mut dyn Transport,
    calls: u32,
}

impl fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("calls", &self.calls)
            .finish_non_exhaustive()
    }
}

impl<'a> Context<'a> {
    pub(crate) fn new(transport: &'a mut dyn Transport) -> Self {
        Self {
            transport,
            calls: 0,
        }
    }

    /// Capability calls made so far in this invocation.
    pub fn calls(&self) -> u32 {
        self.calls
    }

    fn call(
        &mut self,
        op: &'static str,
        request: &CapabilityRequest,
    ) -> Result<Value, CapabilityError> {
        // Past the limit the host would trap; refuse locally instead.
        if self.calls >= MAX_HOST_CALLS_PER_INVOCATION {
            return Err(CapabilityError {
                op,
                kind: CapabilityErrorKind::Throttled,
                detail: format!(
                    "more than {MAX_HOST_CALLS_PER_INVOCATION} capability calls in one invocation"
                ),
                target: None,
            });
        }
        self.calls += 1;
        match self.transport.call(request) {
            CallResult::Ok(v) => Ok(v),
            CallResult::Error(e) => Err(CapabilityError::from_call(op, e)),
        }
    }

    /// Writes one line to the plugin log (always granted, rate limited).
    /// Best effort: a dropped line is not an error for the action.
    pub fn log(&mut self, level: LogLevel, message: impl AsRef<str>) {
        let message = cut(message.as_ref(), MAX_LOG_LEN);
        let _ = self.call(
            "log",
            &CapabilityRequest::Log(Log {
                level,
                message: message.to_owned(),
            }),
        );
    }

    pub fn debug(&mut self, message: impl AsRef<str>) {
        self.log(LogLevel::Debug, message);
    }

    pub fn info(&mut self, message: impl AsRef<str>) {
        self.log(LogLevel::Info, message);
    }

    pub fn warn(&mut self, message: impl AsRef<str>) {
        self.log(LogLevel::Warn, message);
    }

    pub fn error(&mut self, message: impl AsRef<str>) {
        self.log(LogLevel::Error, message);
    }

    /// Performs an HTTP request through the agent. Needs a `network`
    /// permission for the request's exact https origin and method. A
    /// non-2xx status is a [`Response`], not an error.
    pub fn fetch(&mut self, request: Request) -> Result<Response, CapabilityError> {
        let target = origin_of(&request.url);
        let req = CapabilityRequest::HttpFetch(HttpFetch {
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body,
            auth: request.auth,
        });
        let value = self
            .call("http.fetch", &req)
            .map_err(|e| e.with_target(target.clone()))?;
        let r: HttpResponse = serde_json::from_value(value).map_err(|_| CapabilityError {
            op: "http.fetch",
            kind: CapabilityErrorKind::MalformedReply,
            detail: "the reply is not an HTTP response".into(),
            target,
        })?;
        Ok(Response {
            status: r.status,
            headers: r.headers,
            body: r.body,
        })
    }

    /// `fetch(Request::get(url))`.
    pub fn http_get(&mut self, url: impl Into<String>) -> Result<Response, CapabilityError> {
        self.fetch(Request::get(url))
    }

    /// Shows a desktop notification (needs the `notifications` permission;
    /// the agent prefixes the plugin's name).
    pub fn notify(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<(), CapabilityError> {
        self.call(
            "notify",
            &CapabilityRequest::Notify(Notify {
                title: title.into(),
                body: body.into(),
            }),
        )
        .map(drop)
    }

    /// Reads a value from the plugin's own storage (always granted).
    /// `None` when the key is not set.
    pub fn storage_get(
        &mut self,
        key: impl Into<String>,
    ) -> Result<Option<Value>, CapabilityError> {
        let v = self.call(
            "storage.get",
            &CapabilityRequest::StorageGet(StorageKey { key: key.into() }),
        )?;
        Ok((!v.is_null()).then_some(v))
    }

    pub fn storage_set(
        &mut self,
        key: impl Into<String>,
        value: impl Serialize,
    ) -> Result<(), CapabilityError> {
        let value = serde_json::to_value(value).map_err(|_| CapabilityError {
            op: "storage.set",
            kind: CapabilityErrorKind::BadRequest,
            detail: "the value does not serialize to JSON".into(),
            target: None,
        })?;
        self.call(
            "storage.set",
            &CapabilityRequest::StorageSet(StorageSet {
                key: key.into(),
                value,
            }),
        )
        .map(drop)
    }

    pub fn storage_delete(&mut self, key: impl Into<String>) -> Result<(), CapabilityError> {
        self.call(
            "storage.delete",
            &CapabilityRequest::StorageDelete(StorageKey { key: key.into() }),
        )
        .map(drop)
    }
}

/// `scheme://host[:port]` of a URL, for messages (never path or query,
/// which may carry secrets).
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let ok = scheme.chars().all(|c| c.is_ascii_alphanumeric())
        && !host.is_empty()
        && host.len() <= 260
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    ok.then(|| {
        format!(
            "{}://{}",
            scheme.to_ascii_lowercase(),
            host.to_ascii_lowercase()
        )
    })
}

fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A credential slot, by key. The secret never enters the plugin: the
/// agent injects it into a request to one of the slot's origins, in one of
/// its schemes, if the user stored one and the grant allows it.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SecretHandle {
    key: String,
}

impl SecretHandle {
    /// The slot `key` from the manifest's `contributes.credentials`.
    pub fn slot(key: &str) -> Result<Self, PluginError> {
        if is_camel_key(key) {
            Ok(Self::new_unchecked(key))
        } else {
            Err(PluginError::new(
                codes::PLUGIN_ERROR,
                "a credential slot key is camelCase, at most 32 characters",
            ))
        }
    }

    pub(crate) fn new_unchecked(key: &str) -> Self {
        Self {
            key: key.to_owned(),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

impl fmt::Debug for SecretHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretHandle({})", self.key)
    }
}

/// An HTTP request for [`Context::fetch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    method: HttpMethod,
    url: String,
    headers: Vec<Header>,
    body: Option<String>,
    auth: Option<HttpAuth>,
}

impl Request {
    pub fn new(method: HttpMethod, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            auth: None,
        }
    }

    pub fn get(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Get, url)
    }

    pub fn post(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Post, url)
    }

    /// Adds a header. Credentials, cookies and framing headers are refused
    /// by the broker; use [`Request::credential`] for secrets.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push(Header {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// A JSON body with `Content-Type: application/json`.
    pub fn json(self, value: &impl Serialize) -> Result<Self, PluginError> {
        let body = serde_json::to_string(value).map_err(|_| {
            PluginError::new(codes::PLUGIN_ERROR, "the request body does not serialize")
        })?;
        Ok(self.header("Content-Type", "application/json").body(body))
    }

    /// Asks the agent to inject the secret of `handle` in `scheme`.
    pub fn credential(mut self, handle: &SecretHandle, scheme: AuthScheme) -> Self {
        self.auth = Some(HttpAuth {
            credential: handle.key.clone(),
            scheme,
        });
        self
    }

    /// `credential(handle, AuthScheme::Bearer)`.
    pub fn bearer(self, handle: &SecretHandle) -> Self {
        self.credential(handle, AuthScheme::Bearer)
    }
}

/// The agent's reply to an allowed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<Header>,
    pub body: Option<String>,
}

impl Response {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// First header named `name` (ASCII case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    /// The body text (empty if there was none).
    pub fn text(&self) -> &str {
        self.body.as_deref().unwrap_or("")
    }

    /// The body parsed as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(self.text())
    }
}

/// Why a capability call did not happen (or failed while happening).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapabilityErrorKind {
    /// The permission broker denied it: not declared, not approved,
    /// revoked, or never grantable.
    NotPermitted,
    /// A rate limit was hit.
    Throttled,
    /// The request is malformed or exceeds a size limit.
    BadRequest,
    /// The operation does not exist for this host.
    Unsupported,
    /// Allowed, but failed while being carried out (network error, ...).
    Failed,
    /// The capability is not available right now (or there is no host).
    Unavailable,
    /// The host's reply did not have the expected shape.
    MalformedReply,
}

/// A capability call failed. The message says which operation, what was
/// wrong and, for a denial, how to declare the permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityError {
    op: &'static str,
    kind: CapabilityErrorKind,
    detail: String,
    target: Option<String>,
}

impl CapabilityError {
    fn from_call(op: &'static str, e: CallError) -> Self {
        let kind = match e.code {
            CallErrorCode::NotPermitted => CapabilityErrorKind::NotPermitted,
            CallErrorCode::Throttled => CapabilityErrorKind::Throttled,
            CallErrorCode::BadRequest => CapabilityErrorKind::BadRequest,
            CallErrorCode::UnsupportedOp => CapabilityErrorKind::Unsupported,
            CallErrorCode::Failed => CapabilityErrorKind::Failed,
            CallErrorCode::Unavailable => CapabilityErrorKind::Unavailable,
        };
        Self {
            op,
            kind,
            detail: e.message,
            target: None,
        }
    }

    fn with_target(mut self, target: Option<String>) -> Self {
        self.target = target;
        self
    }

    /// The operation, e.g. `http.fetch`.
    pub fn op(&self) -> &'static str {
        self.op
    }

    pub fn kind(&self) -> CapabilityErrorKind {
        self.kind
    }

    /// The host's reason, e.g. `origin not granted`.
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// The request's origin (`https://host[:port]`), when there was one.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn is_denied(&self) -> bool {
        self.kind == CapabilityErrorKind::NotPermitted
    }
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = self.op;
        let to = self
            .target
            .as_deref()
            .map(|t| format!(" to {t}"))
            .unwrap_or_default();
        let detail = &self.detail;
        match self.kind {
            CapabilityErrorKind::NotPermitted => {
                write!(
                    f,
                    "{op}{to} was denied by the permission broker ({detail}). "
                )?;
                match op {
                    "http.fetch" => f.write_str(
                        "Declare a `network` permission for this exact https host and method \
                         in manifest.json; it applies once the user approves it",
                    ),
                    "notify" => f.write_str(
                        "Declare the `notifications` permission in manifest.json; it applies \
                         once the user approves it",
                    ),
                    _ => f.write_str("It is not grantable to plugins in API 1"),
                }
            }
            CapabilityErrorKind::Throttled => write!(f, "{op}{to} was rate limited ({detail})"),
            CapabilityErrorKind::BadRequest => {
                write!(f, "{op}{to} was refused as malformed ({detail})")
            }
            CapabilityErrorKind::Unsupported => {
                write!(f, "{op}{to} is not supported by this host ({detail})")
            }
            CapabilityErrorKind::Failed => write!(f, "{op}{to} failed ({detail})"),
            CapabilityErrorKind::Unavailable => write!(f, "{op}{to} is unavailable ({detail})"),
            CapabilityErrorKind::MalformedReply => {
                write!(f, "{op}{to}: unexpected reply from the host ({detail})")
            }
        }
    }
}

impl std::error::Error for CapabilityError {}

/// `?` in a handler: a denial becomes `not-permitted`, a rate limit
/// `throttled` (retryable), anything else `request-failed` or
/// `unsupported`. The message is the full explanation above.
impl From<CapabilityError> for PluginError {
    fn from(e: CapabilityError) -> Self {
        let msg = e.to_string();
        match e.kind {
            CapabilityErrorKind::NotPermitted => PluginError::new(codes::NOT_PERMITTED, msg),
            CapabilityErrorKind::Throttled => PluginError::new(codes::THROTTLED, msg).retryable(),
            CapabilityErrorKind::Unsupported => PluginError::new(codes::UNSUPPORTED, msg),
            CapabilityErrorKind::Failed | CapabilityErrorKind::Unavailable => {
                PluginError::new(codes::REQUEST_FAILED, msg).retryable()
            }
            CapabilityErrorKind::BadRequest | CapabilityErrorKind::MalformedReply => {
                PluginError::new(codes::REQUEST_FAILED, msg)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_for_messages_drop_path_query_and_userinfo() {
        assert_eq!(
            origin_of("https://API.example.com/v1?key=s3cret").as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(
            origin_of("https://user:pw@evil.net:8443/x").as_deref(),
            Some("https://evil.net:8443")
        );
        assert_eq!(origin_of("not a url"), None);
        assert_eq!(origin_of("https://a b/"), None);
    }

    #[test]
    fn denials_explain_themselves() {
        let e = CapabilityError::from_call(
            "http.fetch",
            CallError {
                code: CallErrorCode::NotPermitted,
                message: "origin not granted".into(),
            },
        )
        .with_target(Some("https://evil.net".into()));
        let text = e.to_string();
        assert!(
            text.contains("http.fetch to https://evil.net was denied"),
            "{text}"
        );
        assert!(text.contains("origin not granted"), "{text}");
        assert!(text.contains("`network` permission"), "{text}");
        let p = PluginError::from(e);
        assert_eq!(p.code(), codes::NOT_PERMITTED);
        assert!(!p.is_retryable());
        let t = PluginError::from(CapabilityError::from_call(
            "log",
            CallError {
                code: CallErrorCode::Throttled,
                message: "rate limited".into(),
            },
        ));
        assert_eq!(t.code(), codes::THROTTLED);
        assert!(t.is_retryable());
    }

    #[test]
    fn log_lines_are_cut_at_character_boundaries() {
        let s = "é".repeat(MAX_LOG_LEN);
        let c = cut(&s, MAX_LOG_LEN);
        assert!(c.len() <= MAX_LOG_LEN);
        assert!(c.chars().all(|ch| ch == 'é'));
    }

    #[test]
    fn secret_handles_are_keys_only() {
        let h = SecretHandle::slot("apiKey").unwrap();
        assert_eq!(format!("{h:?}"), "SecretHandle(apiKey)");
        assert!(SecretHandle::slot("api key").is_err());
        let r = Request::get("https://api.example.com/").bearer(&h);
        assert_eq!(
            r.auth,
            Some(HttpAuth {
                credential: "apiKey".into(),
                scheme: AuthScheme::Bearer
            })
        );
    }
}
