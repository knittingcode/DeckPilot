//! Brokered capabilities for one invocation.

use std::fmt;

pub use deckpilot_plugin_abi::capability::StorageUsage;
use deckpilot_plugin_abi::capability::{
    AuthScheme, CredentialRef, Header, HttpAuth, HttpFetch, HttpResponse, Log, LogLevel, NoArgs,
    Notify, NotifySeverity, OpenUrl, StorageKey, StoragePrefix, StorageSet, VariableClear,
    VariablePublish,
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

    /// Writes one line to the plugin log (op `log`): always granted to an
    /// enabled plugin, 50 lines per second, at most [`MAX_LOG_LEN`] bytes
    /// (longer lines are cut). Best effort: a dropped or throttled line is
    /// not an error for the action. The agent writes it to its own log,
    /// attributed to the plugin; never log secrets or personal data.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CapabilityRequest, LogLevel};
    ///
    /// let mut host = MockHost::allow_all();
    /// let mut ctx = host.context();
    /// ctx.log(LogLevel::Info, "checking the build");
    /// ctx.warn("rate limit almost reached"); // also: debug, info, error
    /// assert!(matches!(&host.requests()[0],
    ///     CapabilityRequest::Log(l) if l.message == "checking the build"));
    /// ```
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

    /// Performs an HTTP request through the agent (op `http.fetch`).
    ///
    /// Needs a `network` permission for the request's exact https host and
    /// method; redirects are re-checked hop by hop (at most 5) and never
    /// carry a credential to another origin. Every resolved address must be
    /// public. Limits: URL 8 KiB, 32 headers of at most 4 KiB, request
    /// body 32 KiB, response body 512 KiB, 15 s for all hops, 60 requests
    /// per minute. The response keeps only the headers listed in
    /// [`RESPONSE_HEADERS`](deckpilot_plugin_abi::capability::RESPONSE_HEADERS):
    /// content and caching metadata (`content-type`, `etag`,
    /// `last-modified`, `retry-after`, ...), pagination (`link`, `x-total`,
    /// `x-total-pages`, `x-next-page`, ...) and rate limits
    /// (`x-ratelimit-*`, `ratelimit-*`, by exact name). `set-cookie`,
    /// `www-authenticate` and every other header are dropped, and an
    /// injected secret is redacted from the values that are kept. Read
    /// them with [`Response::header`], [`Response::header_values`] and
    /// [`Response::link`]. A non-2xx status is a [`Response`], not an
    /// error.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::prelude::*;
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CallErrorCode, CallResult, CapabilityRequest};
    ///
    /// let mut host = MockHost::new(|req| match req {
    ///     CapabilityRequest::HttpFetch(f) if f.url.starts_with("https://api.example.com/") => {
    ///         CallResult::Ok(serde_json::json!({"status": 200, "body": "{\"n\": 3}"}))
    ///     }
    ///     _ => CallResult::error(CallErrorCode::NotPermitted, "origin not granted"),
    /// });
    /// let mut ctx = host.context();
    /// let r = ctx
    ///     .fetch(Request::get("https://api.example.com/count").header("Accept", "application/json"))
    ///     .unwrap();
    /// assert!(r.is_success());
    /// let v: serde_json::Value = r.json().unwrap();
    /// assert_eq!(v["n"], 3);
    ///
    /// let denied = ctx.http_get("https://evil.example.net/").unwrap_err();
    /// assert!(denied.is_denied());
    /// assert_eq!(denied.target(), Some("https://evil.example.net"));
    /// ```
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

    /// Shows a notification (op `notify`). Needs the `notifications`
    /// permission; the agent titles it with the plugin's name. Title at
    /// most 64, body at most 300 characters; 5 per minute.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    ///
    /// let mut host = MockHost::deny_all(); // `notifications` not granted
    /// let mut ctx = host.context();
    /// let e = ctx.notify("Build failed", "main is red").unwrap_err();
    /// assert!(e.is_denied());
    /// assert!(e.to_string().contains("`notifications` permission"));
    /// ```
    pub fn notify(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<(), CapabilityError> {
        self.send_notification(title.into(), body.into(), None)
    }

    /// [`Context::notify`] with a severity (`Info`, `Success`, `Warning`,
    /// `Error`), which sets how the notification looks. It never changes
    /// where it is shown, the rate limit or the plugin-name prefix. An
    /// agent that predates severities refuses the request as malformed
    /// ([`CapabilityErrorKind::BadRequest`]); fall back to
    /// [`Context::notify`] then.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CapabilityRequest, NotifySeverity};
    ///
    /// let mut host = MockHost::allow_all();
    /// let mut ctx = host.context();
    /// ctx.notify_with(NotifySeverity::Error, "Build failed", "main is red").unwrap();
    /// assert!(matches!(&host.requests()[0],
    ///     CapabilityRequest::Notify(n) if n.severity == Some(NotifySeverity::Error)));
    /// ```
    pub fn notify_with(
        &mut self,
        severity: NotifySeverity,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<(), CapabilityError> {
        self.send_notification(title.into(), body.into(), Some(severity))
    }

    fn send_notification(
        &mut self,
        title: String,
        body: String,
        severity: Option<NotifySeverity>,
    ) -> Result<(), CapabilityError> {
        self.call(
            "notify",
            &CapabilityRequest::Notify(Notify {
                title,
                body,
                severity,
            }),
        )
        .map(drop)
    }

    /// Opens an https URL in the user's browser (op `url.open`). Needs an
    /// `openUrl` permission for the URL's exact host (`openUrl:github.com`
    /// covers `https://github.com/...` on port 443 only, not
    /// `gist.github.com`), approved by the user. Only `https:` URLs
    /// without userinfo are opened: never `file:`, `mailto:` or another
    /// scheme that could start a program, never an IP address or a local
    /// name. At most 2,048 bytes; 1 per 2 seconds and 20 per 10 minutes
    /// (then [`CapabilityErrorKind::Throttled`]), and at most 3 in one
    /// invocation. The agent records the URL's origin, never its path or
    /// query. Open URLs only in actions the user starts on purpose (an
    /// "Open pull request" key), not on every run of a workflow.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CallErrorCode, CallResult, CapabilityRequest};
    ///
    /// let mut host = MockHost::new(|req| match req {
    ///     CapabilityRequest::OpenUrl(o) if o.url.starts_with("https://github.com/") => {
    ///         CallResult::Ok(serde_json::Value::Null)
    ///     }
    ///     _ => CallResult::error(CallErrorCode::NotPermitted, "origin not granted"),
    /// });
    /// let mut ctx = host.context();
    /// ctx.open_url("https://github.com/acme/app/pull/7").unwrap();
    /// let denied = ctx.open_url("https://gitlab.com/acme/app").unwrap_err();
    /// assert!(denied.is_denied());
    /// assert_eq!(denied.target(), Some("https://gitlab.com"));
    /// assert!(denied.to_string().contains("`openUrl` permission"));
    /// ```
    pub fn open_url(&mut self, url: impl Into<String>) -> Result<(), CapabilityError> {
        let url = url.into();
        let target = origin_of(&url);
        self.call("url.open", &CapabilityRequest::OpenUrl(OpenUrl { url }))
            .map(drop)
            .map_err(|e| e.with_target(target))
    }

    /// Reads a value from the plugin's own storage (op `storage.get`,
    /// always granted to an enabled plugin). `None` when the key is not
    /// set. Storage is per plugin and per computer, kept in the agent's
    /// encrypted plugin database, never exported, and deleted on uninstall.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CallResult, CapabilityRequest};
    ///
    /// let mut host = MockHost::new(|req| match req {
    ///     CapabilityRequest::StorageGet(k) if k.key == "last" => {
    ///         CallResult::Ok(serde_json::json!({"repo": "acme/app", "prs": 4}))
    ///     }
    ///     _ => CallResult::Ok(serde_json::Value::Null), // not set
    /// });
    /// let mut ctx = host.context();
    /// assert_eq!(ctx.storage_get("last").unwrap().unwrap()["prs"], 4);
    /// assert_eq!(ctx.storage_get("other").unwrap(), None);
    /// ```
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

    /// [`Context::storage_get`], deserialized as `T`. A stored value of
    /// another shape is [`CapabilityErrorKind::MalformedReply`].
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::CallResult;
    ///
    /// #[derive(serde::Deserialize)]
    /// struct Last {
    ///     prs: u32,
    /// }
    ///
    /// let mut host = MockHost::new(|_| CallResult::Ok(serde_json::json!({"prs": 4})));
    /// let mut ctx = host.context();
    /// let last: Option<Last> = ctx.storage_get_as("last").unwrap();
    /// assert_eq!(last.unwrap().prs, 4);
    /// ```
    pub fn storage_get_as<T: DeserializeOwned>(
        &mut self,
        key: impl Into<String>,
    ) -> Result<Option<T>, CapabilityError> {
        match self.storage_get(key)? {
            None => Ok(None),
            Some(v) => serde_json::from_value(v)
                .map(Some)
                .map_err(|_| malformed("storage.get", "a value of the expected type")),
        }
    }

    /// Stores `value` as JSON under `key` (op `storage.set`, always
    /// granted to an enabled plugin). Key 1-128 characters without control
    /// characters, value at most 64 KiB of JSON. The plugin's storage holds
    /// at most 1,000 keys and 256 KiB (key plus JSON value, UTF-8), or the
    /// approved `storage` permission's `quotaKb`; a write that would grow it
    /// past the quota fails with [`CapabilityErrorKind::Failed`] and a
    /// detail starting `storage-quota` (a write that does not grow it is
    /// allowed even above the quota). 100 storage calls per second.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::CapabilityRequest;
    ///
    /// let mut host = MockHost::allow_all();
    /// let mut ctx = host.context();
    /// ctx.storage_set("last", serde_json::json!({"prs": 4})).unwrap();
    /// ctx.storage_delete("stale").unwrap();
    /// assert!(matches!(&host.requests()[0],
    ///     CapabilityRequest::StorageSet(s) if s.key == "last" && s.value["prs"] == 4));
    /// ```
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

    /// Removes `key` (op `storage.delete`); removing a key that is not set
    /// is not an error.
    pub fn storage_delete(&mut self, key: impl Into<String>) -> Result<(), CapabilityError> {
        self.call(
            "storage.delete",
            &CapabilityRequest::StorageDelete(StorageKey { key: key.into() }),
        )
        .map(drop)
    }

    /// Publishes the value of one of the plugin's own variables, declared
    /// in `contributes.variables` of manifest.json (op `variables.publish`,
    /// always granted to an enabled plugin, 10 per second, at most 16 KiB).
    /// `id` is relative (`temperature`); workflows read it as
    /// `{{ plugins.<publisher>.<name>.temperature }}`. The agent refuses an
    /// undeclared id ([`CapabilityErrorKind::NotPermitted`]), a value of
    /// another type than the declared `valueType`
    /// ([`CapabilityErrorKind::BadRequest`]) and a variable whose scope is
    /// not `machine` ([`CapabilityErrorKind::Unavailable`]). A `null`
    /// value clears it. Values are kept in memory only: not across
    /// restarts of the agent, and cleared when the plugin is disabled,
    /// updated or removed.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::CapabilityRequest;
    ///
    /// let mut host = MockHost::allow_all();
    /// let mut ctx = host.context();
    /// ctx.publish_variable("openPrs", 4).unwrap();
    /// ctx.publish_variable("buildStatus", "success").unwrap();
    /// ctx.clear_variable("buildStatus").unwrap();
    /// ctx.clear_variables().unwrap();
    /// assert!(matches!(&host.requests()[0],
    ///     CapabilityRequest::VariablesPublish(v) if v.id == "openPrs" && v.value == 4));
    /// ```
    pub fn publish_variable(
        &mut self,
        id: impl Into<String>,
        value: impl Serialize,
    ) -> Result<(), CapabilityError> {
        let value = serde_json::to_value(value).map_err(|_| CapabilityError {
            op: "variables.publish",
            kind: CapabilityErrorKind::BadRequest,
            detail: "the value does not serialize to JSON".into(),
            target: None,
        })?;
        self.call(
            "variables.publish",
            &CapabilityRequest::VariablesPublish(VariablePublish {
                id: id.into(),
                value,
            }),
        )
        .map(drop)
    }

    /// Clears one published variable (it then has no value).
    pub fn clear_variable(&mut self, id: impl Into<String>) -> Result<(), CapabilityError> {
        self.call(
            "variables.clear",
            &CapabilityRequest::VariablesClear(VariableClear {
                id: Some(id.into()),
            }),
        )
        .map(drop)
    }

    /// Clears every variable the plugin published.
    pub fn clear_variables(&mut self) -> Result<(), CapabilityError> {
        self.call(
            "variables.clear",
            &CapabilityRequest::VariablesClear(VariableClear { id: None }),
        )
        .map(drop)
    }

    /// The plugin's own storage keys (those starting with `prefix`, if
    /// given), sorted (op `storage.keys`).
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CallResult, CapabilityRequest};
    ///
    /// let mut host = MockHost::new(|req| match req {
    ///     CapabilityRequest::StorageKeys(_) => {
    ///         CallResult::Ok(serde_json::json!(["repo.a", "repo.b"]))
    ///     }
    ///     _ => CallResult::Ok(serde_json::Value::Null),
    /// });
    /// let mut ctx = host.context();
    /// assert_eq!(ctx.storage_keys(Some("repo.")).unwrap(), ["repo.a", "repo.b"]);
    /// ```
    pub fn storage_keys(&mut self, prefix: Option<&str>) -> Result<Vec<String>, CapabilityError> {
        let v = self.call(
            "storage.keys",
            &CapabilityRequest::StorageKeys(StoragePrefix {
                prefix: prefix.map(str::to_owned),
            }),
        )?;
        serde_json::from_value(v).map_err(|_| malformed("storage.keys", "a list of keys"))
    }

    /// How much of its quota the plugin's storage uses (op
    /// `storage.usage`). A write beyond the quota fails with
    /// [`CapabilityErrorKind::Failed`] and a detail that starts with
    /// `storage-quota`.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::CallResult;
    ///
    /// let mut host = MockHost::new(|_| CallResult::Ok(serde_json::json!({
    ///     "usedBytes": 120, "keys": 3, "quotaBytes": 262144, "maxKeys": 1000
    /// })));
    /// let mut ctx = host.context();
    /// let u = ctx.storage_usage().unwrap();
    /// assert!(u.used_bytes < u.quota_bytes && u.keys < u.max_keys);
    /// ```
    pub fn storage_usage(&mut self) -> Result<StorageUsage, CapabilityError> {
        let v = self.call("storage.usage", &CapabilityRequest::StorageUsage(NoArgs {}))?;
        serde_json::from_value(v).map_err(|_| malformed("storage.usage", "a storage usage"))
    }

    /// Whether the user stored a secret for `handle`'s slot (op
    /// `credentials.has`). Never returns the secret; needs the slot's
    /// `credentials` grant. Use it to send a request without credentials,
    /// or to fail with [`codes::CREDENTIAL_MISSING`], when there is none:
    /// a fetch that names a slot without a secret fails with
    /// [`CapabilityErrorKind::Unavailable`] ("no secret is stored").
    ///
    /// ```
    /// use deckpilot_plugin_sdk::prelude::*;
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::{CallResult, CapabilityRequest};
    ///
    /// let mut host = MockHost::new(|req| match req {
    ///     CapabilityRequest::CredentialHas(_) => CallResult::Ok(serde_json::json!(true)),
    ///     _ => CallResult::Ok(serde_json::json!({"status": 200})),
    /// });
    /// let token = SecretHandle::slot("githubToken").unwrap();
    /// let mut ctx = host.context();
    /// let mut request = Request::get("https://api.github.com/user");
    /// if ctx.has_credential(&token).unwrap() {
    ///     request = request.bearer(&token); // the agent injects the secret
    /// }
    /// ctx.fetch(request).unwrap();
    /// let CapabilityRequest::HttpFetch(sent) = &host.requests()[1] else {
    ///     panic!()
    /// };
    /// assert_eq!(sent.auth.as_ref().unwrap().credential, "githubToken");
    /// ```
    pub fn has_credential(&mut self, handle: &SecretHandle) -> Result<bool, CapabilityError> {
        let v = self.call(
            "credentials.has",
            &CapabilityRequest::CredentialHas(CredentialRef {
                credential: handle.key.clone(),
            }),
        )?;
        v.as_bool()
            .ok_or_else(|| malformed("credentials.has", "true or false"))
    }

    /// The plugin's settings (`contributes.settings` in the manifest), as
    /// the user saved them, with the declared defaults filled in (op
    /// `settings.get`, always granted to an enabled plugin). A saved value
    /// that no longer fits its field is replaced by the default; a setting
    /// with neither is absent. Read them in every invocation: there is no
    /// change notification in ABI 1.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::testing::MockHost;
    /// use deckpilot_plugin_sdk::CallResult;
    ///
    /// let mut host = MockHost::new(|_| CallResult::Ok(serde_json::json!({
    ///     "defaultRepo": "acme/app", "notifyOnChange": false
    /// })));
    /// let mut ctx = host.context();
    /// let settings = ctx.settings().unwrap();
    /// assert_eq!(settings.text("defaultRepo"), Some("acme/app"));
    /// assert_eq!(settings.boolean("notifyOnChange"), Some(false));
    /// assert_eq!(settings.text("defaultBranch"), None);
    /// ```
    pub fn settings(&mut self) -> Result<Settings, CapabilityError> {
        let v = self.call("settings.get", &CapabilityRequest::SettingsGet(NoArgs {}))?;
        match v {
            Value::Object(map) => Ok(Settings(map)),
            _ => Err(malformed("settings.get", "an object of settings")),
        }
    }
}

