//! Approved permissions (the stored consent) and the grants derived from
//! them.
//!
//! The input is the list the user approved; the broker never reads a
//! manifest. Derivation is strict: an unknown kind, a malformed origin or a
//! kind that is never grantable to plugins (file system, process, clipboard
//! and friends, design §5.2 and #191) fails the whole derivation, so a bad
//! record can never widen into a partial grant by accident.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use deckpilot_plugin_abi::HttpMethod;
use deckpilot_plugin_abi::capability::{AuthScheme, is_credential_key};
use serde::Deserialize;
use serde_json::Value;

use crate::origin::{Origin, OriginError, UserOrigin};

/// Kinds that a plugin can never be granted in API 1.
pub const NEVER_GRANTABLE: &[&str] = &[
    "filesystem",
    "fs",
    "file",
    "files",
    "process",
    "exec",
    "shell",
    "clipboard",
    "machineResources",
    "projectContext",
    "rawDevice",
    "brightness",
    "firmware",
    "workflows",
    "customUi",
];

/// One approved permission, as stored after consent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ApprovedPermission {
    /// HTTPS to one exact origin with the listed methods.
    Network {
        origin: String,
        methods: BTreeSet<HttpMethod>,
    },
    /// Struct variant so that unknown fields are rejected.
    Notifications {},
    /// One credential slot (design §5.5): the agent may inject its secret
    /// into requests to the listed exact origins, in the listed schemes.
    /// The plugin only ever holds the slot key (a handle).
    Credential {
        key: String,
        origins: BTreeSet<String>,
        schemes: BTreeSet<AuthScheme>,
    },
    /// A larger storage quota than the default (design §5.2: 257 to
    /// 10,240 KiB). Without it a plugin still has its own storage, with
    /// [`DEFAULT_STORAGE_QUOTA_BYTES`].
    #[serde(rename_all = "camelCase")]
    Storage { quota_kb: u64 },
    /// The plugin may report the state of keys bound to its own actions
    /// (plugin API 1.2; the manifest's `device` permission with
    /// `state: true`): a typed description the agent draws, never pixels
    /// and nothing of a device (`rawDevice` stays never grantable).
    KeyState {},
    /// Opening https URLs of one exact origin in the user's browser
    /// (design §5.2 `openUrl`). Independent of `network`: it gives no
    /// request to that origin, and a network grant gives no `openUrl`.
    OpenUrl { origin: String },
    /// An origin the user entered in one of the plugin's origin settings
    /// and consented to (G2): requests with the listed methods to exactly
    /// that scheme, host and port, within `path_prefix` when there is one.
    /// The secrets of the `credentials` slots (each granted on its own)
    /// may be injected there. `private_network` is the user's separate
    /// consent to reach it on a private network: without it a private
    /// host is refused and every address must be public.
    #[serde(rename_all = "camelCase")]
    UserOrigin {
        origin: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path_prefix: Option<String>,
        methods: BTreeSet<HttpMethod>,
        #[serde(default)]
        private_network: bool,
        #[serde(default)]
        credentials: BTreeSet<String>,
        /// Pages of this server may be opened in the user's browser
        /// (`openUrl.userOrigins`): any https URL of exactly this scheme,
        /// host and port. Like every `openUrl` grant it is by origin, not
        /// by path (a server's pages are rarely under its API's path), and
        /// it gives no request beyond `methods` and `path_prefix`.
        #[serde(default)]
        open_url: bool,
    },
}

/// Storage every plugin has without a `storage` grant (design §5.2).
pub const DEFAULT_STORAGE_QUOTA_BYTES: u64 = 256 * 1024;
/// The largest quota a `storage` grant may give (design §5.2).
pub const MAX_STORAGE_QUOTA_BYTES: u64 = 10 * 1024 * 1024;
/// Keys one plugin may store, whatever its quota (design §5.2).
pub const MAX_STORAGE_KEYS: u64 = 1_000;

/// How much one plugin may store: bytes (keys and JSON values, as UTF-8)
/// and keys. The executor enforces it on every write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageQuota {
    pub bytes: u64,
    pub keys: u64,
}

