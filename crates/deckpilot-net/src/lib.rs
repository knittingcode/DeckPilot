//! DeckPilot's HTTP transport, shared by the `http`, `github`, `gitlab` and
//! `jira` modules and by the agent's plugin broker executor.
//!
//! It lives in the core, not in a module crate, so that the plugin
//! broker's transport never depends on a built-in module
//! (`docs/marketplace/module-migration.md`, §3).
//!
//! **Network boundary.** [`client::send`] goes to `http` and `https`
//! addresses on the public internet. Addresses on this computer and private
//! networks (loopback, RFC 1918, link-local including cloud metadata
//! services, carrier NAT, unique local IPv6, …) are refused unless the
//! [`policy::NetworkPolicy`] allows them, for all hosts or for listed hosts.
//! Every address a name resolves to is checked, the connection uses exactly
//! the checked addresses, and each redirect is checked again, so neither
//! DNS nor a redirect can lead inside.
//!
//! **Credentials** ([`client::Auth`]) are sent only over https (or to this
//! computer), never along a redirect to another origin.
//!
//! **TLS** uses the operating system's certificate store; certificate
//! verification cannot be turned off. No proxy is used.

pub mod client;
pub mod policy;

/// Stable error codes of the transport. The `http` module re-exports them
/// as part of its own `codes`, so their values are part of its contract.
pub mod codes {
    pub const INVALID_URL: &str = "invalid-url";
    pub const CREDENTIALS_IN_URL: &str = "credentials-in-url";
    pub const INVALID_HEADER: &str = "invalid-header";
    pub const INVALID_QUERY: &str = "invalid-query";
    /// A header or query parameter that holds a secret was written into
    /// the workflow.
    pub const SENSITIVE_HEADER: &str = "sensitive-header";
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
}
