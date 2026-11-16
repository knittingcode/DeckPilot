//! Carries out the capability requests the broker allowed (design §5).
//!
//! The broker decides; this executor only performs what it was handed:
//!
//! * `http.fetch` with the `deckpilot-net` client: exactly the normalized
//!   URL the broker allowed, addresses resolved once, checked against the
//!   private-network policy (loopback, private, link-local, metadata and
//!   other non-public addresses refused) and pinned for the connection, no
//!   proxy, a bounded body and a timeout. Only a request the broker
//!   allowed within a user-entered origin the user allowed on a private
//!   network (G2, `AllowedFetch::private_network`) may also connect to
//!   private ranges (RFC 1918, carrier NAT, unique local IPv6); loopback
//!   and link-local stay refused for it too. Redirects are never followed by
//!   the client: each one goes back to the broker
//!   (`Session::authorize_redirect`), which re-checks the origin, method,
//!   headers and hop count exactly like a new request. A request stops at
//!   once when its invocation is aborted or runs out of time
//!   (`CallInterrupt`): the connection is dropped and nothing it would
//!   still have answered reaches the plugin.
//! * `variables.publish` / `variables.clear` into the plugin's own values
//!   (`variables.rs`): only ids its active manifest declares, of the
//!   declared type, machine scope only, within the generation the
//!   invocation started in.
//! * `notify` through the agent's notifier, titled with the plugin's name:
//!   in the status bar of an open app, and (G9) where the user's settings
//!   for the plugin say when its manifest declares a notification routing
//!   (`permissions[notifications].routing`): on the desktop, in the status
//!   bar, from a least severity on. The agent reads those settings; the
//!   plugin only learns where it was shown ([`NotifyDelivery`], for guests
//!   that read it).
//! * `log` into the agent's log, attributed to the plugin.
//! * Storage (`storage.*`) in the plugin's own rows of plugins.db
//!   ([`PluginData`]), within the quota the broker passed with the call.
//! * Credentials as handles (design §5.5): for an `http.fetch` with
//!   `auth`, the secret of the slot (allowed by the broker for this origin
//!   and scheme) is read from plugins.db and injected by the HTTP client
//!   into this request only, header or query parameter as the slot
//!   declares, again on a same-origin redirect the broker re-allowed.
//!   The secret never reaches the guest: it is replaced by `[redacted]`
//!   in the response's body and headers and in error messages, it is
//!   never logged, and `credentials.has` answers only `true` or `false`.
//! * `settings.get`: the saved settings, validated against the manifest,
//!   defaults filled in.
//! * `url.open`: exactly the normalized https URL the broker allowed for a
//!   granted origin, re-checked here, opened with the operating system's
//!   handler through the core launcher (`launch::plan(OpenTarget::Url)`,
//!   no shell), at most [`MAX_OPENS_PER_INVOCATION`] times per invocation,
//!   and only in a run the user started (a key, a gesture, the app, a
//!   workflow the user started): the broker refuses it in any other run,
//!   and the executor checks its own run context again before opening.
//!   Only the origin is logged, never the path or query.
//! * `run.context` (G8): what started the invocation's run and, for a
//!   workflow node, how the run is going ([`RunContext`]); the context the
//!   agent handed this executor, never anything the guest said.
//! * Network failures: the transport's code for what went wrong (DNS,
//!   connection, TLS, time limit, a blocked address) travels to the guest
//!   as the error's network kind
//!   ([`NetworkErrorKind`](deckpilot_plugin_abi::NetworkErrorKind)), the
//!   same codes the built-in modules report; a redirect the broker refused
//!   as too many or from https to http, likewise. The kind is a closed
//!   name, never text of the request; the host passes it on only to
//!   guests that read it.
//! * Response headers: only the ABI's allowlist
//!   ([`RESPONSE_HEADERS`](deckpilot_plugin_abi::capability::RESPONSE_HEADERS):
//!   caching, pagination and rate-limit headers, never `set-cookie` or an
//!   authentication header), after an injected secret was redacted from
//!   every value.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use deckpilot_core::notify::{Delivery, Notification, Notifier, Severity};
use deckpilot_ipc::AgentEvent;
use deckpilot_plugin_abi::capability::{
    AuthScheme, Header, HttpResponse, LogLevel, NotifyDelivery, NotifySeverity, RunContext,
    StorageUsage, is_response_header,
};
use deckpilot_plugin_abi::limits::{MAX_HEADER_LEN, MAX_HEADERS};
use deckpilot_plugin_abi::{CallErrorCode, CallResult, HttpMethod, NetworkErrorKind};
use deckpilot_plugin_broker::{
    Allowed, AllowedFetch, Broker, Decision, PluginId, StorageQuota as GrantedQuota,
};
use deckpilot_plugin_manifest::{Manifest, Variable, VariableScope};
use deckpilot_plugin_store::{PluginData, Secret, StorageError, StorageQuota};
use deckpilot_plugin_supervisor::{CallInterrupt, CapabilityExecutor};
use serde_json::Value;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use super::variables::{PublishedVariables, typed};

/// The plugin data connection the executors share; `None` once the store
/// is closed (configuration locked).
pub(super) type SharedData = Arc<Mutex<Option<PluginData>>>;

/// What an executor needs for storage, credentials and settings: the data
/// connection and the manifest of the version that runs.
#[derive(Clone)]
pub(super) struct DataAccess {
    pub data: SharedData,
    pub manifest: Arc<Manifest>,
}

/// What replaces a secret wherever it could reach the guest or a log.
const REDACTED: &str = "[redacted]";

/// Longest a fetch (all hops) may take.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Largest response body handed to a plugin (the reply must fit into one
/// `host_call` reply, `MAX_HOST_CALL_REPLY`).
const MAX_BODY: usize = 512 * 1024;
/// URLs one invocation may open (on top of the broker's rate limits).
const MAX_OPENS_PER_INVOCATION: u32 = 3;

/// How allowed `url.open` requests are carried out: the operating system's
/// handler for https URLs, or (agent tests only, feature `test-hooks`) a
/// file that records them, so that a test never starts a browser.
#[derive(Debug, Clone)]
pub(super) enum Opener {
    System,
    #[cfg(feature = "test-hooks")]
    Recorded(Arc<std::path::PathBuf>),
}

impl Opener {
    /// The system's handler, unless an agent test asked for a record.
    pub fn from_env() -> Self {
        #[cfg(feature = "test-hooks")]
        if let Some(path) = std::env::var_os(test_opener::ENV) {
            return Self::Recorded(Arc::new(path.into()));
        }
        Self::System
    }

    fn open(&self, url: &url::Url) -> Result<(), String> {
        match self {
            Self::System => {
                use deckpilot_core::launch::{self, Launcher as _, OpenTarget, SystemLauncher};
                let plan = launch::plan(
                    deckpilot_core::Os::current(),
                    &OpenTarget::Url(url.to_string()),
                )
                .map_err(|e| e.to_string())?;
                SystemLauncher.launch(&plan).map_err(|e| e.to_string())
            }
            #[cfg(feature = "test-hooks")]
            Self::Recorded(path) => test_opener::record(path, url),
        }
    }
}

/// The URL is what the broker allowed for `origin`: https, no userinfo,
/// the origin's host and port. The broker guarantees it; the executor
/// refuses anything else before a launcher sees it.
fn openable(url: &url::Url, origin: &deckpilot_plugin_broker::Origin) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.host(), Some(url::Host::Domain(host)) if host == origin.host())
        && url.port_or_known_default() == Some(origin.port())
        && url.as_str().len() <= deckpilot_plugin_abi::limits::MAX_OPEN_URL_LEN
}