impl Default for StorageQuota {
    fn default() -> Self {
        Self {
            bytes: DEFAULT_STORAGE_QUOTA_BYTES,
            keys: MAX_STORAGE_KEYS,
        }
    }
}

/// The set of approved permissions of one plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovedPermissions(pub Vec<ApprovedPermission>);

impl ApprovedPermissions {
    /// Parses a JSON array of `{ "kind": ..., ... }` objects.
    pub fn from_json(json: &str) -> Result<Self, GrantError> {
        let raw: Vec<Value> = serde_json::from_str(json).map_err(|_| GrantError::Malformed)?;
        let mut out = Vec::with_capacity(raw.len());
        for item in raw {
            let kind = item
                .get("kind")
                .and_then(Value::as_str)
                .ok_or(GrantError::Malformed)?;
            if let Some(k) = NEVER_GRANTABLE
                .iter()
                .find(|k| k.eq_ignore_ascii_case(kind))
            {
                return Err(GrantError::NeverGrantable(k));
            }
            out.push(serde_json::from_value(item).map_err(|_| GrantError::Malformed)?);
        }
        Ok(Self(out))
    }
}

/// Effective grants of one plugin: what the broker checks on every call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    pub(crate) network: BTreeMap<Origin, BTreeSet<HttpMethod>>,
    pub(crate) notifications: bool,
    /// Whether the agent may ask the plugin for key state.
    pub(crate) key_state: bool,
    pub(crate) credentials: BTreeMap<String, CredentialGrant>,
    /// A granted larger quota, in bytes.
    pub(crate) storage_quota: Option<u64>,
    /// Origins whose URLs the plugin may open in the browser.
    pub(crate) open_url: BTreeSet<Origin>,
    /// Origins the user entered and consented to (G2).
    pub(crate) user_origins: Vec<UserOriginGrant>,
    /// Credential origins withheld because they have no network grant
    /// (revoked after consent), by slot key. A slot left with no origin is
    /// not in [`Grants::credentials`] at all: it is unusable until the
    /// network grant is back.
    pub(crate) suspended: BTreeMap<String, BTreeSet<Origin>>,
}

/// One granted user-entered origin (G2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserOriginGrant {
    pub(crate) origin: UserOrigin,
    pub(crate) methods: BTreeSet<HttpMethod>,
    /// The user's consent to reach it on a private network.
    pub(crate) private_network: bool,
    /// Credential slots whose secrets may be injected there.
    pub(crate) credentials: BTreeSet<String>,
    /// Its pages may be opened in the browser.
    pub(crate) open_url: bool,
}

impl UserOriginGrant {
    pub fn origin(&self) -> &UserOrigin {
        &self.origin
    }

    pub fn private_network(&self) -> bool {
        self.private_network
    }

    /// Whether pages of this server may be opened in the browser.
    pub fn open_url(&self) -> bool {
        self.open_url
    }

    pub fn methods(&self) -> impl Iterator<Item = HttpMethod> + '_ {
        self.methods.iter().copied()
    }

    pub fn credentials(&self) -> impl Iterator<Item = &str> {
        self.credentials.iter().map(String::as_str)
    }
}

/// Where and how one credential slot may be injected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialGrant {
    pub(crate) origins: BTreeSet<Origin>,
    pub(crate) schemes: BTreeSet<AuthScheme>,
}

impl CredentialGrant {
    pub fn origins(&self) -> impl Iterator<Item = &Origin> {
        self.origins.iter()
    }

    pub fn schemes(&self) -> impl Iterator<Item = AuthScheme> + '_ {
        self.schemes.iter().copied()
    }
}

/// Most credential slots one plugin may be granted (the manifest's limit).
pub const MAX_CREDENTIALS: usize = 8;

/// Most origins one plugin may be granted.
pub const MAX_ORIGINS: usize = 64;

/// Most user-entered origins one plugin may be granted (one per origin
/// setting; the manifest allows four).
pub const MAX_USER_ORIGINS: usize = 8;

