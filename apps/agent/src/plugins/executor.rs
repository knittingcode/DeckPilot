//! Carries out the capability requests the broker allowed (design §5).
//!
//! The broker decides; this executor only performs what it was handed:
//!
//! * `http.fetch` with the HTTP module's client: exactly the normalized
//!   URL the broker allowed, addresses resolved once, checked against the
//!   private-network policy (loopback, private, link-local, metadata and
//!   other non-public addresses refused) and pinned for the connection, no
//!   proxy, a bounded body and a timeout. Redirects are never followed by
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
//! * `notify` as an agent notification, titled with the plugin's name.
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

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use deckpilot_core::notify::{Notification, Severity};
use deckpilot_ipc::AgentEvent;
use deckpilot_plugin_abi::capability::{AuthScheme, Header, HttpResponse, LogLevel, StorageUsage};
use deckpilot_plugin_abi::{CallErrorCode, CallResult, HttpMethod};
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
/// Response headers a plugin receives (others are dropped).
const RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "cache-control",
    "etag",
    "last-modified",
    "retry-after",
    "date",
];

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
    ) -> Result<Exchange, String> {
        match self {
            Self::Network => network_send(fetch, auth, interrupt),
            #[cfg(feature = "test-hooks")]
            Self::Canned(canned) => canned.send(fetch, auth, interrupt),
        }
    }
}

/// A credential to inject into one request: the HTTP client's own type,
/// whose `Debug` never shows the secret.
pub(super) type Auth = deckpilot_module_http::client::Auth;

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
) -> Result<Exchange, String> {
    use deckpilot_module_http::client;
    use deckpilot_module_http::policy::NetworkPolicy;
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
    // Private networks are never reachable for plugins (no per-plugin
    // `privateNetwork` grant is implemented): every resolved address must
    // be public, and the connection uses only the checked addresses.
    let policy = NetworkPolicy {
        allow_private: false,
        private_hosts: Vec::new(),
    };
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
        let response = client::send(request, &policy, Some(&token));
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        response
    });
    if token.is_cancelled() {
        return Err(INTERRUPTED.to_owned());
    }
    let response = response.map_err(|(_, message)| message)?;
    if response.truncated {
        return Err(format!("the response is larger than {MAX_BODY} bytes"));
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
    transport: Transport,
    variables: OwnVariables,
    /// Storage, credentials and settings; without it they are
    /// `unavailable`.
    access: Option<DataAccess>,
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
    ) -> Self {
        Self {
            broker,
            name,
            events,
            transport,
            variables,
            access: None,
        }
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
                Err(message) => {
                    return CallResult::error(CallErrorCode::Failed, redact(&message, &forms));
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
                    return CallResult::error(
                        reason.call_error_code(),
                        format!("a redirect was refused by the permission broker ({reason})"),
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

fn reply(exchange: Exchange) -> CallResult {
    let headers = exchange
        .headers
        .into_iter()
        .filter(|(name, _)| RESPONSE_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .map(|(name, value)| Header { name, value })
        .collect();
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
            Allowed::Notify { title, body } => {
                let notification =
                    Notification::new(&format!("{}: {title}", self.name), body, Severity::Info);
                let _ = self.events.send(AgentEvent::Notification { notification });
                CallResult::Ok(serde_json::Value::Null)
            }
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
        ) -> Result<Exchange, String> {
            use super::Auth;
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
                .ok_or_else(|| format!("no canned response for {url}"))?;
            // A slow server, which an interrupt stops waiting for (like
            // the network client dropping its connection).
            let answer_at =
                std::time::Instant::now() + std::time::Duration::from_millis(route.delay_ms);
            while std::time::Instant::now() < answer_at {
                if interrupt.is_some_and(CallInterrupt::is_raised) {
                    return Err(super::INTERRUPTED.to_owned());
                }
                std::thread::sleep(super::INTERRUPT_POLL);
            }
            Ok(Exchange {
                status: route.status,
                headers: route.headers.clone(),
                body: route.body.clone().into_bytes(),
            })
        }
    }
}