/// One response, before it is handed to the plugin.
#[derive(Debug, Clone)]
pub(super) struct Exchange {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Where fetches go: the network, or (agent tests only, feature
/// `test-hooks`) canned responses.
#[derive(Debug, Clone)]
pub(super) enum Transport {
    Network,
    #[cfg(feature = "test-hooks")]
    Canned(std::sync::Arc<test_transport::Canned>),
}

impl Transport {
    /// The network, unless an agent test configured canned responses.
    pub fn from_env() -> Self {
        #[cfg(feature = "test-hooks")]
        if let Some(canned) = test_transport::Canned::from_env() {
            return Self::Canned(std::sync::Arc::new(canned));
        }
        Self::Network
    }

    /// Sends one hop. Returns at once, dropping the request, when
    /// `interrupt` is raised (the invocation was aborted or ran out of
    /// time).
    fn send(
        &self,
        fetch: &AllowedFetch,
        auth: Option<&Auth>,
        interrupt: Option<&CallInterrupt>,
    ) -> Result<Exchange, SendFailure> {
        match self {
            Self::Network => network_send(fetch, auth, interrupt),
            #[cfg(feature = "test-hooks")]
            Self::Canned(canned) => canned.send(fetch, auth, interrupt),
        }
    }
}

/// A credential to inject into one request: the HTTP client's own type,
/// whose `Debug` never shows the secret.
pub(super) type Auth = deckpilot_net::client::Auth;

/// `secret` as `application/x-www-form-urlencoded` (how a query
/// parameter carries it).
fn form_encoded(secret: &str) -> String {
    let mut out = String::with_capacity(secret.len());
    for byte in secret.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            other => {
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

/// Standard padded base64 (how `Basic` carries `user:password`).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Every form in which `secret` could come back from a request: as it is,
/// form-encoded and base64 (`Basic`).
fn secret_forms(secret: &str) -> Vec<String> {
    let mut forms = vec![
        secret.to_owned(),
        form_encoded(secret),
        base64(secret.as_bytes()),
    ];
    forms.retain(|f| f.len() >= 4);
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms.dedup();
    forms
}

fn redact(text: &str, forms: &[String]) -> String {
    forms.iter().fold(text.to_owned(), |text, form| {
        text.replace(form.as_str(), REDACTED)
    })
}

/// How often a running request looks at its invocation's interrupt.
const INTERRUPT_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// What a request that was interrupted reports (the supervisor replaces it
/// for the guest anyway).
const INTERRUPTED: &str = "the action was cancelled or ran out of time; the request was stopped";

/// Why one hop got no answer: the message for the plugin and, for a
/// network failure, its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SendFailure {
    pub message: String,
    pub kind: Option<NetworkErrorKind>,
}

impl SendFailure {
    fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: None,
        }
    }

    /// From the transport's `(code, message)`. A blocked address is
    /// named by the granted origin only, never by the address it resolved
    /// to (the transport's message names it and the app's settings, which
    /// do not apply to plugins).
    fn from_transport((code, message): (&str, String), fetch: &AllowedFetch) -> Self {
        let kind = NetworkErrorKind::from_code(code);
        let message = match kind {
            Some(NetworkErrorKind::BlockedAddress) if fetch.private_network => format!(
                "{} resolves to an address on this computer or a link-local or special \
                 network, which plugins never reach",
                fetch.origin
            ),
            Some(NetworkErrorKind::BlockedAddress) => format!(
                "{} resolves to an address on this computer or a private network, which \
                 plugins reach only for a server the user entered and allowed on a private \
                 network",
                fetch.origin
            ),
            _ => message,
        };
        Self { message, kind }
    }
}

/// The addresses a request within a user origin the user allowed on a
/// private network (G2) may connect to: public ones and private ranges
/// (`check_private_network_addr`), never this computer or link-local. Every
/// address the name resolves to must pass, and the client connects to
/// exactly these, so DNS cannot swap in another one.
pub(super) struct PrivateNetworkOrigin;

impl deckpilot_net::policy::AddressPolicy for PrivateNetworkOrigin {
    fn check(
        &self,
        host: &str,
        addresses: &[std::net::IpAddr],
    ) -> Result<(), (&'static str, String)> {
        use deckpilot_net::codes;
        if addresses.is_empty() {
            return Err((codes::DNS_FAILED, format!("{host} did not resolve")));
        }
        match addresses
            .iter()
            .find(|a| deckpilot_plugin_broker::check_private_network_addr(**a).is_err())
        {
            Some(address) => Err((
                codes::BLOCKED_ADDRESS,
                format!("{host} is on this computer or a link-local network ({address})"),
            )),
            None => Ok(()),
        }
    }
}

/// The address check of one hop: public addresses only, unless the broker
/// allowed it within a user origin on a private network.
pub(super) fn address_policy(
    fetch: &AllowedFetch,
) -> Box<dyn deckpilot_net::policy::AddressPolicy> {
    if fetch.private_network {
        Box::new(PrivateNetworkOrigin)
    } else {
        // No `privateNetwork` grant of the G3 kind is implemented: every
        // resolved address must be public.
        Box::new(deckpilot_net::policy::NetworkPolicy {
            allow_private: false,
            private_hosts: Vec::new(),
        })
    }
}

fn method(method: HttpMethod) -> reqwest::Method {
    match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Head => reqwest::Method::HEAD,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
    }
}

/// The body's content type from the plugin's `Content-Type` header (one of
/// a few known types), which the client sets itself.
fn content_type(headers: &[Header]) -> &'static str {
    let declared = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase());
    match declared
        .as_deref()
        .map(|v| v.split(';').next().unwrap_or_default().trim())
    {
        Some("text/plain") => "text/plain; charset=utf-8",
        Some("application/x-www-form-urlencoded") => "application/x-www-form-urlencoded",
        Some("application/octet-stream") => "application/octet-stream",
        _ => "application/json",
    }
}

fn network_send(
    fetch: &AllowedFetch,
    auth: Option<&Auth>,
    interrupt: Option<&CallInterrupt>,
) -> Result<Exchange, SendFailure> {
    use deckpilot_net::client;
    let request = client::Request {
        method: method(fetch.method),
        url: fetch.url.clone(),
        headers: fetch
            .headers
            .iter()
            .filter(|h| !h.name.eq_ignore_ascii_case("content-type"))
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect(),
        body: fetch
            .body
            .clone()
            .map(|body| (body, content_type(&fetch.headers))),
        // Only ever a secret the broker allowed for this origin and scheme
        // (`Executor::fetch`); the client sends it over https only.
        auth: auth.cloned(),
        timeout: FETCH_TIMEOUT,
        // Every redirect goes back to the broker.
        follow_redirects: false,
        max_bytes: MAX_BODY,
    };
    // Public addresses only, unless the broker allowed this hop within a
    // user origin the user allowed on a private network; the connection
    // uses only the checked addresses.
    let policy = address_policy(fetch);
    // The client stops at once when the token is cancelled (it drops the
    // connection); a watcher cancels it when the invocation is interrupted.
    let token = deckpilot_core::CancellationToken::new();
    let done = std::sync::atomic::AtomicBool::new(false);
    let response = std::thread::scope(|scope| {
        if let Some(interrupt) = interrupt {
            let watcher = std::thread::Builder::new()
                .name("plugin fetch interrupt".into())
                .spawn_scoped(scope, || {
                    while !done.load(std::sync::atomic::Ordering::SeqCst) {
                        if interrupt.is_raised() {
                            token.cancel();
                            return;
                        }
                        std::thread::sleep(INTERRUPT_POLL);
                    }
                });
            if let Err(error) = watcher {
                warn!(%error, "a plugin request cannot be interrupted");
            }
        }
        let response = client::send(request, policy.as_ref(), Some(&token));
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        response
    });
    if token.is_cancelled() {
        return Err(SendFailure::other(INTERRUPTED));
    }
    let response = response.map_err(|failure| SendFailure::from_transport(failure, fetch))?;
    if response.truncated {
        return Err(SendFailure::other(format!(
            "the response is larger than {MAX_BODY} bytes"
        )));
    }
    Ok(Exchange {
        status: response.status,
        headers: response.headers,
        body: response.body,
    })
}

