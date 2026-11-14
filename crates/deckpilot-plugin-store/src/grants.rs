//! Normalized grants (architecture section 5.4) and the permission diff of
//! an update (section 13.2).
//!
//! A manifest's permissions become one [`GrantKey`] per normalized grant:
//! per host for `network` and `openUrl`, per ref for `credentials`, per
//! read entry for `variables`, one key for each other kind (two for
//! `device`, one extra for `privateNetwork`). Consent is stored per
//! (plugin, key), never per version.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use deckpilot_plugin_manifest::{
    CredentialScheme, HttpMethod, Manifest, Permission, PermissionKind, ProfileScope,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The key of the `privateNetwork` grant (no host has a name without a dot).
pub const PRIVATE_NETWORK_KEY: &str = "private-network";

const ALL_METHODS: [HttpMethod; 6] = [
    HttpMethod::Get,
    HttpMethod::Head,
    HttpMethod::Post,
    HttpMethod::Put,
    HttpMethod::Patch,
    HttpMethod::Delete,
];

const KINDS: [PermissionKind; 9] = [
    PermissionKind::Network,
    PermissionKind::Credentials,
    PermissionKind::Notifications,
    PermissionKind::Profile,
    PermissionKind::Variables,
    PermissionKind::Storage,
    PermissionKind::OpenUrl,
    PermissionKind::Background,
    PermissionKind::Device,
];

pub(crate) fn parse_kind(text: &str) -> Option<PermissionKind> {
    KINDS.into_iter().find(|kind| kind.as_str() == text)
}

/// One normalized grant: the permission kind and its key (a host, a
/// credential ref, a variable entry, `state`/`icons` for `device`,
/// [`PRIVATE_NETWORK_KEY`], or empty for kinds with a single grant).
///
/// Written as `kind` or `kind:key` (`network:api.github.com`). It never
/// holds a secret, so it is what the audit log records as the target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrantKey {
    pub kind: PermissionKind,
    pub key: String,
}

impl GrantKey {
    pub fn new(kind: PermissionKind, key: impl Into<String>) -> Self {
        Self {
            kind,
            key: key.into(),
        }
    }
}

impl fmt::Display for GrantKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.key.is_empty() {
            f.write_str(self.kind.as_str())
        } else {
            write!(f, "{}:{}", self.kind.as_str(), self.key)
        }
    }
}

impl FromStr for GrantKey {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (kind, key) = text.split_once(':').unwrap_or((text, ""));
        let kind =
            parse_kind(kind).ok_or_else(|| format!("unknown permission kind in {text:?}"))?;
        Ok(Self::new(kind, key))
    }
}

impl Serialize for GrantKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for GrantKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// What a grant allows, beyond its key (`grant_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum GrantScope {
    /// A `network` host with its methods (all methods when the manifest
    /// lists none).
    Host {
        methods: Vec<HttpMethod>,
    },
    PrivateNetwork,
    /// A credential ref, with the hosts and schemes of its slot.
    Credential {
        hosts: Vec<String>,
        schemes: Vec<CredentialScheme>,
    },
    Notifications,
    Profile {
        read: Vec<ProfileScope>,
    },
    /// A `variables.read` entry.
    Variable,
    #[serde(rename_all = "camelCase")]
    Storage {
        quota_kb: u64,
    },
    /// An `openUrl` host.
    OpenUrl,
    #[serde(rename_all = "camelCase")]
    Background {
        min_interval_seconds: u64,
    },
    DeviceState,
    DeviceIcons,
}

/// The consent tier of a grant (architecture section 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Low,
    Medium,
    High,
}

impl GrantScope {
    pub fn tier(&self) -> Tier {
        match self {
            Self::Credential { .. } | Self::PrivateNetwork => Tier::High,
            Self::Notifications | Self::Storage { .. } | Self::DeviceState | Self::DeviceIcons => {
                Tier::Low
            }
            Self::Host { .. }
            | Self::Profile { .. }
            | Self::Variable
            | Self::OpenUrl
            | Self::Background { .. } => Tier::Medium,
        }
    }

