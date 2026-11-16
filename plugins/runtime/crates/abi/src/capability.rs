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
    MAX_NOTIFY_REASON, MAX_NOTIFY_TITLE, MAX_OPEN_URL_LEN, MAX_RUN_FAILED_NODES, MAX_RUN_NODE_ID,
    MAX_RUN_WORKFLOW_NAME, MAX_STORAGE_KEY, MAX_STORAGE_VALUE, MAX_URL_LEN, MAX_VARIABLE_ID,
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
    /// Asks the agent to open an https URL in the user's browser (design
    /// §5.2 `openUrl`): only for an exact origin the user approved, never a
    /// `file:` or custom scheme, an IP literal or a local name, rate
    /// limited. The answer is `null`; the plugin never learns more than
    /// whether it was opened. Added compatibly within ABI 1.
    #[serde(rename = "url.open")]
    OpenUrl(OpenUrl),
    /// What started the invocation's run, and how the workflow run it is
    /// a node of is going ([`RunContext`], G8): the invocation's own run
    /// only. Always allowed. Added compatibly within ABI 1: a guest built
    /// before it never sends it, so nothing it receives changes; a host or
    /// agent that predates it refuses the unknown op.
    #[serde(rename = "run.context")]
    RunContext(NoArgs),
}

/// The `ok` value of `run.context` (G8). Unknown fields are **ignored**
/// (host to guest only), and a trigger or cause added later reads as
/// `other`, so the agent can add to it within ABI 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunContext {
    pub trigger: RunTrigger,
    /// For [`RunTrigger::Workflow`]: what started the workflow's run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<RunCause>,
    /// How the workflow run is going when this node starts; only for a
    /// node of a workflow graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunSummary>,
}

impl RunContext {
    /// A run that was not started by a workflow.
    pub fn direct(trigger: RunTrigger) -> Self {
        Self {
            trigger,
            cause: None,
            run: None,
        }
    }

    /// A node of a workflow run started by `cause`.
    pub fn workflow(cause: RunCause, run: Option<RunSummary>) -> Self {
        Self {
            trigger: RunTrigger::Workflow,
            cause: Some(cause),
            run,
        }
    }

    /// What nobody said anything about: not started by the user.
    pub fn unknown() -> Self {
        Self::direct(RunTrigger::Other)
    }

    /// Whether the user started the run directly: a key, a gesture, the
    /// app (the editor, the Action Library or the plugin's own page), or a
    /// workflow the user started that way. Never a schedule, an event,
    /// the agent's start, a work session or anything unknown. `url.open`
    /// is allowed only then.
    pub fn is_user_initiated(&self) -> bool {
        match self.trigger {
            RunTrigger::Key | RunTrigger::Gesture | RunTrigger::App | RunTrigger::Test => true,
            RunTrigger::Workflow => self.cause == Some(RunCause::User),
            RunTrigger::Other => false,
        }
    }

    /// The same context within the ABI's bounds: the workflow name and
    /// node ids cut, at most [`MAX_RUN_FAILED_NODES`] node ids.
    #[must_use]
    pub fn bounded(mut self) -> Self {
        if let Some(run) = &mut self.run {
            run.workflow = run
                .workflow
                .take()
                .map(|name| cut(&name, MAX_RUN_WORKFLOW_NAME));
            run.failed_nodes.truncate(MAX_RUN_FAILED_NODES);
            for node in &mut run.failed_nodes {
                *node = cut(node, MAX_RUN_NODE_ID);
            }
        }
        self
    }
}

fn cut(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// What started an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunTrigger {
    /// A key, dial or touch control of a device.
    Key,
    /// A gesture (double press, long press…) on a control.
    Gesture,
    /// The app: the profile editor or the Action Library.
    App,
    /// A node of a workflow run; [`RunContext::cause`] says what started
    /// the run.
    Workflow,
    /// The plugin's own page in the app, trying one of its actions.
    Test,
    /// Unknown, or a trigger this guest does not know yet.
    #[serde(other)]
    Other,
}

/// What started a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunCause {
    /// The user, from a key, a gesture or the app.
    User,
    /// One of the workflow's schedules.
    Schedule,
    /// A lifecycle event (a device connected, the project changed…).
    Event,
    /// The agent started.
    Startup,
    /// A work session's start or end-verification workflow.
    WorkSession,
    /// A cause this guest does not know yet.
    #[serde(other)]
    Other,
}