/// What an invocation may do with the plugin's own variables: publish the
/// declared ones into the store, within the generation it started in.
pub(super) struct OwnVariables {
    pub store: PublishedVariables,
    /// `plugins.<publisherCamel>.<nameCamel>`.
    pub namespace: String,
    pub generation: u64,
    /// The active version's `contributes.variables`.
    pub declared: Vec<Variable>,
}

/// The executor for one plugin's invocations.
pub(super) struct Executor {
    broker: Broker,
    /// The plugin's display name, for notifications.
    name: String,
    events: broadcast::Sender<AgentEvent>,
    /// The agent's notifier: the status bar of open app windows and the
    /// desktop ([`Executor::with_notifier`]); by default the status bar
    /// only, through `events`.
    notifier: Arc<dyn Notifier>,
    transport: Transport,
    variables: OwnVariables,
    /// Storage, credentials and settings; without it they are
    /// `unavailable`.
    access: Option<DataAccess>,
    opener: Opener,
    /// URLs this invocation opened (an executor serves one invocation).
    opened: std::sync::atomic::AtomicU32,
    /// What started the invocation's run (G8); unknown until the caller
    /// says, which counts as not started by the user.
    run: RunContext,
}

impl std::fmt::Debug for Executor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Executor")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Executor {
    pub fn new(
        broker: Broker,
        name: String,
        events: broadcast::Sender<AgentEvent>,
        transport: Transport,
        variables: OwnVariables,
        opener: Opener,
    ) -> Self {
        Self {
            broker,
            name,
            notifier: Arc::new(StatusBar(events.clone())),
            events,
            transport,
            variables,
            access: None,
            opener,
            opened: std::sync::atomic::AtomicU32::new(0),
            run: RunContext::unknown(),
        }
    }

    /// Plugin notifications go to `notifier` (the agent's: the status bar
    /// of open app windows, and the desktop).
    pub fn with_notifier(mut self, notifier: Arc<dyn Notifier>) -> Self {
        self.notifier = notifier;
        self
    }

    /// The executor of an invocation whose run `run` describes.
    pub fn with_run(mut self, run: RunContext) -> Self {
        self.run = run.bounded();
        self
    }

    /// Opens a URL the broker allowed, in the user's browser.
    fn open_url(
        &self,
        plugin: &PluginId,
        url: &url::Url,
        origin: &deckpilot_plugin_broker::Origin,
    ) -> CallResult {
        if !openable(url, origin) {
            warn!(plugin = %plugin, origin = %origin, "a plugin URL was refused before opening");
            return CallResult::error(CallErrorCode::NotPermitted, "the URL cannot be opened");
        }
        // The broker allows it only in runs the user started; checked
        // again here, so nothing but such a run reaches the launcher.
        if !self.run.is_user_initiated() {
            warn!(plugin = %plugin, origin = %origin, trigger = ?self.run.trigger,
                cause = ?self.run.cause, "a plugin page was refused: the user did not start the run");
            return CallResult::error(
                CallErrorCode::NotPermitted,
                deckpilot_plugin_broker::DenyReason::NotUserInitiated.to_string(),
            );
        }
        let opened = self
            .opened
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if opened >= MAX_OPENS_PER_INVOCATION {
            return CallResult::error(
                CallErrorCode::Throttled,
                format!("at most {MAX_OPENS_PER_INVOCATION} URLs are opened in one action"),
            );
        }
        match self.opener.open(url) {
            Ok(()) => {
                info!(plugin = %plugin, origin = %origin, "a plugin opened a page in the browser");
                CallResult::Ok(Value::Null)
            }
            Err(problem) => {
                warn!(plugin = %plugin, origin = %origin, %problem, "a plugin page could not be opened");
                CallResult::error(
                    CallErrorCode::Failed,
                    format!("the browser could not be started: {problem}"),
                )
            }
        }
    }

    /// Shows an allowed notification, titled with the plugin's name, where
    /// the user's routing for the plugin says (G9); without a routing in
    /// the manifest, in the status bar only, as for every plugin before
    /// it. The answer says where it was shown (the host passes it only to
    /// guests that read it); a notification shown nowhere is still `ok`.
    fn notify(
        &self,
        plugin: &PluginId,
        title: &str,
        body: &str,
        severity: NotifySeverity,
    ) -> CallResult {
        let severity = notification_severity(severity);
        let mut notification =
            Notification::new(&format!("{}: {title}", self.name), body, severity);
        let route = match self.route(plugin) {
            Ok(route) => route,
            Err(refused) => return refused,
        };
        let delivery = if rank(severity) < rank(route.minimum) {
            NotifyDelivery::not_shown("less important than the plugin's setting")
        } else if !route.desktop && !route.status_bar {
            NotifyDelivery::not_shown(&format!(
                "notifications are turned off in Settings › Plugins › {}",
                self.name
            ))
        } else {
            notification.desktop = route.desktop;
            notification.status_bar = route.status_bar;
            match self.notifier.notify(&notification) {
                Ok(shown) => NotifyDelivery::shown(shown.desktop, shown.status_bar),
                Err(reason) => NotifyDelivery::not_shown(&reason),
            }
        };
        debug!(plugin = %plugin, shown = delivery.shown, desktop = delivery.desktop,
            status_bar = delivery.status_bar, "a plugin notification");
        serde_json::to_value(&delivery).map_or_else(
            |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
            CallResult::Ok,
        )
    }

    /// Where the plugin's notifications go: the settings its manifest's
    /// routing names, as the user saved them (defaults filled in), read
    /// here and never by the plugin. Without a routing: the status bar
    /// only, every severity.
    fn route(&self, plugin: &PluginId) -> Result<Route, CallResult> {
        let without = Route {
            desktop: false,
            status_bar: true,
            minimum: Severity::Info,
        };
        let Some(routing) = self
            .access
            .as_ref()
            .and_then(|access| access.manifest.notification_routing().cloned())
        else {
            return Ok(without);
        };
        self.with_store(plugin, |data, id, manifest| {
            let saved = data
                .settings(id)
                .map_err(|e| CallResult::error(CallErrorCode::Failed, e.to_string()))?;
            let (effective, _) = super::data::effective_settings(manifest, &saved);
            let flag = |key: Option<&String>, absent: bool| {
                key.map_or(absent, |key| {
                    effective.get(key).and_then(Value::as_bool).unwrap_or(false)
                })
            };
            Ok(Route {
                desktop: flag(routing.desktop.as_ref(), false),
                status_bar: flag(routing.status_bar.as_ref(), true),
                minimum: routing
                    .minimum_severity
                    .as_ref()
                    .and_then(|key| effective.get(key))
                    .and_then(Value::as_str)
                    .map_or(Severity::Info, Severity::parse),
            })
        })
        .and_then(|route| route)
    }

    /// Gives the executor the plugin's data (storage, credentials,
    /// settings).
    pub fn with_data(mut self, access: DataAccess) -> Self {
        self.access = Some(access);
        self
    }

    /// Runs `f` on the data connection, scoped to the plugin the session
    /// belongs to (which must be the manifest's).
    fn with_store<T>(
        &self,
        plugin: &PluginId,
        f: impl FnOnce(&mut PluginData, &deckpilot_plugin_manifest::PluginId, &Manifest) -> T,
    ) -> Result<T, CallResult> {
        let unavailable = || {
            CallResult::error(
                CallErrorCode::Unavailable,
                "plugin data is not available right now (the configuration is locked)",
            )
        };
        let access = self.access.as_ref().ok_or_else(unavailable)?;
        if access.manifest.id.as_str() != plugin.as_str() {
            return Err(CallResult::error(
                CallErrorCode::Failed,
                "the executor belongs to another plugin",
            ));
        }
        let mut guard = access.data.lock().map_err(|_| unavailable())?;
        let data = guard.as_mut().ok_or_else(unavailable)?;
        Ok(f(data, &access.manifest.id, &access.manifest))
    }

    /// The secret to inject for an allowed `auth`, as the HTTP client's
    /// [`Auth`] (header or query name from the slot's declaration).
    fn credential(
        &self,
        plugin: &PluginId,
        auth: &deckpilot_plugin_abi::capability::HttpAuth,
    ) -> Result<Auth, CallResult> {
        let (secret, slot) = self.with_store(plugin, |data, id, manifest| {
            let slot = manifest
                .contributes
                .credentials()
                .iter()
                .find(|c| c.key == auth.credential)
                .cloned();
            (data.credential_secret(id, &auth.credential), slot)
        })?;
        let missing = || {
            CallResult::error(
                CallErrorCode::Unavailable,
                format!(
                    "no secret is stored for the credential `{}`; the user sets it in the \
                     plugin's settings",
                    auth.credential
                ),
            )
        };
        let secret: Secret = secret
            .map_err(|e| CallResult::error(CallErrorCode::Failed, e.to_string()))?
            .ok_or_else(missing)?;
        let slot = slot.ok_or_else(|| {
            CallResult::error(
                CallErrorCode::NotPermitted,
                "the credential slot is not declared",
            )
        })?;
        let value = deckpilot_core::Secret::new(secret.expose());
        let named = |name: Option<String>, what: &str| {
            name.filter(|n| !n.is_empty()).ok_or_else(|| {
                CallResult::error(
                    CallErrorCode::Failed,
                    format!("the credential slot declares no {what} name"),
                )
            })
        };
        Ok(match auth.scheme {
            AuthScheme::Bearer => Auth::Bearer(value),
            AuthScheme::Basic => Auth::Basic(value),
            AuthScheme::Header => Auth::Header(named(slot.header_name, "header")?, value),
            AuthScheme::Query => Auth::Query(named(slot.query_name, "query parameter")?, value),
        })
    }

    /// Publishes (`Some`) or clears one of the plugin's declared variables.
    fn publish(
        &self,
        plugin: &PluginId,
        id: &str,
        value: Option<&serde_json::Value>,
    ) -> CallResult {
        let own = &self.variables;
        let Some(variable) = own.declared.iter().find(|v| v.id == id) else {
            return CallResult::error(
                CallErrorCode::NotPermitted,
                format!(
                    "`{id}` is not one of the plugin's variables: declare it in \
                     contributes.variables of manifest.json"
                ),
            );
        };
        if variable.scope != VariableScope::Machine {
            return CallResult::error(
                CallErrorCode::Unavailable,
                format!(
                    "`{id}` is a {} variable; this version of DeckPilot keeps only values \
                     that are the same everywhere (scope machine)",
                    if variable.scope == VariableScope::Profile {
                        "profile"
                    } else {
                        "project"
                    }
                ),
            );
        }
        let value = match value.map(|value| typed(variable, value)) {
            Some(Ok(value)) => value,
            Some(Err(problem)) => return CallResult::error(CallErrorCode::BadRequest, problem),
            None => None,
        };
        if !own.store.set(plugin.as_str(), own.generation, id, value) {
            return CallResult::error(
                CallErrorCode::Unavailable,
                "the plugin was stopped since this action started; its values were dropped",
            );
        }
        self.announce(vec![format!("{}.{id}", own.namespace)]);
        CallResult::Ok(serde_json::Value::Null)
    }

    /// Clears every variable the plugin published.
    fn clear(&self, plugin: &PluginId) -> CallResult {
        let own = &self.variables;
        match own.store.clear(plugin.as_str(), own.generation) {
            Some(ids) => {
                self.announce(
                    ids.iter()
                        .map(|id| format!("{}.{id}", own.namespace))
                        .collect(),
                );
                CallResult::Ok(serde_json::Value::Null)
            }
            None => CallResult::error(
                CallErrorCode::Unavailable,
                "the plugin was stopped since this action started; its values were dropped",
            ),
        }
    }

    /// Tells pickers and previews that these variables changed (ids only).
    fn announce(&self, keys: Vec<String>) {
        if !keys.is_empty() {
            let _ = self
                .events
                .send(AgentEvent::SharedVariablesChanged { keys });
        }
    }

    fn fetch(
        &self,
        plugin: &PluginId,
        allowed: &AllowedFetch,
        interrupt: Option<&CallInterrupt>,
    ) -> CallResult {
        let session = self.broker.session(plugin.clone());
        let mut current = allowed.clone();
        let mut hop = 0;
        loop {
            debug!(plugin = %plugin, origin = %current.origin, "plugin fetch");
            // Read for every hop: the broker re-allowed this hop's
            // credential for its origin (a redirect off the origin never
            // carries one).
            let auth = match current.auth.as_ref().map(|a| self.credential(plugin, a)) {
                Some(Ok(auth)) => Some(auth),
                Some(Err(refused)) => return refused,
                None => None,
            };
            let forms = auth.as_ref().map_or_else(Vec::new, |a| {
                let secret = match a {
                    Auth::Bearer(s) | Auth::Basic(s) | Auth::Header(_, s) | Auth::Query(_, s) => s,
                };
                secret_forms(secret.expose())
            });
            let exchange = match self.transport.send(&current, auth.as_ref(), interrupt) {
                Ok(exchange) => redacted(exchange, &forms),
                Err(failure) => {
                    return CallResult::error_with_kind(
                        CallErrorCode::Failed,
                        redact(&failure.message, &forms),
                        failure.kind,
                    );
                }
            };
            let location = (300..400)
                .contains(&exchange.status)
                .then(|| {
                    exchange
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                        .map(|(_, value)| value.clone())
                })
                .flatten();
            let Some(location) = location.filter(|_| exchange.status != 304) else {
                return reply(exchange);
            };
            hop += 1;
            match session.authorize_redirect(&current, &location, hop) {
                Decision::Allow(Allowed::Fetch(next)) => current = next,
                Decision::Deny(reason) => {
                    return CallResult::error_with_kind(
                        reason.call_error_code(),
                        format!("a redirect was refused by the permission broker ({reason})"),
                        reason.network_kind(),
                    );
                }
                Decision::Allow(_) => {
                    return CallResult::error(
                        CallErrorCode::Failed,
                        "unexpected redirect decision",
                    );
                }
            }
        }
    }
}