fn malformed(op: &'static str, expected: &str) -> CapabilityError {
    CapabilityError {
        op,
        kind: CapabilityErrorKind::MalformedReply,
        detail: format!("the reply is not {expected}"),
        target: None,
    }
}

/// The plugin's settings, by key ([`Context::settings`]). A setting the
/// user did not set and that has no default is absent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings(serde_json::Map<String, Value>);

impl Settings {
    /// The setting `key` as `T` (`String`, `i64`, `bool`, ...); `None`
    /// when absent or of another type.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.0
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    pub fn integer(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(Value::as_i64)
    }

    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(Value::as_bool)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Every setting, as JSON.
    pub fn as_map(&self) -> &serde_json::Map<String, Value> {
        &self.0
    }

    /// Every setting as one JSON object text.
    pub fn to_json(&self) -> String {
        Value::Object(self.0.clone()).to_string()
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

/// The entries of one `link` header value: `(target, rel)` for each
/// `<target>; ...; rel="..."` (RFC 8288; entries without `rel` are
/// skipped). A comma inside `<...>` or a quoted string does not split.
fn link_entries(value: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut entries = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else { break };
        let target = &after[..close];
        let params_start = &after[close + 1..];
        // Up to the next entry: a comma outside a quoted string.
        let mut quoted = false;
        let mut end = params_start.len();
        for (i, c) in params_start.char_indices() {
            match c {
                '"' => quoted = !quoted,
                ',' if !quoted => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        let params = &params_start[..end];
        let rel = params.split(';').find_map(|param| {
            let (name, value) = param.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("rel")
                .then(|| value.trim().trim_matches('"'))
        });
        if let Some(rel) = rel {
            entries.push((target.trim(), rel));
        }
        rest = &params_start[end..];
    }
    entries.into_iter()
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

    pub fn put(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Put, url)
    }

    pub fn patch(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Patch, url)
    }

    pub fn delete(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Delete, url)
    }