impl Grants {
    /// Derives grants. Fails on the first invalid entry; never partial. A
    /// credential origin that lacks a network grant is not invalid (the
    /// user may revoke a host at any time): only that origin is withheld
    /// from its slot ([`Grants::suspended_credentials`]).
    pub fn derive(approved: &ApprovedPermissions) -> Result<Self, GrantError> {
        let mut g = Grants::default();
        for p in &approved.0 {
            match p {
                ApprovedPermission::Network { origin, methods } => {
                    if methods.is_empty() {
                        return Err(GrantError::NoMethods);
                    }
                    let o = Origin::parse_grant(origin).map_err(GrantError::Origin)?;
                    g.network
                        .entry(o)
                        .or_default()
                        .extend(methods.iter().copied());
                }
                ApprovedPermission::Notifications {} => g.notifications = true,
                ApprovedPermission::KeyState {} => g.key_state = true,
                ApprovedPermission::Credential {
                    key,
                    origins,
                    schemes,
                } => {
                    if !is_credential_key(key) {
                        return Err(GrantError::CredentialKey);
                    }
                    // A slot without fixed origins is valid only when a
                    // user origin binds it (checked below).
                    if schemes.is_empty() {
                        return Err(GrantError::EmptyCredential);
                    }
                    let mut slot = CredentialGrant {
                        schemes: schemes.clone(),
                        ..CredentialGrant::default()
                    };
                    for o in origins {
                        slot.origins
                            .insert(Origin::parse_grant(o).map_err(GrantError::Origin)?);
                    }
                    if g.credentials.insert(key.clone(), slot).is_some() {
                        return Err(GrantError::DuplicateCredential);
                    }
                }
                ApprovedPermission::OpenUrl { origin } => {
                    g.open_url
                        .insert(Origin::parse_grant(origin).map_err(GrantError::Origin)?);
                }
                ApprovedPermission::UserOrigin {
                    origin,
                    path_prefix,
                    methods,
                    private_network,
                    credentials,
                    open_url,
                } => {
                    if methods.is_empty() {
                        return Err(GrantError::NoMethods);
                    }
                    // The record holds the normalized origin and prefix the
                    // user consented to; it must read back as exactly that.
                    let text = format!("{origin}{}", path_prefix.as_deref().unwrap_or(""));
                    let parsed = UserOrigin::parse(&text).map_err(GrantError::Origin)?;
                    if parsed.to_string() != text {
                        return Err(GrantError::Origin(OriginError::NotAnOrigin));
                    }
                    if parsed.private_host() && !private_network {
                        return Err(GrantError::PrivateNetworkNotGranted);
                    }
                    if credentials.iter().any(|k| !is_credential_key(k)) {
                        return Err(GrantError::CredentialKey);
                    }
                    g.user_origins.push(UserOriginGrant {
                        origin: parsed,
                        methods: methods.clone(),
                        private_network: *private_network,
                        credentials: credentials.clone(),
                        open_url: *open_url,
                    });
                }
                ApprovedPermission::Storage { quota_kb } => {
                    let bytes = quota_kb.saturating_mul(1024);
                    if bytes <= DEFAULT_STORAGE_QUOTA_BYTES || bytes > MAX_STORAGE_QUOTA_BYTES {
                        return Err(GrantError::StorageQuota);
                    }
                    if g.storage_quota.replace(bytes).is_some() {
                        return Err(GrantError::StorageQuota);
                    }
                }
            }
        }
        if g.network.len() > MAX_ORIGINS
            || g.open_url.len() > MAX_ORIGINS
            || g.user_origins.len() > MAX_USER_ORIGINS
        {
            return Err(GrantError::TooManyOrigins);
        }
        // A slot with no fixed origin exists only for its user origin: a
        // record without either is malformed (the agent passes such a slot
        // only together with the consented user origin that binds it).
        let unbound = g.credentials.iter().any(|(key, slot)| {
            slot.origins.is_empty() && !g.user_origins.iter().any(|u| u.credentials.contains(key))
        });
        if unbound {
            return Err(GrantError::EmptyCredential);
        }
        if g.credentials.len() > MAX_CREDENTIALS {
            return Err(GrantError::TooManyCredentials);
        }
        g.suspend_unreachable_credentials();
        Ok(g)
    }

