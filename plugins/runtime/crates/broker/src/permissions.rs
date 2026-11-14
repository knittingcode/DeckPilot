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

use crate::origin::{Origin, OriginError};

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
    pub(crate) credentials: BTreeMap<String, CredentialGrant>,
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

impl Grants {
    /// Derives grants. Fails on the first invalid entry; never partial.
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
                ApprovedPermission::Credential {
                    key,
                    origins,
                    schemes,
                } => {
                    if !is_credential_key(key) {
                        return Err(GrantError::CredentialKey);
                    }
                    if origins.is_empty() || schemes.is_empty() {
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
            }
        }
        if g.network.len() > MAX_ORIGINS {
            return Err(GrantError::TooManyOrigins);
        }
        if g.credentials.len() > MAX_CREDENTIALS {
            return Err(GrantError::TooManyCredentials);
        }
        // A secret may only travel to origins the plugin can reach at all.
        let reachable = g
            .credentials
            .values()
            .flat_map(|c| c.origins.iter())
            .all(|o| g.network.contains_key(o));
        if !reachable {
            return Err(GrantError::CredentialOriginNotGranted);
        }
        Ok(g)
    }

    pub fn credential(&self, key: &str) -> Option<&CredentialGrant> {
        self.credentials.get(key)
    }

    pub fn origins(&self) -> impl Iterator<Item = &Origin> {
        self.network.keys()
    }

    pub fn notifications(&self) -> bool {
        self.notifications
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
    /// A credential slot names an origin without a network grant.
    CredentialOriginNotGranted,
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
            Self::CredentialOriginNotGranted => {
                f.write_str("credential origin has no network grant")
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
