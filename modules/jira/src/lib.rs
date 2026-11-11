//! The `jira` feature module: reads the signed-in user, issues and lists
//! of issues (assigned to you, or from a saved filter) from Jira Cloud or
//! Jira Data Center, and opens Jira pages in the browser.
//!
//! **Read-only.** Only `GET` requests are sent; there are no actions that
//! create, transition, assign, comment on or change anything in Jira.
//! Searches are built from fixed templates (assigned to you, or a saved
//! filter by its number) with validated project keys; no free-form JQL is
//! accepted.
//!
//! **Token.** The secret is a credential stored on this computer and named
//! by id (`jira.token` by default): `email:api-token` for Jira Cloud (sent
//! as Basic authentication) or a personal access token for Data Center
//! (sent as a Bearer token). It reaches the module only through
//! [`ExecutionContext::credential`](deckpilot_core::ExecutionContext::credential),
//! never appears in parameters, settings, outputs, messages or logs, and
//! every result is checked for it before it is returned.
//!
//! **Network.** Requests go through the HTTP module's client and its
//! network policy, as for the GitHub and GitLab modules.

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

pub const MODULE_ID: &str = "jira";
/// Module setting: the Jira site's address.
pub const SITE_URL: &str = "siteUrl";
/// Module setting: `cloud` or `data-center`.
pub const DEPLOYMENT: &str = "deployment";
/// Module setting: the stored credential.
pub const CREDENTIAL: &str = "credential";
/// Module setting: time limit of one action, in seconds.
pub const TIMEOUT: &str = "timeoutSeconds";
/// Module setting: the site may be on this computer or a private network.
pub const ALLOW_PRIVATE: &str = "allowPrivateHost";

/// The credential used when none is configured.
pub const DEFAULT_CREDENTIAL: &str = "jira.token";
pub const DEPLOYMENTS: [&str; 2] = ["cloud", "data-center"];

/// Stable error codes of the module's actions. Network failures use the
/// HTTP module's codes (`blocked-address`, `dns-failed`,
/// `connection-failed`, `tls-failed`, `insecure-credentials`, …).
pub mod codes {
    /// The site URL is missing or not a valid https address.
    pub const INVALID_SITE_URL: &str = "invalid-site-url";
    /// The issue key is not like `ABC-123`.
    pub const INVALID_ISSUE_KEY: &str = "invalid-issue-key";
    /// The project key is not like `ABC`.
    pub const INVALID_PROJECT: &str = "invalid-project";
    /// Jira refused the credential (401).
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// The account may not read this (403).
    pub const FORBIDDEN: &str = "forbidden";
    /// Not found, or not visible to this account (404).
    pub const NOT_FOUND: &str = "not-found";
    /// The rate limit is reached (429); the data has `retryAfterSeconds`.
    pub const RATE_LIMITED: &str = "rate-limited";
    /// Another error status from Jira, e.g. a filter that does not exist.
    pub const JIRA_ERROR: &str = "jira-error";
    /// The response is not the JSON expected, or too large.
    pub const INVALID_RESPONSE: &str = "invalid-response";
    /// The page to open is not an https page on the Jira site.
    pub const URL_NOT_ALLOWED: &str = "url-not-allowed";
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

/// Which Jira the site runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployment {
    /// Jira Cloud: REST API 3, `email:api-token` as Basic authentication,
    /// token-paged search.
    Cloud,
    /// Jira Data Center or Server: REST API 2, a personal access token as
    /// Bearer, offset-paged search with totals.
    DataCenter,
}

/// The module's settings.
#[derive(Debug, Clone)]
pub struct JiraSettings {
    /// The site URL as configured; checked when an action runs.
    pub site_url: String,
    pub deployment: Deployment,
    /// The default credential's name.
    pub credential: String,
    pub timeout: Duration,
    pub allow_private: bool,
}

pub fn settings(config: &ActionParameters) -> JiraSettings {
    let text = |key: &str, default: &str| {
        config
            .text(key)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(default)
            .to_owned()
    };
    JiraSettings {
        site_url: text(SITE_URL, ""),
        deployment: if config.text(DEPLOYMENT) == Some("data-center") {
            Deployment::DataCenter
        } else {
            Deployment::Cloud
        },
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
pub struct JiraModule {
    manifest: ModuleManifest,
    launcher: Arc<dyn Launcher>,
}

impl JiraModule {
    pub fn new() -> Self {
        Self::with_launcher(Arc::new(SystemLauncher))
    }

    /// The module with another launcher for `jira.open`.
    #[allow(clippy::too_many_lines)] // The manifest with its settings.
    pub fn with_launcher(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "Jira".into(),
                description: "Issues assigned to you, saved filters and single issues from Jira \
                              Cloud or Data Center. Read-only."
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
                        key: SITE_URL.into(),
                        label: "Site".into(),
                        description: Some(
                            "e.g. https://your-team.atlassian.net, or your Data Center address. \
                             Only for this computer."
                                .into(),
                        ),
                        kind: ParameterKind::Text {
                            multiline: false,
                            max_length: Some(512),
                        },
                        required: false,
                        default: None,
                    },
                    ParameterSpec {
                        key: DEPLOYMENT.into(),
                        label: "Jira".into(),
                        description: Some(
                            "cloud: email:api-token as the credential; data-center: a personal \
                             access token."
                                .into(),
                        ),
                        kind: ParameterKind::Choice {
                            options: DEPLOYMENTS.iter().map(|d| (*d).to_owned()).collect(),
                        },
                        required: true,
                        default: Some(ParameterValue::Choice("cloud".into())),
                    },
                    ParameterSpec {
                        key: CREDENTIAL.into(),
                        label: "Credential".into(),
                        description: Some(
                            "The name of a credential stored on this computer (Settings › HTTP \
                             › Credentials). Never the secret itself."
                                .into(),
                        ),
                        kind: ParameterKind::Credential,
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
                        label: "Site may be on a private network".into(),
                        description: Some(
                            "For Jira Data Center inside a company network: lets requests to the \
                             site's host reach private addresses. Off by default; only for this \
                             computer."
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
                        (SITE_URL.to_owned(), SettingScope::MachineSpecific),
                        (DEPLOYMENT.to_owned(), SettingScope::MachineSpecific),
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

impl Default for JiraModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for JiraModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        actions::handlers(&Arc::new(settings(config)), &self.launcher)
    }
}