    /// A secret may only travel to origins the plugin can reach at all: a
    /// credential origin without a network grant (the user revoked that
    /// host after consenting to the slot) is withheld from its slot, and a
    /// slot left without origins is removed, so every call using it is
    /// denied as not granted. Everything else the plugin was granted stays
    /// in force; the slot comes back with the network grant.
    ///
    /// A user-entered origin (G2) is reachable by its own consent, not by a
    /// network grant of a declared host: a slot a granted user origin binds
    /// stays for that origin even when it has no fixed origin left, and
    /// is removed like any other once that user origin is gone too. A user
    /// origin never injects a slot that is not granted itself.
    pub(crate) fn suspend_unreachable_credentials(&mut self) {
        let network = &self.network;
        let suspended = &mut self.suspended;
        let user_origins = &self.user_origins;
        self.credentials.retain(|key, slot| {
            let (kept, withheld): (BTreeSet<Origin>, BTreeSet<Origin>) =
                std::mem::take(&mut slot.origins)
                    .into_iter()
                    .partition(|o| network.contains_key(o));
            if !withheld.is_empty() {
                suspended.entry(key.clone()).or_default().extend(withheld);
            }
            slot.origins = kept;
            !slot.origins.is_empty() || user_origins.iter().any(|u| u.credentials.contains(key))
        });
        let granted = &self.credentials;
        for user in &mut self.user_origins {
            user.credentials.retain(|key| granted.contains_key(key));
        }
    }

    /// Credential origins withheld from their slot because the plugin has
    /// no network grant for them, by slot key. A slot whose every origin
    /// is withheld is not granted at all ([`Grants::credential`] is
    /// `None`).
    pub fn suspended_credentials(&self) -> &BTreeMap<String, BTreeSet<Origin>> {
        &self.suspended
    }

    pub fn credential(&self, key: &str) -> Option<&CredentialGrant> {
        self.credentials.get(key)
    }

    pub fn origins(&self) -> impl Iterator<Item = &Origin> {
        self.network.keys()
    }

    /// Origins whose URLs the plugin may open in the browser.
    pub fn open_url_origins(&self) -> impl Iterator<Item = &Origin> {
        self.open_url.iter()
    }

    /// The user-entered origins the plugin was granted (G2).
    pub fn user_origins(&self) -> impl Iterator<Item = &UserOriginGrant> {
        self.user_origins.iter()
    }

    pub fn notifications(&self) -> bool {
        self.notifications
    }