    /// Whether `self` allows at least everything `other` allows.
    pub fn covers(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Host { methods: old }, Self::Host { methods: new }) => is_subset(new, old),
            (
                Self::Credential {
                    hosts: old_hosts,
                    schemes: old_schemes,
                },
                Self::Credential {
                    hosts: new_hosts,
                    schemes: new_schemes,
                },
            ) => {
                is_subset(new_schemes, old_schemes)
                    && new_hosts
                        .iter()
                        .all(|host| old_hosts.iter().any(|old| host_covers(old, host)))
            }
            (Self::Profile { read: old }, Self::Profile { read: new }) => is_subset(new, old),
            (Self::Storage { quota_kb: old }, Self::Storage { quota_kb: new }) => new <= old,
            (
                Self::Background {
                    min_interval_seconds: old,
                },
                Self::Background {
                    min_interval_seconds: new,
                },
            ) => new >= old,
            (a, b) => a == b,
        }
    }

    /// The broader of two declarations of the same key.
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Host { methods: a }, Self::Host { methods: b }) => Self::Host {
                methods: union(a, b),
            },
            (Self::Profile { read: a }, Self::Profile { read: b }) => {
                Self::Profile { read: union(a, b) }
            }
            (Self::Storage { quota_kb: a }, Self::Storage { quota_kb: b }) => {
                Self::Storage { quota_kb: a.max(b) }
            }
            (
                Self::Background {
                    min_interval_seconds: a,
                },
                Self::Background {
                    min_interval_seconds: b,
                },
            ) => Self::Background {
                min_interval_seconds: a.min(b),
            },
            (a, _) => a,
        }
    }
}

fn is_subset<T: PartialEq>(small: &[T], big: &[T]) -> bool {
    small.iter().all(|item| big.contains(item))
}