/// The exchange with every form of the injected secret replaced (a server
/// that echoes the request must not hand the secret to the plugin).
fn redacted(exchange: Exchange, forms: &[String]) -> Exchange {
    if forms.is_empty() {
        return exchange;
    }
    Exchange {
        status: exchange.status,
        headers: exchange
            .headers
            .into_iter()
            .map(|(name, value)| (name, redact(&value, forms)))
            .collect(),
        body: match String::from_utf8(exchange.body) {
            Ok(text) => redact(&text, forms).into_bytes(),
            // Not UTF-8: never delivered in ABI 1 (`reply`).
            Err(_) => Vec::new(),
        },
    }
}

/// A plugin's severity as the core's: how the notification looks, and
/// (with a routing) whether it is less severe than the user shows.
fn notification_severity(severity: NotifySeverity) -> Severity {
    match severity {
        NotifySeverity::Info => Severity::Info,
        NotifySeverity::Success => Severity::Success,
        NotifySeverity::Warning => Severity::Warning,
        NotifySeverity::Error => Severity::Error,
    }
}

/// The status bar only, through the agent's events (an executor without
/// the agent's notifier).
#[derive(Debug)]
struct StatusBar(broadcast::Sender<AgentEvent>);

impl Notifier for StatusBar {
    fn notify(&self, notification: &Notification) -> Result<Delivery, String> {
        if !notification.status_bar {
            return Err("desktop notifications are not available".into());
        }
        self.0
            .send(AgentEvent::Notification {
                notification: notification.clone(),
            })
            .map(|_| Delivery {
                desktop: false,
                status_bar: true,
            })
            .map_err(|_| "the app is not open".to_owned())
    }
}