    pub fn head(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Head, url)
    }

    /// The request's method.
    pub fn method(&self) -> HttpMethod {
        self.method
    }

    /// The request's URL.
    pub fn url(&self) -> &str {
        &self.url
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

    /// First header named `name` (ASCII case-insensitive). Only the
    /// headers the agent returns are here (see [`Context::fetch`]).
    ///
    /// ```
    /// use deckpilot_plugin_sdk::prelude::*;
    /// use deckpilot_plugin_sdk::Header;
    ///
    /// let r = Response {
    ///     status: 200,
    ///     headers: vec![Header { name: "x-ratelimit-remaining".into(), value: "4999".into() }],
    ///     body: None,
    /// };
    /// assert_eq!(r.header("X-RateLimit-Remaining"), Some("4999"));
    /// assert_eq!(r.header("set-cookie"), None); // never returned
    /// ```
    pub fn header(&self, name: &str) -> Option<&str> {
        self.header_values(name).next()
    }

    /// Every value of the headers named `name`, in order (a server may
    /// send `link` more than once).
    pub fn header_values<'r>(&'r self, name: &str) -> impl Iterator<Item = &'r str> + use<'r> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(move |h| h.name.eq_ignore_ascii_case(&name))
            .map(|h| h.value.as_str())
    }

    /// The target of the `link` header entry (RFC 8288) whose `rel` is
    /// `rel` (`next`, `prev`, `first`, `last`; ASCII case-insensitive),
    /// as the server sent it. Paginated APIs such as GitHub's say where the
    /// next page is this way. Fetch it like any URL: it must be on a
    /// granted origin too.
    ///
    /// ```
    /// use deckpilot_plugin_sdk::prelude::*;
    /// use deckpilot_plugin_sdk::Header;
    ///
    /// let link = r#"<https://api.github.com/repositories/1/pulls?page=2>; rel="next", <https://api.github.com/repositories/1/pulls?page=5>; rel="last""#;
    /// let r = Response {
    ///     status: 200,
    ///     headers: vec![Header { name: "link".into(), value: link.into() }],
    ///     body: None,
    /// };
    /// assert_eq!(r.link("next"), Some("https://api.github.com/repositories/1/pulls?page=2"));
    /// assert_eq!(r.link("last"), Some("https://api.github.com/repositories/1/pulls?page=5"));
    /// assert_eq!(r.link("prev"), None);
    /// ```
    pub fn link(&self, rel: &str) -> Option<&str> {
        self.header_values("link")
            .flat_map(link_entries)
            .find(|(_, rels)| {
                rels.split_ascii_whitespace()
                    .any(|r| r.eq_ignore_ascii_case(rel))
            })
            .map(|(target, _)| target)
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
                    "url.open" => f.write_str(
                        "Declare an `openUrl` permission for this exact https host in \
                         manifest.json; it applies once the user approves it. Only https URLs \
                         without userinfo are opened, never an IP address or a local name",
                    ),
                    "notify" => f.write_str(
                        "Declare the `notifications` permission in manifest.json; it applies \
                         once the user approves it",
                    ),
                    "credentials.has" => f.write_str(
                        "Declare the slot in `contributes.credentials` and in a `credentials` \
                         permission in manifest.json; it applies once the user approves it",
                    ),
                    "variables.publish" | "variables.clear" => f.write_str(
                        "Declare the variable in `contributes.variables` in manifest.json; \
                         publishing is allowed only while the plugin is enabled",
                    ),
                    "log" | "settings.get" | "storage.get" | "storage.set" | "storage.delete"
                    | "storage.keys" | "storage.usage" => f.write_str(
                        "It is always allowed while the plugin is enabled and has its \
                         grants; the plugin is disabled or being removed, or the agent \
                         refused its grant set (for example a credential slot whose host \
                         lost its network grant; see the agent log)",
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
    fn denials_of_always_on_ops_say_the_plugin_is_not_enabled() {
        let denied = |op| {
            CapabilityError::from_call(
                op,
                CallError {
                    code: CallErrorCode::NotPermitted,
                    message: "plugin has no grants".into(),
                },
            )
            .to_string()
        };
        for op in ["storage.set", "settings.get", "log"] {
            assert!(denied(op).contains("while the plugin is enabled"), "{op}");
        }
        assert!(denied("variables.publish").contains("contributes.variables"));
        assert!(denied("process.spawn").contains("not grantable"));
    }

    #[test]
    fn requests_have_every_method_and_typed_storage_reads() {
        use crate::testing::MockHost;
        for (r, m) in [
            (Request::put("https://a.example.com/"), HttpMethod::Put),
            (Request::patch("https://a.example.com/"), HttpMethod::Patch),
            (
                Request::delete("https://a.example.com/"),
                HttpMethod::Delete,
            ),
            (Request::head("https://a.example.com/"), HttpMethod::Head),
        ] {
            assert_eq!(r.method(), m);
            assert_eq!(r.url(), "https://a.example.com/");
        }
        let mut host = MockHost::new(|_| CallResult::Ok(serde_json::json!("text")));
        let mut ctx = host.context();
        assert_eq!(
            ctx.storage_get_as::<u32>("n").unwrap_err().kind(),
            CapabilityErrorKind::MalformedReply
        );
        assert_eq!(
            ctx.storage_get_as::<String>("n").unwrap().as_deref(),
            Some("text")
        );
    }

    #[test]
    fn log_lines_are_cut_at_character_boundaries() {
        let s = "é".repeat(MAX_LOG_LEN);
        let c = cut(&s, MAX_LOG_LEN);
        assert!(c.len() <= MAX_LOG_LEN);
        assert!(c.chars().all(|ch| ch == 'é'));
    }

    #[test]
    fn storage_credential_and_settings_wrappers_are_typed() {
        use crate::testing::MockHost;
        let mut host = MockHost::new(|req| match req {
            CapabilityRequest::StorageKeys(p) => {
                assert_eq!(p.prefix.as_deref(), Some("a."));
                CallResult::Ok(serde_json::json!(["a.1", "a.2"]))
            }
            CapabilityRequest::StorageUsage(_) => CallResult::Ok(serde_json::json!({
                "usedBytes": 12, "keys": 2, "quotaBytes": 262_144, "maxKeys": 1000
            })),
            CapabilityRequest::CredentialHas(c) if c.credential == "token" => {
                CallResult::Ok(Value::Bool(true))
            }
            CapabilityRequest::CredentialHas(_) => {
                CallResult::error(CallErrorCode::NotPermitted, "credential not granted")
            }
            CapabilityRequest::SettingsGet(_) => CallResult::Ok(serde_json::json!({
                "city": "berlin", "count": 3, "metric": true
            })),
            _ => CallResult::Ok(Value::Null),
        });
        let mut ctx = Context::new(&mut host);
        assert_eq!(ctx.storage_keys(Some("a.")).unwrap(), ["a.1", "a.2"]);
        let usage = ctx.storage_usage().unwrap();
        assert_eq!((usage.used_bytes, usage.keys), (12, 2));
        let token = SecretHandle::slot("token").unwrap();
        assert!(ctx.has_credential(&token).unwrap());
        let other = ctx
            .has_credential(&SecretHandle::slot("other").unwrap())
            .unwrap_err();
        assert!(other.is_denied());
        assert!(
            other.to_string().contains("contributes.credentials"),
            "{other}"
        );
        let settings = ctx.settings().unwrap();
        assert_eq!(settings.text("city"), Some("berlin"));
        assert_eq!(settings.integer("count"), Some(3));
        assert_eq!(settings.boolean("metric"), Some(true));
        assert_eq!(settings.get::<String>("count"), None);
        assert!(!settings.contains("missing"));
        assert_eq!(ctx.calls(), 5);
    }

    #[test]
    fn malformed_replies_are_reported() {
        use crate::testing::MockHost;
        let mut host = MockHost::new(|_| CallResult::Ok(serde_json::json!("nonsense")));
        let mut ctx = Context::new(&mut host);
        assert_eq!(
            ctx.settings().unwrap_err().kind(),
            CapabilityErrorKind::MalformedReply
        );
        assert_eq!(
            ctx.storage_keys(None).unwrap_err().kind(),
            CapabilityErrorKind::MalformedReply
        );
        assert_eq!(
            ctx.has_credential(&SecretHandle::slot("token").unwrap())
                .unwrap_err()
                .kind(),
            CapabilityErrorKind::MalformedReply
        );
    }

    #[test]
    fn open_url_sends_the_url_and_explains_denials() {
        use crate::testing::MockHost;
        let mut host = MockHost::new(|req| match req {
            CapabilityRequest::OpenUrl(o) if o.url.starts_with("https://github.com/") => {
                CallResult::Ok(Value::Null)
            }
            CapabilityRequest::OpenUrl(_) => {
                CallResult::error(CallErrorCode::NotPermitted, "origin not granted")
            }
            _ => CallResult::error(CallErrorCode::Throttled, "rate limited"),
        });
        let mut ctx = Context::new(&mut host);
        ctx.open_url("https://github.com/acme/app?tab=1").unwrap();
        let e = ctx
            .open_url("https://user:pw@evil.example.net/x?token=s3cret")
            .unwrap_err();
        assert!(e.is_denied());
        assert_eq!(e.op(), "url.open");
        // The target is the origin: no userinfo, path or query.
        assert_eq!(e.target(), Some("https://evil.example.net"));
        let text = e.to_string();
        assert!(!text.contains("s3cret") && !text.contains("pw@"), "{text}");
        assert!(text.contains("`openUrl` permission"), "{text}");
        assert_eq!(PluginError::from(e).code(), codes::NOT_PERMITTED);
        let CapabilityRequest::OpenUrl(sent) = &host.requests()[0] else {
            panic!()
        };
        assert_eq!(sent.url, "https://github.com/acme/app?tab=1");
    }

    #[test]
    fn link_headers_are_parsed() {
        let response = |values: &[&str]| Response {
            status: 200,
            headers: values
                .iter()
                .map(|v| Header {
                    name: "Link".into(),
                    value: (*v).into(),
                })
                .collect(),
            body: None,
        };
        let r = response(&[
            r#"<https://a.example.com/x?page=2&q=a,b>; rel="next", <https://a.example.com/x?page=9>; rel="last""#,
            r#"<https://a.example.com/x?page=1>; title="a, b; c"; rel="first prev""#,
        ]);
        assert_eq!(r.link("next"), Some("https://a.example.com/x?page=2&q=a,b"));
        assert_eq!(r.link("LAST"), Some("https://a.example.com/x?page=9"));
        assert_eq!(r.link("prev"), Some("https://a.example.com/x?page=1"));
        assert_eq!(r.link("first"), Some("https://a.example.com/x?page=1"));
        assert_eq!(r.header_values("link").count(), 2);
        assert_eq!(r.link("self"), None);
        // GitLab's form, unquoted rel, and garbage.
        let r = response(&["<https://g.example.com/p?page=3>; rel=next"]);
        assert_eq!(r.link("next"), Some("https://g.example.com/p?page=3"));
        for garbage in [
            "",
            "<",
            "<https://x.example.com/",
            "no brackets; rel=next",
            "<a>",
        ] {
            assert_eq!(response(&[garbage]).link("next"), None, "{garbage:?}");
        }
        assert_eq!(response(&[]).link("next"), None);
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
