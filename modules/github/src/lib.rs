//! The `github` feature module: reads the signed-in user, repositories,
//! open pull requests, review requests and the checks of a commit from
//! GitHub or GitHub Enterprise, and opens GitHub pages in the browser.
//!
//! **Read-only.** Only `GET` requests are sent; there are no actions that
//! comment, approve, merge or change anything on GitHub.
//!
//! **Token.** The token is a credential stored on this computer and named
//! by id (`github.token` by default); it reaches the module only through
//! [`ExecutionContext::credential`](deckpilot_core::ExecutionContext::credential)
//! and is sent as `Authorization: Bearer` to the configured API host. It
//! is never part of parameters, settings, outputs, messages or logs, and
//! every result is checked for it before it is returned.
//!
//! **Network.** Requests go through the HTTP module's client and its
//! network policy: addresses are checked before connecting, redirects are
//! checked hop by hop, the token is never sent to another origin and
//! never over plain http (except to this computer). The API host may be on
//! a private network only when that is allowed on this computer.

pub mod actions;
pub mod api;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use deckpilot_core::launch::{Launcher, SystemLauncher};
use deckpilot_core::{
    ActionHandler, ActionParameters, Capability, FeatureModule, ModuleId, ModuleManifest,
    ParameterKind, ParameterSpec, ParameterValue, Portability, SettingScope,
};

pub const MODULE_ID: &str = "github";
/// Module setting: the REST API's base URL.
pub const API_URL: &str = "apiUrl";
/// Module setting: the stored credential holding the token.
pub const CREDENTIAL: &str = "credential";
/// Module setting: time limit of one action, in seconds.
pub const TIMEOUT: &str = "timeoutSeconds";
/// Module setting: the API host may be on this computer or a private
/// network (GitHub Enterprise Server inside a company network).
pub const ALLOW_PRIVATE: &str = "allowPrivateHost";

/// The public GitHub API.
pub const DEFAULT_API_URL: &str = "https://api.github.com";
/// The credential used when none is configured.
pub const DEFAULT_CREDENTIAL: &str = "github.token";

/// Stable error codes of the module's actions. Network failures use the
/// HTTP module's codes (`blocked-address`, `dns-failed`,
/// `connection-failed`, `tls-failed`, `insecure-credentials`, …).
pub mod codes {
    /// The configured API URL is not a valid https address.
    pub const INVALID_API_URL: &str = "invalid-api-url";
    /// The repository is not `owner/name`.
    pub const INVALID_REPOSITORY: &str = "invalid-repository";
    /// The ref is not a branch, tag or commit name.
    pub const INVALID_REF: &str = "invalid-ref";
    /// GitHub refused the token (401).
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// The token may not read this (403), e.g. a missing scope or SSO.
    pub const FORBIDDEN: &str = "forbidden";
    /// Not found, or not visible with this token (404).
    pub const NOT_FOUND: &str = "not-found";
    /// The rate limit is reached; the data has `retryAfterSeconds`.
    pub const RATE_LIMITED: &str = "rate-limited";
    /// Another error status from GitHub.
    pub const GITHUB_ERROR: &str = "github-error";
    /// The response is not the JSON expected, or too large.
    pub const INVALID_RESPONSE: &str = "invalid-response";
    /// The page to open is not an https page on the GitHub host.
    pub const URL_NOT_ALLOWED: &str = "url-not-allowed";
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

/// The module's settings.
#[derive(Debug, Clone)]
pub struct GithubSettings {
    /// The API base URL as configured; checked when an action runs.
    pub api_url: String,
    /// The default credential's name.
    pub credential: String,
    pub timeout: Duration,
    pub allow_private: bool,
}

pub fn settings(config: &ActionParameters) -> GithubSettings {
    let text = |key: &str, default: &str| {
        config
            .text(key)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(default)
            .to_owned()
    };
    GithubSettings {
        api_url: text(API_URL, DEFAULT_API_URL),
        credential: text(CREDENTIAL, DEFAULT_CREDENTIAL),
        timeout: Duration::from_secs(
            config
                .integer(TIMEOUT)
                .unwrap_or(20)
                .clamp(1, 120)
                .unsigned_abs(),
        ),
        allow_private: config.boolean(ALLOW_PRIVATE).unwrap_or(false),
    }
}

#[derive(Debug)]
pub struct GithubModule {
    manifest: ModuleManifest,
    launcher: Arc<dyn Launcher>,
}

impl GithubModule {
    pub fn new() -> Self {
        Self::with_launcher(Arc::new(SystemLauncher))
    }

    /// The module with another launcher for `github.open`.
    #[allow(clippy::too_many_lines)] // The manifest with its settings.
    pub fn with_launcher(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "GitHub".into(),
                description: "Pull requests, review requests, checks and repositories from \
                              GitHub or GitHub Enterprise. Read-only."
                    .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![
                    Capability::Network,
                    Capability::Credentials,
                    Capability::OpenExternal,
                ],
                config: vec![
                    ParameterSpec {
                        key: API_URL.into(),
                        label: "API URL".into(),
                        description: Some(
                            "https://api.github.com, or https://<host>/api/v3 for GitHub \
                             Enterprise Server. Only for this computer."
                                .into(),
                        ),
                        kind: ParameterKind::Text {
                            multiline: false,
                            max_length: Some(512),
                        },
                        required: true,
                        default: Some(ParameterValue::Text(DEFAULT_API_URL.into())),
                    },
                    ParameterSpec {
                        key: CREDENTIAL.into(),
                        label: "Token credential".into(),
                        description: Some(
                            "The name of a credential stored on this computer (Settings › HTTP \
                             › Credentials) that holds a GitHub token. Never the token itself."
                                .into(),
                        ),
                        kind: ParameterKind::Text {
                            multiline: false,
                            max_length: Some(64),
                        },
                        required: true,
                        default: Some(ParameterValue::Text(DEFAULT_CREDENTIAL.into())),
                    },
                    ParameterSpec {
                        key: TIMEOUT.into(),
                        label: "Time limit (s)".into(),
                        description: Some("For all requests of one action.".into()),
                        kind: ParameterKind::Integer {
                            min: Some(1),
                            max: Some(120),
                        },
                        required: true,
                        default: Some(ParameterValue::Integer(20)),
                    },
                    ParameterSpec {
                        key: ALLOW_PRIVATE.into(),
                        label: "API host may be on a private network".into(),
                        description: Some(
                            "For GitHub Enterprise Server inside a company network: lets \
                             requests to the API host reach private addresses. Off by default; \
                             only for this computer."
                                .into(),
                        ),
                        kind: ParameterKind::Boolean,
                        required: true,
                        default: Some(ParameterValue::Boolean(false)),
                    },
                ],
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::from([
                        (API_URL.to_owned(), SettingScope::MachineSpecific),
                        (CREDENTIAL.to_owned(), SettingScope::Portable),
                        (TIMEOUT.to_owned(), SettingScope::Portable),
                        (ALLOW_PRIVATE.to_owned(), SettingScope::MachineSpecific),
                    ]),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
            },
            launcher,
        }
    }
}

impl Default for GithubModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for GithubModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        actions::handlers(&Arc::new(settings(config)), &self.launcher)
    }
}