/// Where one plugin notification goes.
#[derive(Debug, Clone, Copy)]
struct Route {
    desktop: bool,
    status_bar: bool,
    /// Less severe notifications are not shown.
    minimum: Severity,
}

fn rank(severity: Severity) -> u8 {
    match severity {
        Severity::Info => 0,
        Severity::Success => 1,
        Severity::Warning => 2,
        Severity::Error => 3,
    }
}

fn storage_error(error: &StorageError) -> CallResult {
    let code = match error {
        StorageError::InvalidKey | StorageError::ValueTooLarge => CallErrorCode::BadRequest,
        StorageError::QuotaExceeded { .. } | StorageError::TooManyKeys { .. } => {
            CallErrorCode::Failed
        }
        StorageError::NotInstalled => CallErrorCode::Unavailable,
        StorageError::Store(_) => CallErrorCode::Failed,
    };
    CallResult::error(code, error.to_string())
}

fn quota(granted: GrantedQuota) -> StorageQuota {
    StorageQuota {
        bytes: granted.bytes,
        keys: granted.keys,
    }
}

/// The headers a plugin receives: the ABI's allowlist only, by lowercase
/// name, values of a sane size without control characters, at most
/// [`MAX_HEADERS`]. `set-cookie`, authentication challenges and every
/// header not listed are dropped. Called after [`redacted`], so a kept
/// value never carries an injected secret.
fn response_headers(headers: Vec<(String, String)>) -> Vec<Header> {
    headers
        .into_iter()
        .filter(|(name, value)| {
            is_response_header(name)
                && value.len() <= MAX_HEADER_LEN
                && !value.chars().any(char::is_control)
        })
        .take(MAX_HEADERS)
        .map(|(name, value)| Header {
            name: name.to_ascii_lowercase(),
            value,
        })
        .collect()
}

fn reply(exchange: Exchange) -> CallResult {
    let headers = response_headers(exchange.headers);
    let response = HttpResponse {
        status: exchange.status,
        headers,
        body: String::from_utf8(exchange.body)
            .ok()
            .filter(|b| !b.is_empty()),
    };
    match serde_json::to_value(&response) {
        Ok(value) => CallResult::Ok(value),
        Err(e) => CallResult::error(CallErrorCode::Failed, e.to_string()),
    }
}

impl CapabilityExecutor for Executor {
    fn execute(&mut self, plugin: &PluginId, allowed: &Allowed) -> CallResult {
        self.run(plugin, allowed, None)
    }

    /// A fetch stops at once when the invocation is aborted or past its
    /// deadline; what the server would still have answered is dropped.
    fn execute_interruptible(
        &mut self,
        plugin: &PluginId,
        allowed: &Allowed,
        interrupt: &CallInterrupt,
    ) -> CallResult {
        self.run(plugin, allowed, Some(interrupt))
    }

    fn run_context(&self) -> Option<&RunContext> {
        Some(&self.run)
    }
}