/// How the invocation's own workflow run is going when its node starts:
/// the workflow's name, counts and node ids. No messages, outputs,
/// parameters or anything of another workflow or plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    /// The workflow's name (at most [`MAX_RUN_WORKFLOW_NAME`] characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    /// Since the run started.
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Nodes finished so far, by status.
    #[serde(default)]
    pub succeeded: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub skipped: u32,
    #[serde(default)]
    pub cancelled: u32,
    /// Ids of the nodes that failed so far (the first
    /// [`MAX_RUN_FAILED_NODES`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_nodes: Vec<String>,
}

impl RunSummary {
    /// Whether a node of the run failed so far.
    pub fn has_failed(&self) -> bool {
        self.failed > 0
    }
}

/// A URL to open in the user's browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenUrl {
    /// An absolute `https://` URL, at most [`MAX_OPEN_URL_LEN`] bytes.
    pub url: String,
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

/// Response headers the agent hands to a plugin, by lowercase name: caching
/// and content metadata, pagination (`link`, GitLab's `x-total` family) and
/// rate limits (GitHub's `x-ratelimit-*`, GitLab's and the IETF draft's
/// `ratelimit-*`). Exact names only, no prefixes: a header is returned only
/// when it is listed here. Every other header is dropped, in particular
/// `set-cookie`, `www-authenticate`, `authorization`, `proxy-*` and
/// anything a server could use to echo a request's credentials; a value
/// that contains an injected secret is redacted before this filter runs.
/// Values longer than [`MAX_HEADER_LEN`] are dropped, and at most
/// [`MAX_HEADERS`] headers are returned.
pub const RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-language",
    "cache-control",
    "etag",
    "last-modified",
    "expires",
    "age",
    "vary",
    "retry-after",
    "date",
    "link",
    "x-total",
    "x-total-pages",
    "x-page",
    "x-per-page",
    "x-next-page",
    "x-prev-page",
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
    "x-ratelimit-used",
    "x-ratelimit-resource",
    "ratelimit",
    "ratelimit-policy",
    "ratelimit-limit",
    "ratelimit-remaining",
    "ratelimit-reset",
    "ratelimit-observed",
];

/// Whether a response header (any case) is handed to plugins
/// ([`RESPONSE_HEADERS`]).
pub fn is_response_header(name: &str) -> bool {
    RESPONSE_HEADERS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(name))
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
    /// How the notification is shown (G9): its colour and icon only. It
    /// never changes where it goes, its rate or its plugin-name prefix.
    /// Added compatibly within ABI 1: optional, and omitted from the JSON
    /// when absent, so requests without it are byte-identical to the
    /// original form, which an agent shows as `info`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<NotifySeverity>,
}

/// The severity of a plugin notification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotifySeverity {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