fn union<T: Ord>(a: Vec<T>, b: Vec<T>) -> Vec<T> {
    a.into_iter()
        .chain(b)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sorted<T: Ord + Clone>(items: &[T]) -> Vec<T> {
    items
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Whether the host pattern `pattern` (`api.example.com`,
/// `*.example.com`, optionally with `:port`) matches every host `host`
/// matches.
pub fn host_covers(pattern: &str, host: &str) -> bool {
    let (pattern_name, pattern_port) = split_port(pattern);
    let (host_name, host_port) = split_port(host);
    if pattern_port != host_port {
        return false;
    }
    if pattern_name == host_name {
        return true;
    }
    pattern_name
        .strip_prefix('*')
        .is_some_and(|suffix| suffix.starts_with('.') && host_name.ends_with(suffix))
}

fn split_port(host: &str) -> (&str, Option<&str>) {
    match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    }
}

/// Whether the `variables.read` entry `pattern` (an id or `prefix.*`)
/// reads every variable `entry` reads.
pub fn variable_covers(pattern: &str, entry: &str) -> bool {
    if pattern == entry {
        return true;
    }
    pattern
        .strip_suffix('*')
        .is_some_and(|prefix| prefix.ends_with('.') && entry.starts_with(prefix))
}

/// Whether a stored grant for `old` also covers the declared key `new`
/// (same kind; a wildcard host or variable prefix).
fn key_covers(old: &GrantKey, new: &GrantKey) -> bool {
    if old.kind != new.kind {
        return false;
    }
    match old.kind {
        PermissionKind::Network | PermissionKind::OpenUrl
            if old.key != PRIVATE_NETWORK_KEY && new.key != PRIVATE_NETWORK_KEY =>
        {
            host_covers(&old.key, &new.key)
        }
        PermissionKind::Variables => variable_covers(&old.key, &new.key),
        _ => old.key == new.key,
    }
}

/// The normalized grants a manifest declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantSet(BTreeMap<GrantKey, GrantScope>);

impl GrantSet {
    pub fn from_manifest(manifest: &Manifest) -> Self {
        let mut set = Self::default();
        for permission in &manifest.permissions {
            match permission {
                Permission::Network {
                    hosts,
                    methods,
                    private_network,
                    ..
                } => {
                    let methods = methods.as_deref().map_or(ALL_METHODS.to_vec(), sorted);
                    for host in hosts {
                        set.insert(
                            GrantKey::new(PermissionKind::Network, host.clone()),
                            GrantScope::Host {
                                methods: methods.clone(),
                            },
                        );
                    }
                    if *private_network == Some(true) {
                        set.insert(
                            GrantKey::new(PermissionKind::Network, PRIVATE_NETWORK_KEY),
                            GrantScope::PrivateNetwork,
                        );
                    }
                }
                Permission::Credentials { refs, .. } => {
                    for reference in refs {
                        let slot = manifest
                            .contributes
                            .credentials()
                            .iter()
                            .find(|slot| slot.key == *reference);
                        set.insert(
                            GrantKey::new(PermissionKind::Credentials, reference.clone()),
                            GrantScope::Credential {
                                hosts: slot.map(|s| sorted(&s.hosts)).unwrap_or_default(),
                                schemes: slot.map(|s| sorted(&s.schemes)).unwrap_or_default(),
                            },
                        );
                    }
                }
                Permission::Notifications { .. } => set.insert(
                    GrantKey::new(PermissionKind::Notifications, ""),
                    GrantScope::Notifications,
                ),
                Permission::Profile { read, .. } => set.insert(
                    GrantKey::new(PermissionKind::Profile, ""),
                    GrantScope::Profile { read: sorted(read) },
                ),
                Permission::Variables { read, .. } => {
                    for entry in read {
                        set.insert(
                            GrantKey::new(PermissionKind::Variables, entry.clone()),
                            GrantScope::Variable,
                        );
                    }
                }
                Permission::Storage { quota_kb, .. } => set.insert(
                    GrantKey::new(PermissionKind::Storage, ""),
                    GrantScope::Storage {
                        quota_kb: *quota_kb,
                    },
                ),
                Permission::OpenUrl { hosts, .. } => {
                    for host in hosts {
                        set.insert(
                            GrantKey::new(PermissionKind::OpenUrl, host.clone()),
                            GrantScope::OpenUrl,
                        );
                    }
                }
                Permission::Background {
                    min_interval_seconds,
                    ..
                } => set.insert(
                    GrantKey::new(PermissionKind::Background, ""),
                    GrantScope::Background {
                        min_interval_seconds: *min_interval_seconds,
                    },
                ),
                Permission::Device { state, icons, .. } => {
                    if *state == Some(true) {
                        set.insert(
                            GrantKey::new(PermissionKind::Device, "state"),
                            GrantScope::DeviceState,
                        );
                    }
                    if *icons == Some(true) {
                        set.insert(
                            GrantKey::new(PermissionKind::Device, "icons"),
                            GrantScope::DeviceIcons,
                        );
                    }
                }
            }
        }
        set
    }

    fn insert(&mut self, key: GrantKey, scope: GrantScope) {
        let merged = match self.0.remove(&key) {
            Some(existing) => existing.merge(scope),
            None => scope,
        };
        self.0.insert(key, merged);
    }

    pub fn get(&self, key: &GrantKey) -> Option<&GrantScope> {
        self.0.get(key)
    }

    pub fn contains(&self, key: &GrantKey) -> bool {
        self.0.contains_key(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &GrantKey> {
        self.0.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&GrantKey, &GrantScope)> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Whether a stored grant allows or refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantStatus {
    Granted,
    Revoked,
}

impl GrantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Revoked => "revoked",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "granted" => Some(Self::Granted),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

/// Where a grant decision was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantSource {
    /// The install dialog.
    Install,
    /// The approval of an update.
    Update,
    /// Settings › Plugins (a later grant or revocation).
    User,
    /// Copied from a broader stored grant that covers it (a wildcard host
    /// or variable prefix).
    Derived,
}

impl GrantSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::User => "user",
            Self::Derived => "derived",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "install" => Some(Self::Install),
            "update" => Some(Self::Update),
            "user" => Some(Self::User),
            "derived" => Some(Self::Derived),
            _ => None,
        }
    }
}

/// A row of `plugin_grants`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGrant {
    pub key: GrantKey,
    pub scope: GrantScope,
    pub status: GrantStatus,
    pub source: GrantSource,
    /// Who decided: the caller's label (`user`, `app:settings`…).
    pub decided_by: String,
    pub decided_at: i64,
    pub granted_for_version: String,
}

/// The user's consent: the grant keys they approved, and who they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consent {
    pub keys: BTreeSet<GrantKey>,
    pub decided_by: String,
}

impl Consent {
    /// No grant at all (the plugin installs but cannot be enabled until
    /// its permissions are granted).
    pub fn none(decided_by: impl Into<String>) -> Self {
        Self {
            keys: BTreeSet::new(),
            decided_by: decided_by.into(),
        }
    }

    /// Every grant of `set`.
    pub fn all(set: &GrantSet, decided_by: impl Into<String>) -> Self {
        Self {
            keys: set.keys().cloned().collect(),
            decided_by: decided_by.into(),
        }
    }

