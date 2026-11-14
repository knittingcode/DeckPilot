//! Permission broker for DeckPilot plugins (design §5).
//!
//! Pure decision logic: no I/O, no network, no storage. The agent's
//! capability executors carry out what the broker allows, using the
//! normalized data in [`Allowed`] (never the plugin's raw strings).
//!
//! * **Deny by default.** A plugin without grants can only log and use its
//!   own storage (always-on, low-tier capabilities, still rate limited).
//! * **Identity from the channel.** The only way to ask for a decision is a
//!   [`Session`], created by the supervisor for the process it spawned. No
//!   request carries a plugin id.
//! * **Grants read per call.** [`Broker::set_grants`] and the revoke
//!   functions take effect on the next call of any session.
//! * **Audit without values** ([`audit`]) and **rate limits** ([`rate`]).
//! * **Never grantable**: file system, process, clipboard (see
//!   [`permissions::NEVER_GRANTABLE`]); there is no request type for them.
#![forbid(unsafe_code)]

pub mod addr;
pub mod audit;
pub mod origin;
pub mod permissions;
pub mod plugin_id;
pub mod rate;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, RwLock};

use deckpilot_plugin_abi::capability::{Header, HttpAuth, LogLevel};
use deckpilot_plugin_abi::{CallErrorCode, CapabilityRequest, HttpMethod};
use serde_json::Value;
use url::Url;

pub use addr::{BlockedAddr, check_connect_addr};
pub use audit::{AuditDecision, AuditRecord, AuditSink, MemoryAudit};
pub use origin::{Origin, OriginError};
pub use permissions::{
    ApprovedPermission, ApprovedPermissions, CredentialGrant, GrantError, Grants,
};
pub use plugin_id::PluginId;
pub use rate::{Clock, ManualClock, Rate, RateLimits, SystemClock};

use rate::Bucket;

/// Most redirects one brokered fetch may follow.
pub const MAX_REDIRECTS: u32 = 5;

/// Headers a plugin may never set. Credentials are injected by the agent
/// from handles (design §5.5); framing and routing headers belong to the
/// HTTP client.
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "authorization",
    "proxy-authorization",
    "cookie",
    "cookie2",
    "set-cookie",
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "content-length",
    "expect",
    "forwarded",
    "via",
    "origin",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-real-ip",
    "x-http-method-override",
    "x-http-method",
    "x-method-override",
];
const FORBIDDEN_HEADER_PREFIXES: &[&str] = &["proxy-", "sec-"];

/// Why a request was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// No grants were ever set for this plugin, or it was removed.
    UnknownPlugin,
    /// The capability kind is not granted (or was revoked).
    NotGranted,
    OriginNotGranted,
    MethodNotGranted,
    Url(OriginError),
    ForbiddenHeader,
    /// A size bound of ABI 1 was broken.
    TooLarge,
    RateLimited,
    TooManyRedirects,
    /// The credential slot is not granted (or was revoked).
    CredentialNotGranted,
    /// The credential slot may not be sent to this origin (including a
    /// redirect that left the original origin).
    CredentialOriginNotAllowed,
    /// The credential slot may not be sent in this scheme.
    CredentialSchemeNotAllowed,
}

impl DenyReason {
    /// The error code the guest sees.
    pub fn call_error_code(self) -> CallErrorCode {
        match self {
            Self::RateLimited => CallErrorCode::Throttled,
            Self::TooLarge | Self::ForbiddenHeader => CallErrorCode::BadRequest,
            _ => CallErrorCode::NotPermitted,
        }
    }
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPlugin => f.write_str("plugin has no grants"),
            Self::NotGranted => f.write_str("not granted"),
            Self::OriginNotGranted => f.write_str("origin not granted"),
            Self::MethodNotGranted => f.write_str("method not granted"),
            Self::Url(e) => write!(f, "url refused: {e}"),
            Self::ForbiddenHeader => f.write_str("header not allowed"),
            Self::TooLarge => f.write_str("request exceeds a size limit"),
            Self::RateLimited => f.write_str("rate limited"),
            Self::TooManyRedirects => f.write_str("too many redirects"),
            Self::CredentialNotGranted => f.write_str("credential not granted"),
            Self::CredentialOriginNotAllowed => {
                f.write_str("credential may not be sent to this origin")
            }
            Self::CredentialSchemeNotAllowed => {
                f.write_str("credential may not be sent in this scheme")
            }
        }
    }
}