/// The `ok` value of `notify` for a guest that reports
/// [`NOTIFY_DELIVERY`](crate::guest::features::NOTIFY_DELIVERY) (G9):
/// where the agent showed the notification, or why it showed it nowhere.
/// Any other guest gets `null`, as before. A notification that could not
/// be shown is still an `ok` reply (the request was fine); not granted
/// and the rate limit stay errors.
///
/// Unknown fields are **ignored** (host to guest only), so the agent can
/// add to it within ABI 1.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyDelivery {
    /// Shown somewhere: on the desktop, in the status bar or both.
    pub shown: bool,
    /// Shown as a notification of the operating system.
    #[serde(default)]
    pub desktop: bool,
    /// Shown in the status bar of an open DeckPilot window.
    #[serde(default)]
    pub status_bar: bool,
    /// Why it was not shown, in words for the user (never set when shown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl NotifyDelivery {
    /// Shown where `desktop` and `status_bar` say (at least one).
    pub fn shown(desktop: bool, status_bar: bool) -> Self {
        Self {
            shown: desktop || status_bar,
            desktop,
            status_bar,
            reason: None,
        }
    }

    /// Shown nowhere, because of `reason` (cut to
    /// [`MAX_NOTIFY_REASON`] characters).
    pub fn not_shown(reason: &str) -> Self {
        Self {
            reason: Some(cut(reason, MAX_NOTIFY_REASON)),
            ..Self::default()
        }
    }
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
            Self::OpenUrl(_) => "url.open",
            Self::RunContext(_) => "run.context",
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
            Self::StorageUsage(_) | Self::SettingsGet(_) | Self::RunContext(_) => {}
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
            Self::OpenUrl(o) => {
                if o.url.is_empty() || o.url.len() > MAX_OPEN_URL_LEN {
                    return Err(RequestError::TooLarge("url"));
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
    /// What went wrong on the network, for a failed `http.fetch`
    /// (compatible addition within ABI 1). Only a guest that reports
    /// [`features::NETWORK_ERROR_KIND`](crate::guest::features::NETWORK_ERROR_KIND)
    /// from [`EXPORT_FEATURES`](crate::guest::EXPORT_FEATURES) ever
    /// receives it: the host removes it for every other guest, whose
    /// `CallError` refuses unknown fields ([`CallResult::for_features`]).
    /// It is only the kind: never a host name, an address, a URL or
    /// anything else of the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<NetworkErrorKind>,
}

/// Why a brokered `http.fetch` failed on the network, as the built-in
/// modules' transport (`deckpilot-net`) names it: the serialized names are
/// its error codes, so a plugin can report the same code as the built-in
/// module it replaces ([`NetworkErrorKind::code`]).
///
/// A kind added later reaches a guest built before it as
/// [`NetworkErrorKind::Other`], never as a malformed reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkErrorKind {
    /// The host name could not be resolved.
    DnsFailed,
    /// No connection could be made (refused, reset, unreachable).
    ConnectionFailed,
    /// The certificate or the TLS handshake was refused.
    TlsFailed,
    /// No answer within the time limit.
    TimedOut,
    /// More redirects than one fetch may follow.
    TooManyRedirects,
    /// A redirect from https to plain http.
    InsecureRedirect,
    /// The host resolves to an address on this computer or a private
    /// network, which plugins never reach.
    BlockedAddress,
    /// A kind this guest does not know yet.
    #[serde(other)]
    Other,
}

impl NetworkErrorKind {
    /// Every kind but [`NetworkErrorKind::Other`].
    pub const ALL: [Self; 7] = [
        Self::DnsFailed,
        Self::ConnectionFailed,
        Self::TlsFailed,
        Self::TimedOut,
        Self::TooManyRedirects,
        Self::InsecureRedirect,
        Self::BlockedAddress,
    ];

    /// The built-in modules' error code for this kind, e.g. `dns-failed`
    /// (`request-failed` for [`NetworkErrorKind::Other`]).
    pub const fn code(self) -> &'static str {
        match self {
            Self::DnsFailed => "dns-failed",
            Self::ConnectionFailed => "connection-failed",
            Self::TlsFailed => "tls-failed",
            Self::TimedOut => "timed-out",
            Self::TooManyRedirects => "too-many-redirects",
            Self::InsecureRedirect => "insecure-redirect",
            Self::BlockedAddress => "blocked-address",
            Self::Other => "request-failed",
        }
    }

    /// The kind of a transport error code; `None` for a code that is not a
    /// network failure a plugin is told about.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }

    /// Whether trying again later may help (no answer, no connection, a
    /// name that did not resolve), unlike a refusal a retry meets again.
    pub const fn is_transient(self) -> bool {
        matches!(
            self,
            Self::DnsFailed | Self::ConnectionFailed | Self::TimedOut | Self::Other
        )
    }
}