impl Executor {
    fn run(
        &self,
        plugin: &PluginId,
        allowed: &Allowed,
        interrupt: Option<&CallInterrupt>,
    ) -> CallResult {
        match allowed {
            Allowed::Fetch(fetch) => self.fetch(plugin, fetch, interrupt),
            Allowed::VariablePublish { id, value } => self.publish(plugin, id, Some(value)),
            Allowed::VariableClear { id: Some(id) } => self.publish(plugin, id, None),
            Allowed::VariableClear { id: None } => self.clear(plugin),
            Allowed::OpenUrl { url, origin } => self.open_url(plugin, url, origin),
            Allowed::RunContext(context) => serde_json::to_value(context).map_or_else(
                |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
                CallResult::Ok,
            ),
            Allowed::Notify {
                title,
                body,
                severity,
            } => self.notify(plugin, title, body, *severity),
            Allowed::Log { level, message } => {
                let plugin = plugin.as_str();
                match level {
                    LogLevel::Error | LogLevel::Warn => warn!(plugin, "{message}"),
                    LogLevel::Info => info!(plugin, "{message}"),
                    LogLevel::Debug => debug!(plugin, "{message}"),
                }
                CallResult::Ok(serde_json::Value::Null)
            }
            Allowed::StorageGet { key } => self
                .with_store(plugin, |data, id, _| match data.storage_get(id, key) {
                    Ok(value) => CallResult::Ok(value.unwrap_or(Value::Null)),
                    Err(e) => storage_error(&e),
                })
                .unwrap_or_else(|refused| refused),
            Allowed::StorageSet {
                key,
                value,
                quota: granted,
            } => self
                .with_store(plugin, |data, id, _| {
                    match data.storage_set(id, key, value, quota(*granted)) {
                        Ok(_) => CallResult::Ok(Value::Null),
                        Err(e) => storage_error(&e),
                    }
                })
                .unwrap_or_else(|refused| refused),
            Allowed::StorageDelete { key } => self
                .with_store(plugin, |data, id, _| match data.storage_delete(id, key) {
                    Ok(_) => CallResult::Ok(Value::Null),
                    Err(e) => storage_error(&e),
                })
                .unwrap_or_else(|refused| refused),
            Allowed::StorageKeys { prefix } => self
                .with_store(plugin, |data, id, _| {
                    match data.storage_keys(id, prefix.as_deref()) {
                        Ok(keys) => CallResult::Ok(Value::from(keys)),
                        Err(e) => CallResult::error(CallErrorCode::Failed, e.to_string()),
                    }
                })
                .unwrap_or_else(|refused| refused),
            Allowed::StorageUsage { quota } => self
                .with_store(plugin, |data, id, _| match data.storage_usage(id) {
                    Ok(usage) => serde_json::to_value(StorageUsage {
                        used_bytes: usage.bytes,
                        keys: usage.keys,
                        quota_bytes: quota.bytes,
                        max_keys: quota.keys,
                    })
                    .map_or_else(
                        |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
                        CallResult::Ok,
                    ),
                    Err(e) => CallResult::error(CallErrorCode::Failed, e.to_string()),
                })
                .unwrap_or_else(|refused| refused),
            // Only whether a secret is stored, never the secret.
            Allowed::CredentialHas { key } => self
                .with_store(plugin, |data, id, _| {
                    match data.credential_secret(id, key) {
                        Ok(secret) => CallResult::Ok(Value::Bool(secret.is_some())),
                        Err(e) => CallResult::error(CallErrorCode::Failed, e.to_string()),
                    }
                })
                .unwrap_or_else(|refused| refused),
            Allowed::SettingsGet => self
                .with_store(plugin, |data, id, manifest| match data.settings(id) {
                    Ok(saved) => {
                        let (effective, _) = super::data::effective_settings(manifest, &saved);
                        CallResult::Ok(Value::Object(effective.into_iter().collect()))
                    }
                    Err(e) => CallResult::error(CallErrorCode::Failed, e.to_string()),
                })
                .unwrap_or_else(|refused| refused),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_private_network_user_origin_reaches_private_addresses() {
        let fetch = |private_network: bool| AllowedFetch {
            url: url::Url::parse("https://ghe.example.com/api/v3").unwrap(),
            origin: deckpilot_plugin_broker::Origin::parse_grant("https://ghe.example.com")
                .unwrap(),
            method: HttpMethod::Get,
            headers: Vec::new(),
            body: None,
            auth: None,
            private_network,
        };
        let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        let check = |private: bool, address: &str| {
            address_policy(&fetch(private)).check("ghe.example.com", &[ip(address)])
        };
        for address in ["10.0.0.5", "192.168.1.2", "100.64.1.1", "fd00::5"] {
            assert!(check(false, address).is_err(), "{address}");
            check(true, address).unwrap();
        }
        check(false, "140.82.112.5").unwrap();
        check(true, "140.82.112.5").unwrap();
        for address in ["127.0.0.1", "::1", "169.254.169.254", "fe80::1", "0.0.0.0"] {
            for private in [false, true] {
                let (code, _) = check(private, address).unwrap_err();
                assert_eq!(code, deckpilot_net::codes::BLOCKED_ADDRESS, "{address}");
            }
        }
        // Every address must pass: one private address among public ones
        // is refused without the consent.
        assert!(
            address_policy(&fetch(false))
                .check("ghe.example.com", &[ip("140.82.112.5"), ip("10.0.0.5")])
                .is_err()
        );
        // The message names the origin, never the address.
        let failure =
            SendFailure::from_transport(check(false, "10.0.0.5").unwrap_err(), &fetch(false));
        assert!(!failure.message.contains("10.0.0.5"), "{}", failure.message);
        assert!(failure.message.contains("allowed on a private network"));
    }

    #[test]
    fn network_failures_keep_the_transport_code_as_their_kind() {
        use deckpilot_net::codes;
        let fetch = AllowedFetch {
            url: url::Url::parse("https://api.example.com/x?token=s3cret").unwrap(),
            origin: deckpilot_plugin_broker::Origin::parse_grant("https://api.example.com")
                .unwrap(),
            method: HttpMethod::Get,
            headers: Vec::new(),
            body: None,
            auth: None,
            private_network: false,
        };
        let failure = |code: &'static str, message: &str| {
            SendFailure::from_transport((code, message.to_owned()), &fetch)
        };
        for (code, kind) in [
            (codes::DNS_FAILED, NetworkErrorKind::DnsFailed),
            (codes::CONNECTION_FAILED, NetworkErrorKind::ConnectionFailed),
            (codes::TLS_FAILED, NetworkErrorKind::TlsFailed),
            ("timed-out", NetworkErrorKind::TimedOut),
            (
                codes::TOO_MANY_REDIRECTS,
                NetworkErrorKind::TooManyRedirects,
            ),
            (codes::INSECURE_REDIRECT, NetworkErrorKind::InsecureRedirect),
            (codes::BLOCKED_ADDRESS, NetworkErrorKind::BlockedAddress),
        ] {
            assert_eq!(failure(code, "m").kind, Some(kind), "{code}");
        }
        for code in [codes::REQUEST_FAILED, codes::INVALID_URL, "cancelled"] {
            let f = failure(code, "the message");
            assert_eq!(f.kind, None, "{code}");
            assert_eq!(f.message, "the message");
        }
        // A blocked address is named by the granted origin, never by the
        // address it resolved to, nor by the app's HTTP settings.
        let blocked = failure(
            codes::BLOCKED_ADDRESS,
            "api.example.com is on this computer or a private network (10.0.0.7); allow it in \
             Settings › HTTP",
        );
        assert!(!blocked.message.contains("10.0.0.7"), "{blocked:?}");
        assert!(!blocked.message.contains("Settings"), "{blocked:?}");
        assert!(blocked.message.starts_with("https://api.example.com "));
    }

    #[test]
    fn every_form_of_a_secret_is_redacted() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(form_encoded("a b&c/é"), "a+b%26c%2F%C3%A9");
        let forms = secret_forms("user:p@ss word");
        let text = format!(
            "raw user:p@ss word, query {}, basic {}",
            form_encoded("user:p@ss word"),
            base64(b"user:p@ss word")
        );
        let clean = redact(&text, &forms);
        assert!(!clean.contains("p@ss"), "{clean}");
        assert!(!clean.contains(&base64(b"user:p@ss word")), "{clean}");
        assert_eq!(clean.matches(REDACTED).count(), 3, "{clean}");
        let exchange = redacted(
            Exchange {
                status: 200,
                headers: vec![("x-echo".into(), "Bearer user:p@ss word".into())],
                body: b"{\"token\":\"user:p@ss word\"}".to_vec(),
            },
            &forms,
        );
        assert_eq!(exchange.headers[0].1, "Bearer [redacted]");
        assert_eq!(exchange.body, b"{\"token\":\"[redacted]\"}");
    }