/// A fetch the network layer may perform, already normalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedFetch {
    /// The normalized URL. The network layer must request exactly this URL,
    /// check every resolved address with [`check_connect_addr`], and send
    /// any redirect through [`Session::authorize_redirect`].
    pub url: Url,
    pub origin: Origin,
    pub method: HttpMethod,
    pub headers: Vec<Header>,
    pub body: Option<String>,
    /// A credential handle the broker allowed for this origin and scheme.
    /// The executor looks the secret up by `credential` (never from the
    /// plugin) and adds it as `scheme` prescribes.
    pub auth: Option<HttpAuth>,
}

/// What the agent may carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Allowed {
    Fetch(AllowedFetch),
    /// The executor must prefix the title with the plugin's display name.
    Notify {
        title: String,
        body: String,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    StorageGet {
        key: String,
    },
    StorageSet {
        key: String,
        value: Value,
    },
    StorageDelete {
        key: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "one short-lived value per capability call; boxing would only add an allocation"
)]
pub enum Decision {
    Allow(Allowed),
    Deny(DenyReason),
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow(_))
    }

    pub fn denial(&self) -> Option<DenyReason> {
        match self {
            Self::Deny(r) => Some(*r),
            Self::Allow(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RateClass {
    Any,
    Http,
    Notify,
    Log,
    Storage,
}

#[derive(Debug)]
struct Inner {
    grants: RwLock<HashMap<PluginId, Arc<Grants>>>,
    buckets: Mutex<HashMap<(PluginId, RateClass), Bucket>>,
    limits: RateLimits,
    audit: Arc<dyn AuditSink>,
    clock: Arc<dyn Clock>,
    seq: Mutex<u64>,
}

/// The broker. Cheap to clone; all clones share grants, limits and audit.
#[derive(Debug, Clone)]
pub struct Broker {
    inner: Arc<Inner>,
}

impl Broker {
    pub fn new(audit: Arc<dyn AuditSink>, clock: Arc<dyn Clock>, limits: RateLimits) -> Self {
        Self {
            inner: Arc::new(Inner {
                grants: RwLock::new(HashMap::new()),
                buckets: Mutex::new(HashMap::new()),
                limits,
                audit,
                clock,
                seq: Mutex::new(0),
            }),
        }
    }

    /// Replaces a plugin's grants with those derived from its approved
    /// permissions. On error the previous grants stay unchanged.
    pub fn set_grants(
        &self,
        plugin: &PluginId,
        approved: &ApprovedPermissions,
    ) -> Result<(), GrantError> {
        let grants = Grants::derive(approved)?;
        self.write_grants().insert(plugin.clone(), Arc::new(grants));
        Ok(())
    }

    /// Revokes one origin. Effective on the next call.
    pub fn revoke_origin(&self, plugin: &PluginId, origin: &Origin) {
        self.update(plugin, |g| {
            g.network.remove(origin);
        });
    }

    /// Revokes one credential slot. Effective on the next call.
    pub fn revoke_credential(&self, plugin: &PluginId, key: &str) {
        self.update(plugin, |g| {
            g.credentials.remove(key);
        });
    }

    pub fn revoke_notifications(&self, plugin: &PluginId) {
        self.update(plugin, |g| g.notifications = false);
    }

    /// Revokes everything; the plugin is unknown afterwards (only the
    /// always-on capabilities are denied too).
    pub fn remove_plugin(&self, plugin: &PluginId) {
        self.write_grants().remove(plugin);
    }

    /// A session for the plugin whose host process the caller spawned. The
    /// session is the plugin's identity for every later decision.
    pub fn session(&self, plugin: PluginId) -> Session {
        Session {
            broker: self.clone(),
            plugin,
        }
    }

    fn update(&self, plugin: &PluginId, f: impl FnOnce(&mut Grants)) {
        let mut map = self.write_grants();
        if let Some(g) = map.get_mut(plugin) {
            let mut next = (**g).clone();
            f(&mut next);
            *g = Arc::new(next);
        }
    }

    fn write_grants(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<PluginId, Arc<Grants>>> {
        self.inner.grants.write().unwrap_or_else(|e| e.into_inner())
    }

    fn grants(&self, plugin: &PluginId) -> Option<Arc<Grants>> {
        self.inner
            .grants
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(plugin)
            .cloned()
    }

    fn take(&self, plugin: &PluginId, class: RateClass) -> bool {
        let rate = match class {
            RateClass::Any => self.inner.limits.any,
            RateClass::Http => self.inner.limits.http,
            RateClass::Notify => self.inner.limits.notify,
            RateClass::Log => self.inner.limits.log,
            RateClass::Storage => self.inner.limits.storage,
        };
        let now = self.inner.clock.now();
        let mut buckets = self.inner.buckets.lock().unwrap_or_else(|e| e.into_inner());
        buckets
            .entry((plugin.clone(), class))
            .or_insert_with(|| Bucket::new(rate, now))
            .take(rate, now)
    }

    fn audit(
        &self,
        plugin: &PluginId,
        op: &'static str,
        target: Option<&Origin>,
        decision: &Decision,
    ) {
        let seq = {
            let mut s = self.inner.seq.lock().unwrap_or_else(|e| e.into_inner());
            *s += 1;
            *s
        };
        self.inner.audit.record(AuditRecord {
            seq,
            at_ms: self.inner.clock.wall_ms(),
            plugin: plugin.clone(),
            op,
            target: target.map(Origin::to_string),
            decision: match decision {
                Decision::Allow(_) => AuditDecision::Allowed,
                Decision::Deny(r) => AuditDecision::Denied(*r),
            },
        });
    }
}

/// One plugin's view of the broker, bound to its channel.
#[derive(Debug)]
pub struct Session {
    broker: Broker,
    plugin: PluginId,
}

impl Session {
    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// The single decision point for capability requests.
    pub fn authorize(&self, request: &CapabilityRequest) -> Decision {
        let (decision, target) = self.decide(request);
        self.broker
            .audit(&self.plugin, request.op(), target.as_ref(), &decision);
        decision
    }

    /// Re-checks a redirect of an allowed fetch (`location` as received,
    /// resolved against the previous URL). `hop` counts from 1.
    pub fn authorize_redirect(
        &self,
        previous: &AllowedFetch,
        location: &str,
        hop: u32,
    ) -> Decision {
        let (decision, target) = self.decide_redirect(previous, location, hop);
        self.broker
            .audit(&self.plugin, "http.redirect", target.as_ref(), &decision);
        decision
    }

    fn decide(&self, request: &CapabilityRequest) -> (Decision, Option<Origin>) {
        let b = &self.broker;
        let Some(grants) = b.grants(&self.plugin) else {
            return (Decision::Deny(DenyReason::UnknownPlugin), None);
        };
        if !b.take(&self.plugin, RateClass::Any) {
            return (Decision::Deny(DenyReason::RateLimited), None);
        }
        if request.validate().is_err() {
            return (Decision::Deny(DenyReason::TooLarge), None);
        }
        match request {
            CapabilityRequest::HttpFetch(f) => {
                let (url, origin) = match origin::parse_request_url(&f.url) {
                    Ok(v) => v,
                    Err(e) => return (Decision::Deny(DenyReason::Url(e)), None),
                };
                let d = self
                    .check_fetch(&grants, &url, &origin, f.method, &f.headers)
                    .or_else(|| check_auth(&grants, &origin, f.auth.as_ref()));
                let d = match d {
                    Some(reason) => Decision::Deny(reason),
                    None if !b.take(&self.plugin, RateClass::Http) => {
                        Decision::Deny(DenyReason::RateLimited)
                    }
                    None => Decision::Allow(Allowed::Fetch(AllowedFetch {
                        url,
                        origin: origin.clone(),
                        method: f.method,
                        headers: f.headers.clone(),
                        body: f.body.clone(),
                        auth: f.auth.clone(),
                    })),
                };
                (d, Some(origin))
            }
            CapabilityRequest::Notify(n) => {
                let d = if !grants.notifications {
                    Decision::Deny(DenyReason::NotGranted)
                } else if !b.take(&self.plugin, RateClass::Notify) {
                    Decision::Deny(DenyReason::RateLimited)
                } else {
                    Decision::Allow(Allowed::Notify {
                        title: n.title.clone(),
                        body: n.body.clone(),
                    })
                };
                (d, None)
            }
            CapabilityRequest::Log(l) => {
                let d = if b.take(&self.plugin, RateClass::Log) {
                    Decision::Allow(Allowed::Log {
                        level: l.level,
                        message: l.message.clone(),
                    })
                } else {
                    Decision::Deny(DenyReason::RateLimited)
                };
                (d, None)
            }
            CapabilityRequest::StorageGet(_)
            | CapabilityRequest::StorageSet(_)
            | CapabilityRequest::StorageDelete(_) => {
                if !b.take(&self.plugin, RateClass::Storage) {
                    return (Decision::Deny(DenyReason::RateLimited), None);
                }
                let allowed = match request {
                    CapabilityRequest::StorageGet(k) => Allowed::StorageGet { key: k.key.clone() },
                    CapabilityRequest::StorageDelete(k) => {
                        Allowed::StorageDelete { key: k.key.clone() }
                    }
                    CapabilityRequest::StorageSet(s) => Allowed::StorageSet {
                        key: s.key.clone(),
                        value: s.value.clone(),
                    },
                    _ => unreachable!("matched above"),
                };
                (Decision::Allow(allowed), None)
            }
        }
    }

    fn check_fetch(
        &self,
        grants: &Grants,
        _url: &Url,
        origin: &Origin,
        method: HttpMethod,
        headers: &[Header],
    ) -> Option<DenyReason> {
        let Some(methods) = grants.network.get(origin) else {
            return Some(DenyReason::OriginNotGranted);
        };
        if !methods.contains(&method) {
            return Some(DenyReason::MethodNotGranted);
        }
        if headers.iter().any(|h| !header_allowed(h)) {
            return Some(DenyReason::ForbiddenHeader);
        }
        None
    }

    fn decide_redirect(
        &self,
        previous: &AllowedFetch,
        location: &str,
        hop: u32,
    ) -> (Decision, Option<Origin>) {
        let Some(grants) = self.broker.grants(&self.plugin) else {
            return (Decision::Deny(DenyReason::UnknownPlugin), None);
        };
        if hop == 0 || hop > MAX_REDIRECTS {
            return (Decision::Deny(DenyReason::TooManyRedirects), None);
        }
        if location
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        {
            return (
                Decision::Deny(DenyReason::Url(OriginError::IllegalCharacters)),
                None,
            );
        }
        let Ok(joined) = previous.url.join(location) else {
            return (
                Decision::Deny(DenyReason::Url(OriginError::Unparsable)),
                None,
            );
        };
        // Re-run the full request check on the absolute URL: a redirect to
        // http:, to an IP literal, to userinfo or to an origin that is not
        // granted is denied exactly like a direct request.
        let (url, origin) = match origin::parse_request_url(joined.as_str()) {
            Ok(v) => v,
            Err(e) => return (Decision::Deny(DenyReason::Url(e)), None),
        };
        // A credential never follows a redirect off its original origin
        // (design §5.5), and is re-checked against the current grants.
        let auth_check = || {
            if previous.auth.is_some() && origin != previous.origin {
                Some(DenyReason::CredentialOriginNotAllowed)
            } else {
                check_auth(&grants, &origin, previous.auth.as_ref())
            }
        };
        let d = match self
            .check_fetch(&grants, &url, &origin, previous.method, &previous.headers)
            .or_else(auth_check)
        {
            Some(r) => Decision::Deny(r),
            None => Decision::Allow(Allowed::Fetch(AllowedFetch {
                url,
                origin: origin.clone(),
                method: previous.method,
                headers: previous.headers.clone(),
                body: previous.body.clone(),
                auth: previous.auth.clone(),
            })),
        };
        (d, Some(origin))
    }
}

fn check_auth(grants: &Grants, origin: &Origin, auth: Option<&HttpAuth>) -> Option<DenyReason> {
    let auth = auth?;
    let Some(slot) = grants.credentials.get(&auth.credential) else {
        return Some(DenyReason::CredentialNotGranted);
    };
    if !slot.origins.contains(origin) {
        return Some(DenyReason::CredentialOriginNotAllowed);
    }
    if !slot.schemes.contains(&auth.scheme) {
        return Some(DenyReason::CredentialSchemeNotAllowed);
    }
    None
}

fn header_allowed(h: &Header) -> bool {
    let name = h.name.to_ascii_lowercase();
    let token = !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c));
    let value_ok = !h.value.bytes().any(|c| c == b'\r' || c == b'\n' || c == 0);
    token
        && value_ok
        && !FORBIDDEN_HEADERS.contains(&name.as_str())
        && !FORBIDDEN_HEADER_PREFIXES
            .iter()
            .any(|p| name.starts_with(p))
}