    pub fn of(keys: impl IntoIterator<Item = GrantKey>, decided_by: impl Into<String>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
            decided_by: decided_by.into(),
        }
    }
}

/// A declared grant with its tier, for the consent dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredGrant {
    pub key: GrantKey,
    pub scope: GrantScope,
    pub tier: Tier,
}

/// A grant whose scope grows (more methods, hosts, schemes, quota…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broadened {
    pub key: GrantKey,
    pub from: GrantScope,
    pub to: GrantScope,
    pub tier: Tier,
}

/// A candidate version's grants compared with the **stored consent**
/// (section 13.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionDiff {
    /// Declared, and no stored decision covers it: escalation.
    pub added: Vec<DeclaredGrant>,
    /// A granted key whose scope grows: escalation.
    pub broadened: Vec<Broadened>,
    /// Same key, the stored decision is kept with the new scope: a
    /// narrower scope, or a revoked grant (which stays revoked).
    pub narrowed: Vec<GrantKey>,
    /// Same key, same scope.
    pub unchanged: Vec<GrantKey>,
    /// Covered by a broader stored grant (the second key); its decision
    /// is copied.
    pub covered: Vec<(GrantKey, GrantKey)>,
    /// Stored but no longer declared: deleted when applied.
    pub removed: Vec<GrantKey>,
}

impl PermissionDiff {
    pub fn compute(stored: &[StoredGrant], candidate: &GrantSet) -> Self {
        let mut diff = Self::default();
        for (key, scope) in candidate.iter() {
            if let Some(row) = stored.iter().find(|row| row.key == *key) {
                if row.scope == *scope {
                    diff.unchanged.push(key.clone());
                } else if row.scope.covers(scope) || row.status == GrantStatus::Revoked {
                    diff.narrowed.push(key.clone());
                } else {
                    diff.broadened.push(Broadened {
                        key: key.clone(),
                        from: row.scope.clone(),
                        to: scope.clone(),
                        tier: scope.tier(),
                    });
                }
            } else if let Some(row) = stored
                .iter()
                .find(|row| key_covers(&row.key, key) && row.scope.covers(scope))
            {
                diff.covered.push((key.clone(), row.key.clone()));
            } else {
                diff.added.push(DeclaredGrant {
                    key: key.clone(),
                    scope: scope.clone(),
                    tier: scope.tier(),
                });
            }
        }
        diff.removed = stored
            .iter()
            .filter(|row| !candidate.contains(&row.key))
            .map(|row| row.key.clone())
            .collect();
        diff
    }

    pub fn is_escalation(&self) -> bool {
        !self.added.is_empty() || !self.broadened.is_empty()
    }

    /// The keys that need fresh consent.
    pub fn escalations(&self) -> Vec<GrantKey> {
        self.added
            .iter()
            .map(|g| g.key.clone())
            .chain(self.broadened.iter().map(|g| g.key.clone()))
            .collect()
    }

    /// The highest tier among the escalations.
    pub fn highest_tier(&self) -> Option<Tier> {
        self.added
            .iter()
            .map(|g| g.tier)
            .chain(self.broadened.iter().map(|g| g.tier))
            .max()
    }
}