    #[test]
    fn only_allowlisted_headers_reach_the_plugin_redacted() {
        let secret = "tok-9d2e-header-secret";
        let forms = secret_forms(secret);
        let exchange = redacted(
            Exchange {
                status: 200,
                headers: [
                    (
                        "Link",
                        format!(
                            "<https://api.example.com/x?page=2&api_key={secret}>; rel=\"next\""
                        ),
                    ),
                    ("X-RateLimit-Remaining", "4999".to_owned()),
                    ("ratelimit-reset", "60".to_owned()),
                    ("x-total", "120".to_owned()),
                    ("etag", format!("W/\"{}\"", base64(secret.as_bytes()))),
                    ("set-cookie", format!("session={secret}")),
                    ("Set-Cookie", "plain=1".to_owned()),
                    ("www-authenticate", "Bearer realm=x".to_owned()),
                    ("authorization", format!("Bearer {secret}")),
                    ("x-api-key", secret.to_owned()),
                    ("x-oauth-scopes", "repo".to_owned()),
                    ("location", "https://api.example.com/".to_owned()),
                    ("x-ratelimit-token", "t".to_owned()),
                    ("retry-after", "a\u{7f}b".to_owned()),
                    ("x-total-pages", "p".repeat(MAX_HEADER_LEN + 1)),
                ]
                .into_iter()
                .map(|(n, v)| (n.to_owned(), v))
                .collect(),
                body: Vec::new(),
            },
            &forms,
        );
        let headers = response_headers(exchange.headers);
        let names: Vec<&str> = headers.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "link",
                "x-ratelimit-remaining",
                "ratelimit-reset",
                "x-total",
                "etag"
            ]
        );
        let all = format!("{headers:?}");
        assert!(!all.contains(secret), "{all}");
        assert!(!all.contains(&base64(secret.as_bytes())), "{all}");
        assert_eq!(
            headers[0].value,
            "<https://api.example.com/x?page=2&api_key=[redacted]>; rel=\"next\""
        );
        // Never more than the ABI allows.
        let many = (0..100).map(|i| ("link".to_owned(), format!("<https://a.example.com/{i}>")));
        assert_eq!(response_headers(many.collect()).len(), MAX_HEADERS);
    }

    #[test]
    fn only_the_allowed_origin_is_opened() {
        let origin = deckpilot_plugin_broker::Origin::parse_grant("https://github.com").unwrap();
        let ok = |s: &str| openable(&url::Url::parse(s).unwrap(), &origin);
        assert!(ok("https://github.com/acme/app/pull/7?tab=files#diff"));
        for bad in [
            "http://github.com/",
            "file:///etc/passwd",
            "https://u:p@github.com/",
            "https://github.com:8443/",
            "https://gist.github.com/",
            "https://140.82.112.3/",
            "x-github-client://github.com/",
        ] {
            assert!(!ok(bad), "{bad}");
        }
        let long = format!("https://github.com/{}", "a".repeat(4096));
        assert!(!ok(&long));
    }

    #[cfg(feature = "test-hooks")]
    #[test]
    fn an_invocation_opens_at_most_a_few_urls() {
        let dir = std::env::temp_dir().join(format!("dp-opener-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let record = dir.join("opened.txt");
        let _ = std::fs::remove_file(&record);
        let broker = Broker::new(
            Arc::new(deckpilot_plugin_broker::MemoryAudit::new(10)),
            Arc::new(deckpilot_plugin_broker::SystemClock::default()),
            deckpilot_plugin_broker::RateLimits::default(),
        );
        let (events, _) = broadcast::channel(4);
        let executor = Executor::new(
            broker,
            "Test".into(),
            events,
            Transport::Network,
            OwnVariables {
                store: PublishedVariables::default(),
                namespace: "plugins.acme.test".into(),
                generation: 0,
                declared: Vec::new(),
            },
            Opener::Recorded(Arc::new(record.clone())),
        )
        .with_run(RunContext::direct(
            deckpilot_plugin_abi::capability::RunTrigger::Key,
        ));
        let plugin = PluginId::new("acme.test").unwrap();
        let origin = deckpilot_plugin_broker::Origin::parse_grant("https://github.com").unwrap();
        let url = url::Url::parse("https://github.com/acme/app?token=x").unwrap();
        for _ in 0..MAX_OPENS_PER_INVOCATION {
            assert_eq!(
                executor.open_url(&plugin, &url, &origin),
                CallResult::Ok(Value::Null)
            );
        }
        let CallResult::Error(e) = executor.open_url(&plugin, &url, &origin) else {
            panic!()
        };
        assert_eq!(e.code, CallErrorCode::Throttled);
        let other = url::Url::parse("https://evil.example.net/").unwrap();
        let CallResult::Error(e) = executor.open_url(&plugin, &other, &origin) else {
            panic!()
        };
        assert_eq!(e.code, CallErrorCode::NotPermitted);
        let lines = std::fs::read_to_string(&record).unwrap();
        assert_eq!(lines.lines().count(), MAX_OPENS_PER_INVOCATION as usize);
        assert!(lines.lines().all(|l| l == url.as_str()), "{lines}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// G8: pages open from key presses and the app; scheduled, event,
    /// startup and unknown runs are refused by the broker (with the
    /// executor's context) and again by the executor itself, and nothing
    /// reaches the opener. `run.context` answers the executor's context.
    #[cfg(feature = "test-hooks")]
    #[test]
    #[allow(clippy::too_many_lines)] // one scenario, step by step
    fn pages_open_only_in_runs_the_user_started() {
        use deckpilot_plugin_abi::CapabilityRequest;
        use deckpilot_plugin_abi::capability::{NoArgs, OpenUrl, RunCause, RunTrigger};
        use deckpilot_plugin_supervisor::CapabilityExecutor as _;
        let dir = std::env::temp_dir().join(format!("dp-run-context-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let record = dir.join("opened.txt");
        let _ = std::fs::remove_file(&record);
        let clock = deckpilot_plugin_broker::ManualClock::default();
        let broker = Broker::new(
            Arc::new(deckpilot_plugin_broker::MemoryAudit::new(100)),
            Arc::new(clock.clone()),
            deckpilot_plugin_broker::RateLimits::default(),
        );
        let plugin = PluginId::new("acme.test").unwrap();
        broker
            .set_grants(
                &plugin,
                &deckpilot_plugin_broker::ApprovedPermissions::from_json(
                    r#"[{"kind":"openUrl","origin":"https://github.com"}]"#,
                )
                .unwrap(),
            )
            .unwrap();
        let executor = |run: RunContext| {
            let (events, _) = broadcast::channel(4);
            Executor::new(
                broker.clone(),
                "Test".into(),
                events,
                Transport::Network,
                OwnVariables {
                    store: PublishedVariables::default(),
                    namespace: "plugins.acme.test".into(),
                    generation: 0,
                    declared: Vec::new(),
                },
                Opener::Recorded(Arc::new(record.clone())),
            )
            .with_run(run)
        };
        let session = broker.session(plugin.clone());
        let page = "https://github.com/acme/app/pull/7";
        let open = CapabilityRequest::OpenUrl(OpenUrl { url: page.into() });
        // What the supervisor does with each call: the broker decides with
        // the executor's context, the executor carries it out.
        let call = |executor: &mut Executor, request: &CapabilityRequest| {
            clock.advance(std::time::Duration::from_secs(3));
            match session.authorize_in(request, executor.run_context()) {
                deckpilot_plugin_broker::Decision::Allow(allowed) => {
                    executor.execute(&plugin, &allowed)
                }
                deckpilot_plugin_broker::Decision::Deny(reason) => {
                    CallResult::error(reason.call_error_code(), reason.to_string())
                }
            }
        };
        let opened = || {
            std::fs::read_to_string(&record)
                .unwrap_or_default()
                .lines()
                .count()
        };
        for allowed in [
            RunContext::direct(RunTrigger::Key),
            RunContext::direct(RunTrigger::App),
            RunContext::direct(RunTrigger::Gesture),
            RunContext::workflow(RunCause::User, None),
        ] {
            let before = opened();
            let result = call(&mut executor(allowed.clone()), &open);
            assert_eq!(result, CallResult::Ok(Value::Null), "{allowed:?}");
            assert_eq!(opened(), before + 1, "{allowed:?}");
        }
        let before = opened();
        for denied in [
            RunContext::workflow(RunCause::Schedule, None),
            RunContext::workflow(RunCause::Event, None),
            RunContext::workflow(RunCause::Startup, None),
            RunContext::workflow(RunCause::WorkSession, None),
            RunContext::unknown(),
        ] {
            let mut executor = executor(denied.clone());
            let CallResult::Error(e) = call(&mut executor, &open) else {
                panic!("{denied:?}")
            };
            assert_eq!(e.code, CallErrorCode::NotPermitted, "{denied:?}");
            assert!(e.message.contains("the user started"), "{}", e.message);
            // Even with an `Allowed` in hand, the executor refuses.
            let url = url::Url::parse(page).unwrap();
            let origin =
                deckpilot_plugin_broker::Origin::parse_grant("https://github.com").unwrap();
            let CallResult::Error(e) = executor.open_url(&plugin, &url, &origin) else {
                panic!("{denied:?}")
            };
            assert_eq!(e.code, CallErrorCode::NotPermitted);
        }
        assert_eq!(opened(), before, "a refused page reached the opener");
        // An executor without a context is not the user's either.
        let (events, _) = broadcast::channel(4);
        let bare = Executor::new(
            broker.clone(),
            "Test".into(),
            events,
            Transport::Network,
            OwnVariables {
                store: PublishedVariables::default(),
                namespace: "plugins.acme.test".into(),
                generation: 0,
                declared: Vec::new(),
            },
            Opener::Recorded(Arc::new(record.clone())),
        );
        assert!(!bare.run_context().unwrap().is_user_initiated());

        // `run.context`: exactly the executor's context.
        let run = deckpilot_plugin_abi::capability::RunSummary {
            workflow: Some("Nightly".into()),
            succeeded: 2,
            failed: 1,
            failed_nodes: vec!["deploy".into()],
            ..Default::default()
        };
        let context = RunContext::workflow(RunCause::Schedule, Some(run));
        let answer = call(
            &mut executor(context.clone()),
            &CapabilityRequest::RunContext(NoArgs {}),
        );
        assert_eq!(
            answer,
            CallResult::Ok(serde_json::to_value(&context).unwrap())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Records opened URLs for agent tests (feature `test-hooks`, never in a
/// release build): with `DECKPILOT_TEST_PLUGIN_OPEN_URL` set, the agent
/// appends every URL it would open to that file and starts no browser.
#[cfg(feature = "test-hooks")]
pub(super) mod test_opener {
    use std::io::Write as _;
    use std::path::Path;

    pub const ENV: &str = "DECKPILOT_TEST_PLUGIN_OPEN_URL";

    pub fn record(path: &Path, url: &url::Url) -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        writeln!(file, "{url}").map_err(|e| e.to_string())
    }
}

/// Canned HTTP responses for agent tests (feature `test-hooks`, never in
/// a release build): the agent reads them from the JSON file named by
/// `DECKPILOT_TEST_PLUGIN_HTTP` instead of using the network, and appends
/// every URL it fetched to `<file>.log`. Only requests the broker allowed
/// get here, so tests observe exactly what reached the network layer.
#[cfg(feature = "test-hooks")]
pub(super) mod test_transport {
    use std::io::Write as _;
    use std::path::PathBuf;

    use deckpilot_plugin_broker::AllowedFetch;
    use deckpilot_plugin_supervisor::CallInterrupt;
    use serde::Deserialize;

    use super::Exchange;

    pub const ENV: &str = "DECKPILOT_TEST_PLUGIN_HTTP";

    #[derive(Debug, Deserialize)]
    struct Route {
        /// Matches every URL that starts with it.
        url: String,
        status: u16,
        #[serde(default)]
        headers: Vec<(String, String)>,
        #[serde(default)]
        body: String,
        /// Answers only after this long (a slow server).
        #[serde(default, rename = "delayMs")]
        delay_ms: u64,
        /// Fails instead of answering, with this transport code
        /// (`dns-failed`, `connection-failed`, `tls-failed`, ...), as the
        /// network client would.
        #[serde(default)]
        fail: Option<String>,
        /// The address the host "resolves" to: checked with the address
        /// policy the real client would use for this hop
        /// (`address_policy`), so a test sees a private address refused
        /// or allowed exactly as on the network.
        #[serde(default, rename = "resolvesTo")]
        resolves_to: Option<std::net::IpAddr>,
    }

    #[derive(Debug)]
    pub struct Canned {
        routes: Vec<Route>,
        log: PathBuf,
    }

    impl Canned {
        pub fn from_env() -> Option<Self> {
            let path = PathBuf::from(std::env::var_os(ENV)?);
            let routes = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            let mut log = path.into_os_string();
            log.push(".log");
            Some(Self {
                routes,
                log: log.into(),
            })
        }

        /// Logs what would go on the wire: the URL (with an injected query
        /// parameter) and, after a tab, an injected credential header.
        pub fn send(
            &self,
            fetch: &AllowedFetch,
            auth: Option<&super::Auth>,
            interrupt: Option<&CallInterrupt>,
        ) -> Result<Exchange, super::SendFailure> {
            use super::{Auth, SendFailure};
            let mut target = fetch.url.clone();
            if let Some(Auth::Query(name, secret)) = auth {
                target.query_pairs_mut().append_pair(name, secret.expose());
            }
            let header = match auth {
                Some(Auth::Bearer(s)) => format!("\tauthorization: Bearer {}", s.expose()),
                Some(Auth::Basic(s)) => format!("\tauthorization: Basic {}", s.expose()),
                Some(Auth::Header(name, s)) => format!("\t{name}: {}", s.expose()),
                Some(Auth::Query(..)) | None => String::new(),
            };
            let url = target.as_str();
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log)
            {
                let _ = writeln!(file, "{url}{header}");
            }
            let route = self
                .routes
                .iter()
                .find(|r| url.starts_with(&r.url))
                .ok_or_else(|| SendFailure::other(format!("no canned response for {url}")))?;
            // A slow server, which an interrupt stops waiting for (like
            // the network client dropping its connection).
            let answer_at =
                std::time::Instant::now() + std::time::Duration::from_millis(route.delay_ms);
            while std::time::Instant::now() < answer_at {
                if interrupt.is_some_and(CallInterrupt::is_raised) {
                    return Err(SendFailure::other(super::INTERRUPTED));
                }
                std::thread::sleep(super::INTERRUPT_POLL);
            }
            if let Some(address) = route.resolves_to {
                let host = fetch.url.host_str().unwrap_or_default();
                if let Err(failure) = super::address_policy(fetch).check(host, &[address]) {
                    return Err(SendFailure::from_transport(failure, fetch));
                }
            }
            if let Some(code) = &route.fail {
                // The message the client would give, with the address a
                // blocked host resolved to.
                let message = match code.as_str() {
                    "dns-failed" => format!("{} could not be resolved: canned", fetch.origin),
                    "blocked-address" => format!(
                        "{} is on this computer or a private network (10.1.2.3); allow it in \
                         Settings › HTTP",
                        fetch.origin
                    ),
                    other => format!("{other} (canned)"),
                };
                let code: &'static str = deckpilot_plugin_abi::NetworkErrorKind::from_code(code)
                    .map_or(
                        "request-failed",
                        deckpilot_plugin_abi::NetworkErrorKind::code,
                    );
                return Err(SendFailure::from_transport((code, message), fetch));
            }
            Ok(Exchange {
                status: route.status,
                headers: route.headers.clone(),
                body: route.body.clone().into_bytes(),
            })
        }
    }
}