    /// The plugin's storage quota: the granted one, or the default.
    pub fn storage_quota(&self) -> StorageQuota {
        StorageQuota {
            bytes: self.storage_quota.unwrap_or(DEFAULT_STORAGE_QUOTA_BYTES),
            keys: MAX_STORAGE_KEYS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    Malformed,
    NeverGrantable(&'static str),
    Origin(OriginError),
    NoMethods,
    TooManyOrigins,
    CredentialKey,
    EmptyCredential,
    DuplicateCredential,
    TooManyCredentials,
    /// A storage quota outside 257 to 10,240 KiB, or granted twice.
    StorageQuota,
    /// A user origin on a private network without the user's consent to
    /// reach it there.
    PrivateNetworkNotGranted,
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => f.write_str("malformed permission record"),
            Self::NeverGrantable(k) => {
                write!(f, "permission kind {k} is never grantable to plugins")
            }
            Self::Origin(e) => write!(f, "invalid origin: {e}"),
            Self::NoMethods => f.write_str("network permission without methods"),
            Self::TooManyOrigins => f.write_str("too many origins"),
            Self::CredentialKey => f.write_str("invalid credential key"),
            Self::EmptyCredential => f.write_str("credential without origins or schemes"),
            Self::DuplicateCredential => f.write_str("credential key granted twice"),
            Self::TooManyCredentials => f.write_str("too many credentials"),
            Self::StorageQuota => f.write_str("storage quota outside 257 to 10240 KiB"),
            Self::PrivateNetworkNotGranted => {
                f.write_str("user origin on a private network without consent")
            }
        }
    }
}

impl std::error::Error for GrantError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_exact_grants() {
        let p = ApprovedPermissions::from_json(
            r#"[{"kind":"network","origin":"https://API.example.com","methods":["GET"]},
                {"kind":"network","origin":"https://api.example.com:443","methods":["POST"]},
                {"kind":"notifications"}]"#,
        )
        .unwrap();
        let g = Grants::derive(&p).unwrap();
        assert_eq!(g.network.len(), 1);
        let methods = g.network.values().next().unwrap();
        assert_eq!(methods.len(), 2);
        assert!(g.notifications());
    }

    #[test]
    fn open_url_grants_are_exact_origins_of_their_own() {
        let p = ApprovedPermissions::from_json(
            r#"[{"kind":"openUrl","origin":"https://GitHub.com"},
                {"kind":"openUrl","origin":"https://github.com:443/"}]"#,
        )
        .unwrap();
        let g = Grants::derive(&p).unwrap();
        assert_eq!(
            g.open_url_origins()
                .map(Origin::to_string)
                .collect::<Vec<_>>(),
            ["https://github.com"]
        );
        // Opening gives no network access, and no notifications.
        assert_eq!(g.origins().count(), 0);
        assert!(!g.notifications());
        for bad in [
            r#"[{"kind":"openUrl","origin":"https://*.github.com"}]"#,
            r#"[{"kind":"openUrl","origin":"http://github.com"}]"#,
            r#"[{"kind":"openUrl","origin":"file:///etc"}]"#,
            r#"[{"kind":"openUrl","origin":"https://localhost"}]"#,
            r#"[{"kind":"openUrl","origin":"https://192.168.1.1"}]"#,
            r#"[{"kind":"openUrl","origin":"https://github.com/acme"}]"#,
            r#"[{"kind":"openUrl","origin":"https://u@github.com"}]"#,
        ] {
            let p = ApprovedPermissions::from_json(bad).unwrap();
            assert!(
                matches!(Grants::derive(&p), Err(GrantError::Origin(_))),
                "{bad}"
            );
        }
        for malformed in [
            r#"[{"kind":"openUrl"}]"#,
            r#"[{"kind":"openUrl","origin":"https://github.com","hosts":["*"]}]"#,
            r#"[{"kind":"openUrl","origins":["https://github.com"]}]"#,
        ] {
            assert_eq!(
                ApprovedPermissions::from_json(malformed),
                Err(GrantError::Malformed),
                "{malformed}"
            );
        }
        let many: Vec<String> = (0..=MAX_ORIGINS)
            .map(|i| format!(r#"{{"kind":"openUrl","origin":"https://h{i}.example.com"}}"#))
            .collect();
        let p = ApprovedPermissions::from_json(&format!("[{}]", many.join(","))).unwrap();
        assert_eq!(Grants::derive(&p), Err(GrantError::TooManyOrigins));
    }

    #[test]
    fn never_grantable_kinds_fail() {
        for kind in [
            "filesystem",
            "FileSystem",
            "process",
            "clipboard",
            "exec",
            "fs",
        ] {
            let json = format!(r#"[{{"kind":"{kind}","path":"/"}}]"#);
            assert!(
                matches!(
                    ApprovedPermissions::from_json(&json),
                    Err(GrantError::NeverGrantable(_))
                ),
                "{kind}"
            );
        }
    }

    #[test]
    fn malformed_records_fail_whole_derivation() {
        for json in [
            r#"[{"kind":"telepathy"}]"#,
            r#"[{"kind":"network","origin":"https://a.example.com"}]"#,
            r#"[{"kind":"network","origin":"https://a.example.com","methods":["GET"],"hosts":["*"]}]"#,
            r#"[{"origin":"https://a.example.com"}]"#,
            r#"{"kind":"notifications"}"#,
            r#"[{"kind":"notifications","plugin":"other.plugin"}]"#,
        ] {
            assert_eq!(
                ApprovedPermissions::from_json(json),
                Err(GrantError::Malformed),
                "{json}"
            );
        }
        let p = ApprovedPermissions::from_json(
            r#"[{"kind":"network","origin":"https://ok.example.com","methods":["GET"]},
                {"kind":"network","origin":"https://*.example.com","methods":["GET"]}]"#,
        )
        .unwrap();
        assert!(matches!(Grants::derive(&p), Err(GrantError::Origin(_))));
        let empty = ApprovedPermissions::from_json(
            r#"[{"kind":"network","origin":"https://ok.example.com","methods":[]}]"#,
        )
        .unwrap();
        assert_eq!(Grants::derive(&empty), Err(GrantError::NoMethods));
    }
}