/// The declared keys that are not granted with at least the declared scope:
/// what blocks enabling.
pub fn missing_grants(stored: &[StoredGrant], declared: &GrantSet) -> Vec<GrantKey> {
    declared
        .iter()
        .filter(|(key, scope)| {
            !stored.iter().any(|row| {
                row.key == **key && row.status == GrantStatus::Granted && row.scope.covers(scope)
            })
        })
        .map(|(key, _)| key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(methods: &[HttpMethod]) -> GrantScope {
        GrantScope::Host {
            methods: methods.to_vec(),
        }
    }

    fn row(key: &str, scope: GrantScope, status: GrantStatus) -> StoredGrant {
        StoredGrant {
            key: key.parse().unwrap(),
            scope,
            status,
            source: GrantSource::Install,
            decided_by: "user".into(),
            decided_at: 0,
            granted_for_version: "1.0.0".into(),
        }
    }

    fn set(items: &[(&str, GrantScope)]) -> GrantSet {
        let mut set = GrantSet::default();
        for (key, scope) in items {
            set.insert(key.parse().unwrap(), scope.clone());
        }
        set
    }

    #[test]
    fn grant_keys_round_trip_as_text() {
        for text in [
            "network:api.github.com",
            "network:private-network",
            "notifications",
            "credentials:token",
            "variables:git.*",
            "device:state",
        ] {
            let key: GrantKey = text.parse().unwrap();
            assert_eq!(key.to_string(), text);
            let json = serde_json::to_string(&key).unwrap();
            assert_eq!(serde_json::from_str::<GrantKey>(&json).unwrap(), key);
        }
        assert!("filesystem:/".parse::<GrantKey>().is_err());
    }

    #[test]
    fn host_patterns() {
        assert!(host_covers("api.github.com", "api.github.com"));
        assert!(host_covers("*.github.com", "api.github.com"));
        assert!(host_covers("*.github.com", "*.api.github.com"));
        assert!(!host_covers("*.github.com", "github.com"));
        assert!(!host_covers("api.github.com", "*.github.com"));
        assert!(!host_covers("*.github.com", "api.github.com:8443"));
        assert!(host_covers("*.github.com:8443", "api.github.com:8443"));
        assert!(!host_covers("*.github.com", "evilgithub.com"));
    }

    #[test]
    fn variable_patterns() {
        assert!(variable_covers("git.*", "git.branch"));
        assert!(variable_covers("git.*", "git.remote.*"));
        assert!(!variable_covers("git.branch", "git.*"));
        assert!(!variable_covers("git.*", "github.token"));
    }

    #[test]
    fn scope_coverage() {
        use HttpMethod::{Get, Post};
        assert!(host(&[Get, Post]).covers(&host(&[Get])));
        assert!(!host(&[Get]).covers(&host(&[Get, Post])));
        let storage = |q| GrantScope::Storage { quota_kb: q };
        assert!(storage(512).covers(&storage(256)));
        assert!(!storage(256).covers(&storage(512)));
        let bg = |s| GrantScope::Background {
            min_interval_seconds: s,
        };
        assert!(bg(60).covers(&bg(120)));
        assert!(!bg(120).covers(&bg(60)));
        let cred = |hosts: &[&str]| GrantScope::Credential {
            hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
            schemes: vec![CredentialScheme::Bearer],
        };
        assert!(cred(&["*.github.com"]).covers(&cred(&["api.github.com"])));
        assert!(
            !cred(&["api.github.com"]).covers(&cred(&["api.github.com", "uploads.github.com"]))
        );
        assert!(!GrantScope::Notifications.covers(&GrantScope::OpenUrl));
    }

    #[test]
    fn tiers() {
        assert_eq!(GrantScope::PrivateNetwork.tier(), Tier::High);
        assert_eq!(
            GrantScope::Credential {
                hosts: vec![],
                schemes: vec![]
            }
            .tier(),
            Tier::High
        );
        assert_eq!(GrantScope::Notifications.tier(), Tier::Low);
        assert_eq!(GrantScope::Variable.tier(), Tier::Medium);
    }

    fn diff_fixture() -> (Vec<StoredGrant>, GrantSet) {
        use HttpMethod::{Get, Post};
        let stored = vec![
            row(
                "network:api.github.com",
                host(&[Get, Post]),
                GrantStatus::Granted,
            ),
            row("network:*.example.com", host(&[Get]), GrantStatus::Granted),
            row(
                "storage",
                GrantScope::Storage { quota_kb: 512 },
                GrantStatus::Granted,
            ),
            row(
                "notifications",
                GrantScope::Notifications,
                GrantStatus::Granted,
            ),
            row(
                "background",
                GrantScope::Background {
                    min_interval_seconds: 60,
                },
                GrantStatus::Granted,
            ),
            row(
                "variables:git.*",
                GrantScope::Variable,
                GrantStatus::Revoked,
            ),
            row(
                "device:state",
                GrantScope::DeviceState,
                GrantStatus::Granted,
            ),
        ];
        let candidate = set(&[
            ("network:api.github.com", host(&[Get])),     // narrowed
            ("network:api.example.com", host(&[Get])),    // covered
            ("network:uploads.github.com", host(&[Get])), // added
            ("storage", GrantScope::Storage { quota_kb: 1024 }), // broadened
            ("notifications", GrantScope::Notifications), // unchanged
            (
                "background",
                GrantScope::Background {
                    min_interval_seconds: 30,
                },
            ), // broadened
            ("variables:git.branch", GrantScope::Variable), // covered (by a revoked grant)
            (
                "credentials:token",
                GrantScope::Credential {
                    hosts: vec!["api.github.com".into()],
                    schemes: vec![CredentialScheme::Bearer],
                },
            ), // added, high
        ]);
        (stored, candidate)
    }

    #[test]
    fn diff_classifies_every_change() {
        let (stored, candidate) = diff_fixture();
        let diff = PermissionDiff::compute(&stored, &candidate);
        assert_eq!(
            diff.narrowed,
            vec!["network:api.github.com".parse().unwrap()]
        );
        assert_eq!(
            diff.covered,
            vec![
                (
                    "network:api.example.com".parse().unwrap(),
                    "network:*.example.com".parse().unwrap()
                ),
                (
                    "variables:git.branch".parse().unwrap(),
                    "variables:git.*".parse().unwrap()
                ),
            ]
        );
        assert_eq!(diff.unchanged, vec!["notifications".parse().unwrap()]);
        let mut escalations = diff.escalations();
        escalations.sort();
        let mut expected: Vec<GrantKey> = [
            "network:uploads.github.com",
            "credentials:token",
            "storage",
            "background",
        ]
        .iter()
        .map(|k| k.parse().unwrap())
        .collect();
        expected.sort();
        assert_eq!(escalations, expected);
        assert!(diff.is_escalation());
        assert_eq!(diff.highest_tier(), Some(Tier::High));
        let mut removed = diff.removed.clone();
        removed.sort();
        assert_eq!(
            removed,
            vec![
                "network:*.example.com".parse().unwrap(),
                "variables:git.*".parse().unwrap(),
                "device:state".parse().unwrap(),
            ]
            .into_iter()
            .collect::<BTreeSet<GrantKey>>()
            .into_iter()
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn narrowing_only_is_not_an_escalation() {
        use HttpMethod::{Get, Post};
        let stored = vec![
            row(
                "network:api.github.com",
                host(&[Get, Post]),
                GrantStatus::Granted,
            ),
            row(
                "notifications",
                GrantScope::Notifications,
                GrantStatus::Granted,
            ),
        ];
        let candidate = set(&[("network:api.github.com", host(&[Get]))]);
        let diff = PermissionDiff::compute(&stored, &candidate);
        assert!(!diff.is_escalation());
        assert_eq!(diff.removed, vec!["notifications".parse().unwrap()]);
        assert_eq!(diff.highest_tier(), None);
    }

    #[test]
    fn broadening_a_revoked_grant_keeps_it_revoked_without_asking() {
        let stored = vec![row(
            "storage",
            GrantScope::Storage { quota_kb: 300 },
            GrantStatus::Revoked,
        )];
        let candidate = set(&[("storage", GrantScope::Storage { quota_kb: 900 })]);
        let diff = PermissionDiff::compute(&stored, &candidate);
        assert!(!diff.is_escalation());
        assert_eq!(diff.narrowed.len(), 1);
    }

    #[test]
    fn missing_requires_granted_rows_with_enough_scope() {
        use HttpMethod::{Get, Post};
        let declared = set(&[
            ("network:api.github.com", host(&[Get, Post])),
            ("notifications", GrantScope::Notifications),
            ("device:state", GrantScope::DeviceState),
        ]);
        let stored = vec![
            row("network:api.github.com", host(&[Get]), GrantStatus::Granted),
            row(
                "notifications",
                GrantScope::Notifications,
                GrantStatus::Revoked,
            ),
            row(
                "device:state",
                GrantScope::DeviceState,
                GrantStatus::Granted,
            ),
        ];
        let missing = missing_grants(&stored, &declared);
        assert_eq!(
            missing,
            vec![
                "network:api.github.com".parse().unwrap(),
                "notifications".parse().unwrap()
            ]
        );
    }

    #[test]
    fn duplicate_declarations_merge_to_the_broader_scope() {
        let mut set = GrantSet::default();
        let key: GrantKey = "network:api.github.com".parse().unwrap();
        set.insert(key.clone(), host(&[HttpMethod::Get]));
        set.insert(key.clone(), host(&[HttpMethod::Post]));
        assert_eq!(
            set.get(&key),
            Some(&host(&[HttpMethod::Get, HttpMethod::Post]))
        );
    }
}
