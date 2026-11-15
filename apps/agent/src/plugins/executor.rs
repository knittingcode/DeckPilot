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
//!   headers and hop count exactly like a new request.
//! * `notify` as an agent notification, titled with the plugin's name.
//! * `log` into the agent's log, attributed to the plugin.
//! * Storage and credential injection are refused explicitly
//!   (`unavailable`): neither the per-plugin key-value store nor the
//!   per-plugin secret slots exist in the agent yet.

use deckpilot_core::notify::{Notification, Severity};
use deckpilot_ipc::AgentEvent;
use deckpilot_plugin_abi::capability::{Header, HttpResponse, LogLevel};
use deckpilot_plugin_abi::{CallErrorCode, CallResult, HttpMethod};
use deckpilot_plugin_broker::{Allowed, AllowedFetch, Broker, Decision, PluginId};
use deckpilot_plugin_supervisor::CapabilityExecutor;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

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

    fn send(&self, fetch: &AllowedFetch) -> Result<Exchange, String> {
        match self {
            Self::Network => network_send(fetch),
            #[cfg(feature = "test-hooks")]
            Self::Canned(canned) => canned.send(fetch),
        }
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

fn network_send(fetch: &AllowedFetch) -> Result<Exchange, String> {
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
        auth: None,
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
    let response = client::send(request, &policy, None).map_err(|(_, message)| message)?;
    if response.truncated {
        return Err(format!("the response is larger than {MAX_BODY} bytes"));
    }
    Ok(Exchange {
        status: response.status,
        headers: response.headers,
        body: response.body,
    })
}

/// The executor for one plugin's invocations.
pub(super) struct Executor {
    broker: Broker,
    /// The plugin's display name, for notifications.
    name: String,
    events: broadcast::Sender<AgentEvent>,
    transport: Transport,
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
    ) -> Self {
        Self {
            broker,
            name,
            events,
            transport,
        }
    }

    fn fetch(&self, plugin: &PluginId, allowed: &AllowedFetch) -> CallResult {
        if allowed.auth.is_some() {
            return CallResult::error(
                CallErrorCode::Unavailable,
                "credentials are not available to plugins in this version of DeckPilot",
            );
        }
        let session = self.broker.session(plugin.clone());
        let mut current = allowed.clone();
        let mut hop = 0;
        loop {
            debug!(plugin = %plugin, origin = %current.origin, "plugin fetch");
            let exchange = match self.transport.send(&current) {
                Ok(exchange) => exchange,
                Err(message) => {
                    return CallResult::error(CallErrorCode::Failed, message);
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
        match allowed {
            Allowed::Fetch(fetch) => self.fetch(plugin, fetch),
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
            Allowed::StorageGet { .. }
            | Allowed::StorageSet { .. }
            | Allowed::StorageDelete { .. } => CallResult::error(
                CallErrorCode::Unavailable,
                "plugin storage is not available in this version of DeckPilot",
            ),
        }
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

        pub fn send(&self, fetch: &AllowedFetch) -> Result<Exchange, String> {
            let url = fetch.url.as_str();
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log)
            {
                let _ = writeln!(file, "{url}");
            }
            let route = self
                .routes
                .iter()
                .find(|r| url.starts_with(&r.url))
                .ok_or_else(|| format!("no canned response for {url}"))?;
            Ok(Exchange {
                status: route.status,
                headers: route.headers.clone(),
                body: route.body.clone().into_bytes(),
            })
        }
    }
}
