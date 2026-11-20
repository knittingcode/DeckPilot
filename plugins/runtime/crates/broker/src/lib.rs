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
//!   Plugin API 1.9's brokered `tool.run` is not a process capability: it
//!   names `git` or `docker`, an argument array and a resource handle, and
//!   the agent's policy builds and checks the command (X3).
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

pub use deckpilot_plugin_abi::capability::RunContext;
use deckpilot_plugin_abi::capability::{Header, HttpAuth, LogLevel, NotifySeverity};
use deckpilot_plugin_abi::dialog::DialogRequest;
use deckpilot_plugin_abi::privileged::{
    OpenUrlParam, ProjectCycle, ResourceOpen, ResourceResolve, ToolAccess, ToolRun,
};
use deckpilot_plugin_abi::{CallErrorCode, CapabilityRequest, HttpMethod, NetworkErrorKind};
use serde_json::Value;
use url::Url;

pub use addr::{BlockedAddr, check_connect_addr, check_private_network_addr};
pub use audit::{AuditDecision, AuditRecord, AuditSink, MemoryAudit};
pub use origin::{Origin, OriginError, UserOrigin};
pub use permissions::{
    ApprovedPermission, ApprovedPermissions, CredentialGrant, DEFAULT_STORAGE_QUOTA_BYTES,
    GrantError, Grants, MAX_STORAGE_KEYS, MAX_STORAGE_QUOTA_BYTES, ProfileScope, ResourceGrant,
    StorageQuota, ToolGrant, UserOriginGrant,
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
    /// The origin is a granted user origin, but the path is outside its
    /// path prefix (G2).
    PathNotGranted,
    MethodNotGranted,
    Url(OriginError),
    ForbiddenHeader,
    /// A size bound of ABI 1 was broken.
    TooLarge,
    RateLimited,
    TooManyRedirects,
    /// A redirect from https to plain http (refused like every http URL,
    /// named apart so the plugin can tell it as the built-in modules do).
    InsecureRedirect,
    /// The credential slot is not granted (or was revoked).
    CredentialNotGranted,
    /// The credential slot may not be sent to this origin (including a
    /// redirect that left the original origin).
    CredentialOriginNotAllowed,
    /// The credential slot may not be sent in this scheme.
    CredentialSchemeNotAllowed,
    /// The operation is allowed only in a run the user started directly
    /// (a key, a gesture, the app, or a workflow the user started), and
    /// this one was not: a schedule, an event, the agent's start, a work
    /// session, or a run nobody said anything about (`url.open`, G8).
    NotUserInitiated,
    /// The operation is not available while a key's state is read: a
    /// state call only reads (plugin API 1.2).
    NotInStateCall,
    /// A dialog was asked for in a run the user did not start directly
    /// (`dialog.open`, plugin API 1.5; the gate of `url.open`).
    DialogNotUserInitiated,
    /// A plugin API 1.9 operation that only runs the user started may use
    /// (`resource.open`, `url.openParam`, `project.select`,
    /// `project.cycle`, and `tool.run` with `tools:docker:modify`, which a
    /// work session the user started may use too) came from a schedule, an
    /// event, the agent's start or an unknown run.
    PrivilegedNotUserInitiated,
    /// A `tool.run` argument array that does not start with a subcommand
    /// (an option of the tool itself is the host's alone).
    NoSubcommand,
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

    /// The network kind the guest sees with the denial
    /// ([`CallError::kind`](deckpilot_plugin_abi::CallError::kind)): a
    /// redirect refused for what the built-in modules' transport refuses
    /// it for too. Only the kind; where the redirect led is never named.
    pub fn network_kind(self) -> Option<NetworkErrorKind> {
        match self {
            Self::TooManyRedirects => Some(NetworkErrorKind::TooManyRedirects),
            Self::InsecureRedirect => Some(NetworkErrorKind::InsecureRedirect),
            _ => None,
        }
    }
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPlugin => f.write_str("plugin has no grants"),
            Self::NotGranted => f.write_str("not granted"),
            Self::OriginNotGranted => f.write_str("origin not granted"),
            Self::PathNotGranted => f.write_str("path outside the granted path prefix"),
            Self::MethodNotGranted => f.write_str("method not granted"),
            Self::Url(e) => write!(f, "url refused: {e}"),
            Self::ForbiddenHeader => f.write_str("header not allowed"),
            Self::TooLarge => f.write_str("request exceeds a size limit"),
            Self::RateLimited => f.write_str("rate limited"),
            Self::TooManyRedirects => f.write_str("too many redirects"),
            Self::InsecureRedirect => f.write_str("redirect from https to http"),
            Self::CredentialNotGranted => f.write_str("credential not granted"),
            Self::CredentialOriginNotAllowed => {
                f.write_str("credential may not be sent to this origin")
            }
            Self::CredentialSchemeNotAllowed => {
                f.write_str("credential may not be sent in this scheme")
            }
            Self::NotInStateCall => f.write_str("not available while a key's state is read"),
            Self::DialogNotUserInitiated => f.write_str(
                "dialogs open only in an action the user started (a key, a gesture or the app), \
                 not in one a schedule, an event or the agent's start ran",
            ),
            Self::NotUserInitiated => f.write_str(
                "pages open only in an action the user started (a key, a gesture or the app), \
                 not in one a schedule, an event or the agent's start ran",
            ),
            Self::PrivilegedNotUserInitiated => f.write_str(
                "this runs only in an action the user started (a key, a gesture or the app), \
                 not in one a schedule, an event or the agent's start ran",
            ),
            Self::NoSubcommand => {
                f.write_str("the arguments must start with a subcommand, not an option")
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
    /// The request is within a user origin (G2) the user allowed on a
    /// private network: the network layer may then connect to private
    /// ranges (RFC 1918, carrier NAT, unique local IPv6;
    /// [`check_private_network_addr`]) besides public addresses. `false`
    /// for every other request: only public addresses
    /// ([`check_connect_addr`]). Loopback and link-local are never
    /// reachable either way.
    pub private_network: bool,
}

/// What the agent may carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Allowed {
    Fetch(AllowedFetch),
    /// The executor must prefix the title with the plugin's display name.
    /// `severity` changes how it looks, never where it goes.
    Notify {
        title: String,
        body: String,
        severity: NotifySeverity,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    /// The plugin's own storage (always granted while it is enabled). The
    /// executor scopes every storage operation by the session's plugin.
    StorageGet {
        key: String,
    },
    /// The executor must refuse the write when it would exceed `quota`
    /// (read per call, so a revoked `storage` grant applies at once).
    StorageSet {
        key: String,
        value: Value,
        quota: StorageQuota,
    },
    StorageDelete {
        key: String,
    },
    StorageKeys {
        prefix: Option<String>,
    },
    StorageUsage {
        quota: StorageQuota,
    },
    /// Whether a secret is stored for a granted credential slot. The
    /// executor answers `true` or `false`, never the secret.
    CredentialHas {
        key: String,
    },
    /// The plugin's own settings.
    SettingsGet,
    /// One of the plugin's own variables (relative id, always allowed for
    /// a plugin that has grants, rate limited). The executor checks the id
    /// and type against the manifest's declarations and adds the plugin's
    /// namespace; a plugin can never name another owner's variable.
    VariablePublish {
        id: String,
        value: Value,
    },
    /// Clears one of the plugin's variables, or (`None`) all of them.
    VariableClear {
        id: Option<String>,
    },
    /// Opens `url` in the user's browser. The URL is normalized (WHATWG,
    /// IDNA), `https`, without userinfo, and its origin is one the plugin
    /// was granted `openUrl` for; the executor must open exactly this URL
    /// and log at most its origin (the path and query may carry tokens).
    OpenUrl {
        url: Url,
        origin: Origin,
    },
    /// The invocation's run context (G8): what the caller handed to
    /// [`Session::authorize_in`], within the ABI's bounds, or
    /// [`RunContext::unknown`]. The executor returns exactly this.
    RunContext(RunContext),
    /// The agent's wall clock and local UTC offset (plugin API 1.1). The
    /// executor reads the clock; the broker has none of its own to give.
    TimeNow,
    /// `length` bytes (1 to 64, already checked and counted against the
    /// plugin's byte budget) from the operating system's CSPRNG, fresh for
    /// this call.
    RandomBytes {
        length: usize,
    },
    /// Shows a dialog the plugin was granted (`dialogs`, plugin API 1.5):
    /// `request` passed every bound of [`DialogRequest::validate`], and the
    /// run was started by the user. The executor shows it titled with the
    /// plugin's own name, at most one per plugin at a time, and checks the
    /// answer against `request` before the plugin sees it.
    DialogOpen {
        request: DialogRequest,
    },
    /// The app's language (plugin API 1.6). Always allowed.
    LocaleGet,
    /// `resource.resolve` (plugin API 1.9, X2): the executor resolves one
    /// handle for the invocation and adds the bound path only when
    /// `paths` is granted.
    ResourceResolve {
        request: ResourceResolve,
        paths: bool,
    },
    /// `resource.open` (X4), granted and in a run the user started.
    ResourceOpen {
        request: ResourceOpen,
    },
    /// `tool.run` (X3): granted for its tool and for `access`, within the
    /// run-origin rule. The executor checks the argument array against
    /// its policy before it runs anything.
    ToolRun {
        request: ToolRun,
        access: ToolAccess,
    },
    /// `url.openParam` (X5), granted and in a run the user started.
    OpenUrlParam {
        request: OpenUrlParam,
    },
    /// `project.current` (X6).
    ProjectCurrent,
    /// `project.list` (X6).
    ProjectList,
    /// `project.select` (X6), granted and in a run the user started.
    ProjectSelect {
        id: String,
    },
    /// `project.cycle` (X6), granted and in a run the user started.
    ProjectCycle {
        request: ProjectCycle,
    },
    /// `profile.read` (X9) with the granted scopes only.
    ProfileRead {
        scopes: std::collections::BTreeSet<ProfileScope>,
    },
    /// One of the plugin's own translation keys (plugin API 1.6): the
    /// executor resolves it against the locale files of the plugin the
    /// session belongs to, and nothing else. Always allowed.
    LocaleTranslate {
        request: deckpilot_plugin_abi::locale::Translate,
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
    Variables,
    OpenUrl,
    OpenUrlWindow,
    Time,
    RandomBytes,
    Dialog,
    ToolRun,
    ToolModify,
    ResourceOpen,
    ProjectSwitch,
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

    /// Revokes one origin. Effective on the next call. A credential slot
    /// that targets it is withheld from it too (and is unusable when it
    /// had no other origin); every other grant stays.
    pub fn revoke_origin(&self, plugin: &PluginId, origin: &Origin) {
        self.update(plugin, |g| {
            g.network.remove(origin);
            g.suspend_unreachable_credentials();
        });
    }

    /// Revokes one user-entered origin (G2), as granted (origin and path
    /// prefix). Effective on the next call. A credential slot that only
    /// this origin bound is unusable afterwards; one with a fixed origin
    /// keeps that origin.
    pub fn revoke_user_origin(&self, plugin: &PluginId, origin: &UserOrigin) {
        self.update(plugin, |g| {
            g.user_origins.retain(|u| u.origin != *origin);
            g.suspend_unreachable_credentials();
        });
    }

    /// Revokes one credential slot. Effective on the next call.
    pub fn revoke_credential(&self, plugin: &PluginId, key: &str) {
        self.update(plugin, |g| {
            g.credentials.remove(key);
            // No user origin injects it any more either.
            g.suspend_unreachable_credentials();
        });
    }

    /// Revokes opening URLs of one origin. Effective on the next call.
    pub fn revoke_open_url(&self, plugin: &PluginId, origin: &Origin) {
        self.update(plugin, |g| {
            g.open_url.remove(origin);
        });
    }

    /// Revokes showing dialogs. Effective on the next call.
    pub fn revoke_dialogs(&self, plugin: &PluginId) {
        self.update(plugin, |g| g.dialogs = false);
    }

    pub fn revoke_notifications(&self, plugin: &PluginId) {
        self.update(plugin, |g| g.notifications = false);
    }

    /// Revokes reporting key state. Effective on the next state call.
    pub fn revoke_key_state(&self, plugin: &PluginId) {
        self.update(plugin, |g| g.key_state = false);
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
        self.take_n(plugin, class, 1)
    }

    /// Takes `n` tokens of `class` together, or none.
    fn take_n(&self, plugin: &PluginId, class: RateClass, n: u32) -> bool {
        let rate = match class {
            RateClass::Any => self.inner.limits.any,
            RateClass::Http => self.inner.limits.http,
            RateClass::Notify => self.inner.limits.notify,
            RateClass::Log => self.inner.limits.log,
            RateClass::Storage => self.inner.limits.storage,
            RateClass::Variables => self.inner.limits.variables,
            RateClass::OpenUrl => self.inner.limits.open_url,
            RateClass::OpenUrlWindow => self.inner.limits.open_url_window,
            RateClass::Time => self.inner.limits.time,
            RateClass::RandomBytes => self.inner.limits.random_bytes,
            RateClass::Dialog => self.inner.limits.dialog,
            RateClass::ToolRun => self.inner.limits.tool_run,
            RateClass::ToolModify => self.inner.limits.tool_modify,
            RateClass::ResourceOpen => self.inner.limits.resource_open,
            RateClass::ProjectSwitch => self.inner.limits.project_switch,
        };
        let now = self.inner.clock.now();
        let mut buckets = self.inner.buckets.lock().unwrap_or_else(|e| e.into_inner());
        buckets
            .entry((plugin.clone(), class))
            .or_insert_with(|| Bucket::new(rate, now))
            .take_n(rate, now, n)
    }

    /// An audit record for a decision without an [`Allowed`] value.
    fn audit_verdict(&self, plugin: &PluginId, op: &'static str, denied: Option<DenyReason>) {
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
            target: None,
            decision: denied.map_or(AuditDecision::Allowed, AuditDecision::Denied),
        });
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

    /// The single decision point for capability requests, for a call
    /// whose run nobody described: like [`Session::authorize_in`] with no
    /// run context, so an operation only the user's own runs may use
    /// (`url.open`) is denied.
    pub fn authorize(&self, request: &CapabilityRequest) -> Decision {
        self.authorize_in(request, None)
    }

    /// The single decision point for capability requests of an invocation
    /// whose run `context` describes (G8): `url.open` is allowed only when
    /// the user started the run ([`RunContext::is_user_initiated`]), and
    /// `run.context` answers `context`. `None` counts as not started by
    /// the user.
    pub fn authorize_in(
        &self,
        request: &CapabilityRequest,
        context: Option<&RunContext>,
    ) -> Decision {
        let (decision, target) = self.decide(request, context);
        self.broker
            .audit(&self.plugin, request.op(), target.as_ref(), &decision);
        decision
    }

    /// Whether the agent may ask this plugin for the state of a key now
    /// (plugin API 1.2): the plugin has grants and `keyState` is one of
    /// them. Read per call, so a revoked grant stops the next state call.
    /// Audited as `key.state`.
    pub fn authorize_key_state(&self) -> Result<(), DenyReason> {
        let verdict = match self.broker.grants(&self.plugin) {
            None => Err(DenyReason::UnknownPlugin),
            Some(grants) if !grants.key_state => Err(DenyReason::NotGranted),
            Some(_) => Ok(()),
        };
        self.broker
            .audit_verdict(&self.plugin, "key.state", verdict.err());
        verdict
    }

    /// The decision point for a capability request made **inside a state
    /// call**: only the operations of
    /// [`allowed_in_state`](deckpilot_plugin_abi::state::allowed_in_state)
    /// (reading the plugin's own storage and settings, the time, a log
    /// line), each still subject to its own grants and rate limits.
    /// Everything else is denied with [`DenyReason::NotInStateCall`]
    /// before any rate limit, and audited.
    pub fn authorize_state(&self, request: &CapabilityRequest) -> Decision {
        if deckpilot_plugin_abi::state::allowed_in_state(request) {
            return self.authorize_in(request, None);
        }
        let decision = Decision::Deny(DenyReason::NotInStateCall);
        self.broker
            .audit(&self.plugin, request.op(), None, &decision);
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

    fn decide(
        &self,
        request: &CapabilityRequest,
        context: Option<&RunContext>,
    ) -> (Decision, Option<Origin>) {
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
                let (url, origin) = match request_url(&grants, &f.url) {
                    Ok(v) => v,
                    Err(e) => return (Decision::Deny(DenyReason::Url(e)), None),
                };
                let d = match self.check_fetch(&grants, &url, &origin, f.method, &f.headers) {
                    Err(reason) => Err(reason),
                    Ok(route) => match check_auth(&grants, &origin, &url, f.auth.as_ref()) {
                        Some(reason) => Err(reason),
                        None => Ok(route),
                    },
                };
                let d = match d {
                    Err(reason) => Decision::Deny(reason),
                    Ok(_) if !b.take(&self.plugin, RateClass::Http) => {
                        Decision::Deny(DenyReason::RateLimited)
                    }
                    Ok(route) => Decision::Allow(Allowed::Fetch(AllowedFetch {
                        url,
                        origin: origin.clone(),
                        method: f.method,
                        headers: f.headers.clone(),
                        body: f.body.clone(),
                        auth: f.auth.clone(),
                        private_network: route.private_network,
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
                        severity: n.severity.unwrap_or_default(),
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
            | CapabilityRequest::StorageDelete(_)
            | CapabilityRequest::StorageKeys(_)
            | CapabilityRequest::StorageUsage(_) => {
                if !b.take(&self.plugin, RateClass::Storage) {
                    return (Decision::Deny(DenyReason::RateLimited), None);
                }
                let quota = grants.storage_quota();
                let allowed = match request {
                    CapabilityRequest::StorageGet(k) => Allowed::StorageGet { key: k.key.clone() },
                    CapabilityRequest::StorageDelete(k) => {
                        Allowed::StorageDelete { key: k.key.clone() }
                    }
                    CapabilityRequest::StorageSet(s) => Allowed::StorageSet {
                        key: s.key.clone(),
                        value: s.value.clone(),
                        quota,
                    },
                    CapabilityRequest::StorageKeys(p) => Allowed::StorageKeys {
                        prefix: p.prefix.clone(),
                    },
                    _ => Allowed::StorageUsage { quota },
                };
                (Decision::Allow(allowed), None)
            }
            CapabilityRequest::CredentialHas(c) => {
                let d = if grants.credentials.contains_key(&c.credential) {
                    Decision::Allow(Allowed::CredentialHas {
                        key: c.credential.clone(),
                    })
                } else {
                    Decision::Deny(DenyReason::CredentialNotGranted)
                };
                (d, None)
            }
            CapabilityRequest::SettingsGet(_) => (Decision::Allow(Allowed::SettingsGet), None),
            // Plugin API 1.6: the app's language and the plugin's own
            // translations, always allowed (the executor resolves only the
            // plugin's own keys, and the host counts the calls of an
            // invocation).
            CapabilityRequest::LocaleGet(_) => (Decision::Allow(Allowed::LocaleGet), None),
            CapabilityRequest::LocaleTranslate(t) => (
                Decision::Allow(Allowed::LocaleTranslate { request: t.clone() }),
                None,
            ),
            // Plugin API 1.1: always allowed while the plugin has grants,
            // rate limited. Neither reaches anything of the user's.
            CapabilityRequest::TimeNow(_) => {
                let d = if b.take(&self.plugin, RateClass::Time) {
                    Decision::Allow(Allowed::TimeNow)
                } else {
                    Decision::Deny(DenyReason::RateLimited)
                };
                (d, None)
            }
            // Counted in bytes; the length was bounded by `validate`.
            CapabilityRequest::RandomBytes(r) => {
                let d = if b.take_n(&self.plugin, RateClass::RandomBytes, r.length) {
                    Decision::Allow(Allowed::RandomBytes {
                        length: r.length as usize,
                    })
                } else {
                    Decision::Deny(DenyReason::RateLimited)
                };
                (d, None)
            }
            // The invocation's own run only (G8), always allowed.
            CapabilityRequest::RunContext(_) => {
                let context = context.cloned().unwrap_or_else(RunContext::unknown);
                (
                    Decision::Allow(Allowed::RunContext(context.bounded())),
                    None,
                )
            }
            // `openUrl` (design §5.2): the request URL goes through the
            // same strict parser as a fetch (https only, no userinfo, no
            // IP literal or local name, no control characters), and its
            // exact origin must be granted: a declared host, or a server
            // the user entered and allowed whose pages may be opened (only
            // such a server may be a private host). Audited by origin only.
            CapabilityRequest::OpenUrl(o) => {
                let (url, origin) = match open_url_target(&grants, &o.url) {
                    Ok(v) => v,
                    Err(e) => return (Decision::Deny(DenyReason::Url(e)), None),
                };
                let granted = grants.open_url.contains(&origin)
                    || grants
                        .user_origins
                        .iter()
                        .any(|u| u.open_url && u.origin.origin() == &origin);
                let d = if url.as_str().len() > deckpilot_plugin_abi::limits::MAX_OPEN_URL_LEN {
                    // Normalizing (punycode, percent-encoding) made it longer.
                    Decision::Deny(DenyReason::TooLarge)
                } else if !granted {
                    Decision::Deny(DenyReason::OriginNotGranted)
                } else if !context.is_some_and(RunContext::is_user_initiated) {
                    // Only in runs the user started (G8), checked before
                    // the rate limits so a refused call uses none.
                    Decision::Deny(DenyReason::NotUserInitiated)
                } else if !b.take(&self.plugin, RateClass::OpenUrl)
                    || !b.take(&self.plugin, RateClass::OpenUrlWindow)
                {
                    Decision::Deny(DenyReason::RateLimited)
                } else {
                    Decision::Allow(Allowed::OpenUrl {
                        url,
                        origin: origin.clone(),
                    })
                };
                (d, Some(origin))
            }
            // Plugin dialogs (plugin API 1.5): granted, only in runs the
            // user started (checked before the rate limit, like
            // `url.open`, so a refused call uses none), 6 per minute.
            CapabilityRequest::DialogOpen(request) => {
                let d = if !grants.dialogs {
                    Decision::Deny(DenyReason::NotGranted)
                } else if !context.is_some_and(RunContext::is_user_initiated) {
                    Decision::Deny(DenyReason::DialogNotUserInitiated)
                } else if !b.take(&self.plugin, RateClass::Dialog) {
                    Decision::Deny(DenyReason::RateLimited)
                } else {
                    Decision::Allow(Allowed::DialogOpen {
                        request: request.clone(),
                    })
                };
                (d, None)
            }
            CapabilityRequest::ResourceResolve(_)
            | CapabilityRequest::ResourceOpen(_)
            | CapabilityRequest::ToolRun(_)
            | CapabilityRequest::OpenUrlParam(_)
            | CapabilityRequest::ProjectCurrent(_)
            | CapabilityRequest::ProjectList(_)
            | CapabilityRequest::ProjectSelect(_)
            | CapabilityRequest::ProjectCycle(_)
            | CapabilityRequest::ProfileRead(_) => {
                (self.decide_privileged(&grants, request, context), None)
            }
            // Own variables (design §5.2): always granted, rate limited.
            CapabilityRequest::VariablesPublish(_) | CapabilityRequest::VariablesClear(_) => {
                if !b.take(&self.plugin, RateClass::Variables) {
                    return (Decision::Deny(DenyReason::RateLimited), None);
                }
                let allowed = match request {
                    CapabilityRequest::VariablesPublish(p) => Allowed::VariablePublish {
                        id: p.id.clone(),
                        value: p.value.clone(),
                    },
                    CapabilityRequest::VariablesClear(c) => {
                        Allowed::VariableClear { id: c.id.clone() }
                    }
                    _ => unreachable!("matched above"),
                };
                (Decision::Allow(allowed), None)
            }
        }
    }

    /// Plugin API 1.9's operations (`docs/marketplace/module-migration.md`
    /// §7): deny by default, each against its own grant, then the run's
    /// origin where the review restricts it (checked before the rate
    /// limits, so a refused call uses none), then its rate.
    fn decide_privileged(
        &self,
        grants: &Grants,
        request: &CapabilityRequest,
        context: Option<&RunContext>,
    ) -> Decision {
        use deckpilot_plugin_abi::capability::{RunCause, RunTrigger};
        let b = &self.broker;
        let user = context.is_some_and(RunContext::is_user_initiated);
        let deny = Decision::Deny;
        match request {
            CapabilityRequest::ResourceResolve(r) => match grants.resources {
                None => deny(DenyReason::NotGranted),
                Some(granted) => Decision::Allow(Allowed::ResourceResolve {
                    request: r.clone(),
                    paths: granted.paths,
                }),
            },
            CapabilityRequest::ResourceOpen(o) => {
                if !grants.resources.is_some_and(|r| r.open) {
                    deny(DenyReason::NotGranted)
                } else if !user {
                    deny(DenyReason::PrivilegedNotUserInitiated)
                } else if !b.take(&self.plugin, RateClass::ResourceOpen) {
                    deny(DenyReason::RateLimited)
                } else {
                    Decision::Allow(Allowed::ResourceOpen { request: o.clone() })
                }
            }
            CapabilityRequest::ToolRun(run) => {
                let Some(granted) = grants.tools.get(&run.tool) else {
                    return deny(DenyReason::NotGranted);
                };
                let Some(access) = run.access() else {
                    return deny(DenyReason::NoSubcommand);
                };
                // A work session the user started may bring services up
                // and down ("start work, compose up"), never a schedule,
                // an event or the agent's start (X3c).
                let work_session = context.is_some_and(|c| {
                    c.trigger == RunTrigger::Workflow && c.cause == Some(RunCause::WorkSession)
                });
                match access {
                    ToolAccess::Network if !granted.network => deny(DenyReason::NotGranted),
                    ToolAccess::Modify if !granted.modify => deny(DenyReason::NotGranted),
                    ToolAccess::Modify if !user && !work_session => {
                        deny(DenyReason::PrivilegedNotUserInitiated)
                    }
                    _ if !b.take(&self.plugin, RateClass::ToolRun) => deny(DenyReason::RateLimited),
                    ToolAccess::Modify if !b.take(&self.plugin, RateClass::ToolModify) => {
                        deny(DenyReason::RateLimited)
                    }
                    _ => Decision::Allow(Allowed::ToolRun {
                        request: run.clone(),
                        access,
                    }),
                }
            }
            CapabilityRequest::OpenUrlParam(o) => {
                if !grants.open_url_from_params {
                    deny(DenyReason::NotGranted)
                } else if !user {
                    deny(DenyReason::PrivilegedNotUserInitiated)
                } else if !b.take(&self.plugin, RateClass::OpenUrl)
                    || !b.take(&self.plugin, RateClass::OpenUrlWindow)
                {
                    deny(DenyReason::RateLimited)
                } else {
                    Decision::Allow(Allowed::OpenUrlParam { request: o.clone() })
                }
            }
            CapabilityRequest::ProjectCurrent(_) | CapabilityRequest::ProjectList(_) => {
                match (grants.project_context, request) {
                    (None, _) => deny(DenyReason::NotGranted),
                    (Some(_), CapabilityRequest::ProjectCurrent(_)) => {
                        Decision::Allow(Allowed::ProjectCurrent)
                    }
                    (Some(_), _) => Decision::Allow(Allowed::ProjectList),
                }
            }
            CapabilityRequest::ProjectSelect(_) | CapabilityRequest::ProjectCycle(_) => {
                if grants.project_context != Some(true) {
                    deny(DenyReason::NotGranted)
                } else if !user {
                    // Never from a lifecycle event, a schedule or the
                    // agent's start: no loop on `ProjectChanged` (X6).
                    deny(DenyReason::PrivilegedNotUserInitiated)
                } else if !b.take(&self.plugin, RateClass::ProjectSwitch) {
                    deny(DenyReason::RateLimited)
                } else {
                    Decision::Allow(match request {
                        CapabilityRequest::ProjectSelect(p) => {
                            Allowed::ProjectSelect { id: p.id.clone() }
                        }
                        CapabilityRequest::ProjectCycle(c) => Allowed::ProjectCycle { request: *c },
                        _ => unreachable!("matched above"),
                    })
                }
            }
            CapabilityRequest::ProfileRead(_) => {
                if grants.profile.is_empty() {
                    deny(DenyReason::NotGranted)
                } else {
                    Decision::Allow(Allowed::ProfileRead {
                        scopes: grants.profile.clone(),
                    })
                }
            }
            _ => unreachable!("only plugin API 1.9's operations come here"),
        }
    }

    /// Whether a fetch may go to `url`: a network grant of its exact
    /// origin, or a user origin (G2) of its exact origin whose path prefix
    /// contains it, either with the method; and allowed headers.
    fn check_fetch(
        &self,
        grants: &Grants,
        url: &Url,
        origin: &Origin,
        method: HttpMethod,
        headers: &[Header],
    ) -> Result<Route, DenyReason> {
        let route = route(grants, url, origin, method)?;
        if headers.iter().any(|h| !header_allowed(h)) {
            return Err(DenyReason::ForbiddenHeader);
        }
        Ok(route)
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
        // Every fetch is https; a redirect to http is refused below in any
        // case, but named as what it is.
        if joined.scheme() == "http" {
            return (Decision::Deny(DenyReason::InsecureRedirect), None);
        }
        // Re-run the full request check on the absolute URL: a redirect to
        // http:, to an IP literal, to userinfo or to an origin that is not
        // granted (or outside a user origin's path prefix) is denied
        // exactly like a direct request. Whether it may reach a private
        // network is decided for the new URL alone.
        let (url, origin) = match request_url(&grants, joined.as_str()) {
            Ok(v) => v,
            Err(e) => return (Decision::Deny(DenyReason::Url(e)), None),
        };
        // A credential never follows a redirect off its original origin
        // (design §5.5), and is re-checked against the current grants.
        let auth_check = || {
            if previous.auth.is_some() && origin != previous.origin {
                Some(DenyReason::CredentialOriginNotAllowed)
            } else {
                check_auth(&grants, &origin, &url, previous.auth.as_ref())
            }
        };
        let checked = self
            .check_fetch(&grants, &url, &origin, previous.method, &previous.headers)
            .and_then(|route| auth_check().map_or(Ok(route), Err));
        let d = match checked {
            Err(r) => Decision::Deny(r),
            Ok(route) => Decision::Allow(Allowed::Fetch(AllowedFetch {
                url,
                origin: origin.clone(),
                method: previous.method,
                headers: previous.headers.clone(),
                body: previous.body.clone(),
                auth: previous.auth.clone(),
                private_network: route.private_network,
            })),
        };
        (d, Some(origin))
    }
}

/// How an allowed fetch may be carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Route {
    /// Within a user origin the user allowed on a private network.
    private_network: bool,
}

/// The normalized URL and origin of a request: by the strict rules, or,
/// for a host on a private network (an IP literal or a local name), only
/// when a granted user origin may be on one. Otherwise the strict error
/// stands, so nothing changes for a plugin without such a grant.
fn request_url(grants: &Grants, text: &str) -> Result<(Url, Origin), OriginError> {
    match origin::parse_request_url(text) {
        Err(strict @ (OriginError::IpLiteralNotAllowed | OriginError::InvalidHost))
            if grants
                .user_origins
                .iter()
                .any(|u| u.private_network && u.origin.private_host()) =>
        {
            let (url, origin) = origin::parse_private_request_url(text).map_err(|_| strict)?;
            let granted = grants
                .user_origins
                .iter()
                .any(|u| u.private_network && u.origin.origin() == &origin);
            if granted {
                Ok((url, origin))
            } else {
                Err(strict)
            }
        }
        other => other,
    }
}

/// The normalized URL and origin of a page to open: by the strict rules,
/// or, for a host on a private network (an IP literal or a local name),
/// only when it is exactly a server the user allowed on a private network
/// and whose pages may be opened. Otherwise the strict error stands.
fn open_url_target(grants: &Grants, text: &str) -> Result<(Url, Origin), OriginError> {
    match origin::parse_request_url(text) {
        Err(strict @ (OriginError::IpLiteralNotAllowed | OriginError::InvalidHost)) => {
            let opens = |origin: &Origin| {
                grants.user_origins.iter().any(|u| {
                    u.open_url
                        && u.private_network
                        && u.origin.private_host()
                        && u.origin.origin() == origin
                })
            };
            match origin::parse_private_request_url(text) {
                Ok((url, origin)) if opens(&origin) => Ok((url, origin)),
                _ => Err(strict),
            }
        }
        other => other,
    }
}

/// Which grant allows `method` to `url`. A fixed network grant of the
/// origin goes first (public addresses only); then the user origins of
/// that origin whose path prefix contains the URL.
fn route(
    grants: &Grants,
    url: &Url,
    origin: &Origin,
    method: HttpMethod,
) -> Result<Route, DenyReason> {
    let mut reason = DenyReason::OriginNotGranted;
    if let Some(methods) = grants.network.get(origin) {
        if methods.contains(&method) {
            return Ok(Route {
                private_network: false,
            });
        }
        reason = DenyReason::MethodNotGranted;
    }
    let mut found = None;
    for user in grants
        .user_origins
        .iter()
        .filter(|u| u.origin.origin() == origin)
    {
        if !user.origin.matches(origin, url) {
            if reason == DenyReason::OriginNotGranted {
                reason = DenyReason::PathNotGranted;
            }
            continue;
        }
        if !user.methods.contains(&method) {
            reason = DenyReason::MethodNotGranted;
            continue;
        }
        // A private host is reachable only with the private consent
        // (never granted otherwise, `Grants::derive`).
        let private_network = user.private_network;
        found = Some(found.map_or(private_network, |p: bool| p || private_network));
    }
    found
        .map(|private_network| Route { private_network })
        .ok_or(reason)
}

fn check_auth(
    grants: &Grants,
    origin: &Origin,
    url: &Url,
    auth: Option<&HttpAuth>,
) -> Option<DenyReason> {
    let auth = auth?;
    let Some(slot) = grants.credentials.get(&auth.credential) else {
        return Some(DenyReason::CredentialNotGranted);
    };
    // Its fixed origins, or a user origin (with its path prefix) that the
    // slot is bound to.
    let bound = || {
        grants
            .user_origins
            .iter()
            .any(|u| u.credentials.contains(&auth.credential) && u.origin.matches(origin, url))
    };
    if !slot.origins.contains(origin) && !bound() {
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