impl fmt::Display for NetworkErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
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
            kind: None,
        })
    }

    /// An error with the network kind of a failed fetch (`None`: none).
    pub fn error_with_kind(
        code: CallErrorCode,
        message: impl Into<String>,
        kind: Option<NetworkErrorKind>,
    ) -> Self {
        Self::Error(CallError {
            code,
            message: message.into(),
            kind,
        })
    }

    /// The reply as a guest that reports `features` (from `dp_features`;
    /// 0 without it) can read it: an error's `kind` only with
    /// [`features::NETWORK_ERROR_KIND`](crate::guest::features::NETWORK_ERROR_KIND).
    #[must_use]
    pub fn for_features(self, features: u64) -> Self {
        match self {
            Self::Error(mut error)
                if features & crate::guest::features::NETWORK_ERROR_KIND == 0 =>
            {
                error.kind = None;
                Self::Error(error)
            }
            other => other,
        }
    }

    /// [`CallResult::for_features`] for a reply to the operation `op`:
    /// also, a `notify` is answered `null` unless the guest reports
    /// [`NOTIFY_DELIVERY`](crate::guest::features::NOTIFY_DELIVERY), which
    /// is exactly what guests built before it received.
    #[must_use]
    pub fn for_op(self, op: &str, features: u64) -> Self {
        match self.for_features(features) {
            Self::Ok(_)
                if op == "notify" && features & crate::guest::features::NOTIFY_DELIVERY == 0 =>
            {
                Self::Ok(Value::Null)
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_kinds_are_an_optional_compatible_addition() {
        // Without a kind: exactly the shape guests built before it read.
        let plain = CallResult::error(CallErrorCode::Failed, "no route");
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"error":{"code":"failed","message":"no route"}}"#
        );
        let parsed: CallResult =
            serde_json::from_str(r#"{"error":{"code":"failed","message":"x"}}"#).unwrap();
        assert!(matches!(
            parsed,
            CallResult::Error(CallError { kind: None, .. })
        ));

        // With one: the transport's code, and nothing else.
        for kind in NetworkErrorKind::ALL {
            let r = CallResult::error_with_kind(CallErrorCode::Failed, "m", Some(kind));
            let json = serde_json::to_string(&r).unwrap();
            assert_eq!(
                json,
                format!(
                    r#"{{"error":{{"code":"failed","message":"m","kind":"{}"}}}}"#,
                    kind.code()
                )
            );
            assert_eq!(serde_json::from_str::<CallResult>(&json).unwrap(), r);
            assert_eq!(NetworkErrorKind::from_code(kind.code()), Some(kind));
            assert_eq!(kind.to_string(), kind.code());
        }
        for code in ["request-failed", "cancelled", "invalid-url", "other", ""] {
            assert_eq!(NetworkErrorKind::from_code(code), None, "{code}");
        }

        // A kind added later is `other` for this guest, not a malformed
        // reply; and a kind is a closed name, never free text.
        let later: CallResult = serde_json::from_str(
            r#"{"error":{"code":"failed","message":"m","kind":"quic-failed"}}"#,
        )
        .unwrap();
        assert!(matches!(
            later,
            CallResult::Error(CallError {
                kind: Some(NetworkErrorKind::Other),
                ..
            })
        ));
        assert_eq!(NetworkErrorKind::Other.code(), "request-failed");
        assert!(
            serde_json::from_str::<CallResult>(
                r#"{"error":{"code":"failed","message":"m","kind":{"host":"10.0.0.1"}}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn guests_without_the_feature_never_see_a_kind() {
        let r = CallResult::error_with_kind(
            CallErrorCode::Failed,
            "m",
            Some(NetworkErrorKind::DnsFailed),
        );
        assert_eq!(
            serde_json::to_string(&r.clone().for_features(0)).unwrap(),
            r#"{"error":{"code":"failed","message":"m"}}"#
        );
        assert_eq!(
            r.clone()
                .for_features(crate::guest::features::NETWORK_ERROR_KIND),
            r
        );
        // Bits this crate does not define change nothing.
        assert_eq!(
            serde_json::to_string(&r.clone().for_features(1 << 40)).unwrap(),
            r#"{"error":{"code":"failed","message":"m"}}"#
        );
        let ok = CallResult::Ok(Value::Null);
        assert_eq!(ok.clone().for_features(0), ok);
    }

    fn parse(json: &str) -> Result<CapabilityRequest, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn run_context_is_a_request_without_arguments() {
        let json = r#"{"op":"run.context","args":{}}"#;
        let r = parse(json).unwrap();
        assert_eq!(r, CapabilityRequest::RunContext(NoArgs {}));
        assert_eq!(r.op(), "run.context");
        r.validate().unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), json);
        for bad in [
            r#"{"op":"run.context","args":{"plugin":"other"}}"#,
            r#"{"op":"run.context","args":{"workflow":"w"}}"#,
            r#"{"op":"run.context"}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn run_contexts_round_trip_and_tolerate_additions() {
        let key = RunContext::direct(RunTrigger::Key);
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, r#"{"trigger":"key"}"#);
        assert_eq!(serde_json::from_str::<RunContext>(&json).unwrap(), key);

        let run = RunSummary {
            workflow: Some("Deploy".into()),
            elapsed_ms: 1_200,
            succeeded: 3,
            failed: 1,
            skipped: 0,
            cancelled: 0,
            failed_nodes: vec!["push".into()],
        };
        let scheduled = RunContext::workflow(RunCause::Schedule, Some(run.clone()));
        let json = serde_json::to_string(&scheduled).unwrap();
        assert_eq!(
            json,
            r#"{"trigger":"workflow","cause":"schedule","run":{"workflow":"Deploy","elapsedMs":1200,"succeeded":3,"failed":1,"skipped":0,"cancelled":0,"failedNodes":["push"]}}"#
        );
        assert_eq!(
            serde_json::from_str::<RunContext>(&json).unwrap(),
            scheduled
        );
        assert!(run.has_failed());

        // What a later agent may add reaches this guest as `other` or is
        // ignored, never as a malformed reply.
        let later: RunContext = serde_json::from_str(
            r#"{"trigger":"voice","cause":"webhook","run":{"nodes":9},"profile":"p"}"#,
        )
        .unwrap();
        assert_eq!(later.trigger, RunTrigger::Other);
        assert_eq!(later.cause, Some(RunCause::Other));
        assert_eq!(later.run, Some(RunSummary::default()));
        assert!(!later.is_user_initiated());
    }

    #[test]
    fn only_runs_the_user_started_count_as_user_initiated() {
        for trigger in [
            RunTrigger::Key,
            RunTrigger::Gesture,
            RunTrigger::App,
            RunTrigger::Test,
        ] {
            assert!(
                RunContext::direct(trigger).is_user_initiated(),
                "{trigger:?}"
            );
        }
        assert!(RunContext::workflow(RunCause::User, None).is_user_initiated());
        for cause in [
            RunCause::Schedule,
            RunCause::Event,
            RunCause::Startup,
            RunCause::WorkSession,
            RunCause::Other,
        ] {
            assert!(
                !RunContext::workflow(cause, None).is_user_initiated(),
                "{cause:?}"
            );
        }
        assert!(!RunContext::unknown().is_user_initiated());
        // A workflow without a cause is not the user's.
        let no_cause = RunContext {
            cause: None,
            ..RunContext::workflow(RunCause::User, None)
        };
        assert!(!no_cause.is_user_initiated());
    }

    #[test]
    fn run_summaries_are_bounded() {
        let run = RunSummary {
            workflow: Some("w".repeat(MAX_RUN_WORKFLOW_NAME + 10)),
            failed_nodes: (0..MAX_RUN_FAILED_NODES + 5)
                .map(|i| format!("{i}{}", "n".repeat(MAX_RUN_NODE_ID)))
                .collect(),
            ..RunSummary::default()
        };
        let bounded = RunContext::workflow(RunCause::User, Some(run)).bounded();
        let run = bounded.run.unwrap();
        assert_eq!(run.workflow.unwrap().chars().count(), MAX_RUN_WORKFLOW_NAME);
        assert_eq!(run.failed_nodes.len(), MAX_RUN_FAILED_NODES);
        assert!(
            run.failed_nodes
                .iter()
                .all(|n| n.chars().count() <= MAX_RUN_NODE_ID)
        );
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
            severity: None,
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
    fn open_url_is_a_strict_compatible_addition() {
        let json = r#"{"op":"url.open","args":{"url":"https://github.com/acme/app/pull/1"}}"#;
        let r = parse(json).unwrap();
        assert_eq!(r.op(), "url.open");
        r.validate().unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), json);
        for bad in [
            r#"{"op":"url.open","args":{}}"#,
            r#"{"op":"url.open","args":{"url":"https://github.com/","plugin":"o"}}"#,
            r#"{"op":"url.open","args":{"url":"https://github.com/","app":"Safari"}}"#,
            r#"{"op":"url.open","args":{"url":["https://github.com/"]}}"#,
            r#"{"op":"open","args":{"url":"https://github.com/"}}"#,
            r#"{"op":"shell.open","args":{"url":"https://github.com/"}}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let long = CapabilityRequest::OpenUrl(OpenUrl {
            url: format!("https://github.com/{}", "a".repeat(MAX_OPEN_URL_LEN)),
        });
        assert_eq!(long.validate(), Err(RequestError::TooLarge("url")));
        let empty = CapabilityRequest::OpenUrl(OpenUrl { url: String::new() });
        assert_eq!(empty.validate(), Err(RequestError::TooLarge("url")));
    }

    #[test]
    fn response_headers_never_include_credentials_or_cookies() {
        for allowed in RESPONSE_HEADERS {
            assert_eq!(*allowed, allowed.to_ascii_lowercase());
            assert!(is_response_header(allowed));
            assert!(is_response_header(&allowed.to_ascii_uppercase()));
        }
        for name in [
            "link",
            "x-ratelimit-remaining",
            "ratelimit-reset",
            "x-total",
            "x-total-pages",
            "x-next-page",
            "etag",
            "retry-after",
        ] {
            assert!(is_response_header(name), "{name}");
        }
        for name in [
            "set-cookie",
            "set-cookie2",
            "cookie",
            "authorization",
            "proxy-authorization",
            "www-authenticate",
            "proxy-authenticate",
            "authentication-info",
            "x-api-key",
            "private-token",
            "x-auth-token",
            "x-oauth-scopes",
            "location",
            "content-location",
            "x-ratelimit-token",
            "ratelimit-",
            "x-",
            "",
        ] {
            assert!(!is_response_header(name), "{name}");
        }
        let unique: std::collections::BTreeSet<_> = RESPONSE_HEADERS.iter().collect();
        assert_eq!(unique.len(), RESPONSE_HEADERS.len());
        assert!(RESPONSE_HEADERS.len() <= MAX_HEADERS);
    }

    #[test]
    fn notification_severity_is_an_optional_compatible_addition() {
        let old = r#"{"op":"notify","args":{"title":"t","body":"b"}}"#;
        let r = parse(old).unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), old);
        let CapabilityRequest::Notify(n) = &r else {
            panic!()
        };
        assert_eq!(n.severity, None);
        for (text, severity) in [
            ("info", NotifySeverity::Info),
            ("success", NotifySeverity::Success),
            ("warning", NotifySeverity::Warning),
            ("error", NotifySeverity::Error),
        ] {
            let json = format!(
                r#"{{"op":"notify","args":{{"title":"t","body":"b","severity":"{text}"}}}}"#
            );
            let r = parse(&json).unwrap();
            r.validate().unwrap();
            assert_eq!(serde_json::to_string(&r).unwrap(), json);
            let CapabilityRequest::Notify(n) = r else {
                panic!()
            };
            assert_eq!(n.severity, Some(severity));
        }
        for bad in [
            r#"{"op":"notify","args":{"title":"t","body":"b","severity":"critical"}}"#,
            r#"{"op":"notify","args":{"title":"t","body":"b","severity":"Error"}}"#,
            r#"{"op":"notify","args":{"title":"t","body":"b","severity":3}}"#,
            r#"{"op":"notify","args":{"title":"t","body":"b","critical":true}}"#,
            r#"{"op":"notify","args":{"title":"t","body":"b","desktop":true}}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn notify_deliveries_reach_only_guests_that_read_them() {
        use crate::guest::features::{NETWORK_ERROR_KIND, NOTIFY_DELIVERY};
        let shown = NotifyDelivery::shown(true, false);
        assert_eq!(
            serde_json::to_string(&shown).unwrap(),
            r#"{"shown":true,"desktop":true,"statusBar":false}"#
        );
        let nowhere = NotifyDelivery::not_shown(&"x".repeat(500));
        assert!(!nowhere.shown && !nowhere.desktop && !nowhere.status_bar);
        assert_eq!(
            nowhere.reason.as_deref().map(str::len),
            Some(MAX_NOTIFY_REASON)
        );
        // Additions later are ignored; `shown` is required.
        let later: NotifyDelivery =
            serde_json::from_str(r#"{"shown":true,"statusBar":true,"sound":"ping"}"#).unwrap();
        assert_eq!(later, NotifyDelivery::shown(false, true));
        assert!(serde_json::from_str::<NotifyDelivery>(r#"{"desktop":true}"#).is_err());

        let reply = || CallResult::Ok(serde_json::to_value(&shown).unwrap());
        // A guest built before it: `null`, byte for byte the old reply.
        for features in [0, NETWORK_ERROR_KIND] {
            assert_eq!(
                serde_json::to_string(&reply().for_op("notify", features)).unwrap(),
                r#"{"ok":null}"#
            );
        }
        assert_eq!(reply().for_op("notify", NOTIFY_DELIVERY), reply());
        // Other operations and errors are untouched.
        assert_eq!(reply().for_op("settings.get", 0), reply());
        let throttled = CallResult::error(CallErrorCode::Throttled, "5 per minute");
        assert_eq!(throttled.clone().for_op("notify", 0), throttled);
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
