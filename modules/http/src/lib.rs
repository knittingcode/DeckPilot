//! The `http` feature module: HTTP requests and health checks.
//!
//! **Network boundary.** Requests go to `http` and `https` addresses on the
//! public internet. Addresses on this computer and private networks
//! (loopback, RFC 1918, link-local including cloud metadata services,
//! carrier NAT, unique local IPv6, …) are refused unless allowed on this
//! computer — for all hosts (`allowPrivateNetworks`) or for listed hosts
//! (`privateHosts`). Both settings are machine-specific: an imported
//! workflow cannot open them. Every address a name resolves to is checked,
//! the connection uses exactly the checked addresses, and each redirect is
//! checked again, so neither DNS nor a redirect can lead inside.
//!
//! **Credentials** are stored on each computer and named by an id; a
//! workflow never contains a secret, and headers or query parameters that
//! look like secrets cannot be written into one. Credentials are sent only
//! over https (or to this computer), never along a redirect to another
//! origin, and never appear in results, history or messages. Responses
//! show secret headers (cookies, tokens) as `[redacted]`.
//!
//! **TLS** uses the operating system's certificate store; certificate
//! verification cannot be turned off. No proxy is used.

pub mod actions;
pub mod client;
pub mod policy;

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionHandler, ActionParameters, Capability, FeatureModule, ModuleId, ModuleManifest,
    ParameterKind, ParameterSpec, ParameterValue, Portability, SettingScope,
};

pub const MODULE_ID: &str = "http";
/// Module setting: requests may reach this computer and private networks.
pub const ALLOW_PRIVATE: &str = "allowPrivateNetworks";
/// Module setting: hosts that may be on private networks, one per line.
pub const PRIVATE_HOSTS: &str = "privateHosts";
/// Module setting: the most response bytes kept, in KiB.
pub const MAX_RESPONSE_KB: &str = "maxResponseKb";

/// Stable error codes of the module's actions.
pub mod codes {
    pub const INVALID_URL: &str = "invalid-url";
    pub const CREDENTIALS_IN_URL: &str = "credentials-in-url";
    pub const INVALID_HEADER: &str = "invalid-header";
    pub const INVALID_QUERY: &str = "invalid-query";
    /// A header or query parameter that holds a secret was written into
    /// the workflow.
    pub const SENSITIVE_HEADER: &str = "sensitive-header";
    pub const INVALID_BODY: &str = "invalid-body";
    /// The address is on this computer or a private network.
    pub const BLOCKED_ADDRESS: &str = "blocked-address";
    pub const DNS_FAILED: &str = "dns-failed";
    pub const CONNECTION_FAILED: &str = "connection-failed";
    /// The certificate or the TLS handshake was refused.
    pub const TLS_FAILED: &str = "tls-failed";
    pub const TOO_MANY_REDIRECTS: &str = "too-many-redirects";
    /// A redirect from https to http.
    pub const INSECURE_REDIRECT: &str = "insecure-redirect";
    /// A credential would be sent over plain http.
    pub const INSECURE_CREDENTIALS: &str = "insecure-credentials";
    pub const REQUEST_FAILED: &str = "request-failed";
    /// The response has a 4xx or 5xx status.
    pub const HTTP_STATUS: &str = "http-status";
    /// The response is not the JSON the action was asked to read.
    pub const INVALID_RESPONSE: &str = "invalid-response";
    /// The health check's expectation was not met.
    pub const UNHEALTHY: &str = "unhealthy";
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

/// The module's settings.
#[derive(Debug, Clone)]
pub struct HttpSettings {
    pub network: policy::NetworkPolicy,
    pub max_bytes: usize,
}

pub fn settings(config: &ActionParameters) -> HttpSettings {
    HttpSettings {
        network: policy::NetworkPolicy {
            allow_private: config.boolean(ALLOW_PRIVATE).unwrap_or(false),
            private_hosts: config
                .text(PRIVATE_HOSTS)
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(str::to_owned)
                .collect(),
        },
        max_bytes: usize::try_from(
            config
                .integer(MAX_RESPONSE_KB)
                .unwrap_or(1024)
                .clamp(1, 10_240),
        )
        .unwrap_or(1024)
            * 1024,
    }
}

#[derive(Debug)]
pub struct HttpModule {
    manifest: ModuleManifest,
}

impl HttpModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "HTTP".into(),
                description: "HTTP requests and health checks, with credentials stored on this \
                              computer."
                    .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![Capability::Network, Capability::Credentials],
                config: vec![
                    ParameterSpec {
                        key: ALLOW_PRIVATE.into(),
                        label: "Allow this computer and private networks".into(),
                        description: Some(
                            "Lets requests reach localhost, 10.x, 192.168.x and other private \
                             addresses. Off by default; only for this computer."
                                .into(),
                        ),
                        kind: ParameterKind::Boolean,
                        required: true,
                        default: Some(ParameterValue::Boolean(false)),
                    },
                    ParameterSpec {
                        key: PRIVATE_HOSTS.into(),
                        label: "Private hosts allowed".into(),
                        description: Some(
                            "Hosts that may be on this computer or a private network, one per \
                             line, e.g. localhost or *.internal. Only for this computer."
                                .into(),
                        ),
                        kind: ParameterKind::Text {
                            multiline: true,
                            max_length: Some(4000),
                        },
                        required: false,
                        default: None,
                    },
                    ParameterSpec {
                        key: MAX_RESPONSE_KB.into(),
                        label: "Largest response kept (KiB)".into(),
                        description: Some("Longer responses are cut and marked truncated.".into()),
                        kind: ParameterKind::Integer {
                            min: Some(1),
                            max: Some(10_240),
                        },
                        required: true,
                        default: Some(ParameterValue::Integer(1024)),
                    },
                ],
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::from([
                        (ALLOW_PRIVATE.to_owned(), SettingScope::MachineSpecific),
                        (PRIVATE_HOSTS.to_owned(), SettingScope::MachineSpecific),
                        (MAX_RESPONSE_KB.to_owned(), SettingScope::Portable),
                    ]),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
            },
        }
    }
}

impl Default for HttpModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for HttpModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        actions::handlers(&std::sync::Arc::new(settings(config)))
    }
}
