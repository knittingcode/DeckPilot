//! Talking to the Jira REST API through the agent's broker: which URLs
//! are used, how the credential is named, how answers and failures are
//! read, and the action's time limit.
//!
//! A port of the former built-in `jira` module's API code (removed in
//! WP-C5), with the
//! differences a sandboxed plugin has (`plugins/first-party/jira/README.md`):
//!
//! * the site is the server the user entered in `siteUrl` and allowed (the
//!   manifest's one user-origin slot; Jira has no fixed host); pages are
//!   opened on the same site;
//! * the credential is the slot [`CREDENTIAL_SLOT`], injected by the agent
//!   as Basic (Jira Cloud, `email:api-token`) or Bearer (Data Center) by
//!   the `deployment` setting; it never enters this module, so its form
//!   cannot be checked here;
//! * there is no clock in a guest: the time limit and rate-limit waits are
//!   counted with the `Date` headers of Jira's answers.

use deckpilot_first_party_support::{Deadline, MAX_ANSWER_KIB, Server, retry_after, too_large};
use deckpilot_plugin_sdk::prelude::*;
use serde_json::Value;

pub use deckpilot_first_party_support::clean;

/// The credential slot (`contributes.credentials[0].key`).
pub const CREDENTIAL_SLOT: &str = "credential";
/// Settings (`contributes.settings.fields`).
pub const SITE_URL: &str = "siteUrl";
pub const DEPLOYMENT: &str = "deployment";
pub const TIMEOUT: &str = "timeoutSeconds";
/// Most pages read to count a search.
pub const MAX_PAGES: usize = 5;
/// Issues asked for per page when counting (their keys only).
pub const PER_PAGE: usize = 100;

/// Stable error codes, the former built-in `jira` module's (WP-C5);
/// each is declared in `manifest.json` `contributes.errorCodes`.
pub mod codes {
    pub const INVALID_SITE_URL: &str = "invalid-site-url";
    pub const INVALID_ISSUE_KEY: &str = "invalid-issue-key";
    pub const INVALID_PROJECT: &str = "invalid-project";
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const FORBIDDEN: &str = "forbidden";
    pub const NOT_FOUND: &str = "not-found";
    pub const RATE_LIMITED: &str = "rate-limited";
    pub const JIRA_ERROR: &str = "jira-error";
    pub const INVALID_RESPONSE: &str = "invalid-response";
    pub const URL_NOT_ALLOWED: &str = "url-not-allowed";
    pub const TIMED_OUT: &str = "timed-out";
    /// Every code above.
    pub const ALL: &[&str] = &[
        INVALID_SITE_URL,
        INVALID_ISSUE_KEY,
        INVALID_PROJECT,
        UNAUTHORIZED,
        FORBIDDEN,
        NOT_FOUND,
        RATE_LIMITED,
        JIRA_ERROR,
        INVALID_RESPONSE,
        URL_NOT_ALLOWED,
        TIMED_OUT,
    ];
}

/// A failed action.
pub type Failure = PluginError;

pub fn fail(code: &str, message: impl Into<String>) -> Failure {
    PluginError::new(code, message)
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

/// An issue key such as `ABC-123` (upper-cased).
pub fn issue_key(text: &str) -> Result<String, Failure> {
    let key = text.trim().to_ascii_uppercase();
    let valid = key.split_once('-').is_some_and(|(project, number)| {
        project_key_valid(project)
            && (1..=18).contains(&number.len())
            && number.chars().all(|c| c.is_ascii_digit())
    });
    if valid {
        Ok(key)
    } else {
        Err(fail(
            codes::INVALID_ISSUE_KEY,
            format!("“{}” is not an issue key such as ABC-123", clean(text, 40)),
        ))
    }
}

fn project_key_valid(key: &str) -> bool {
    (1..=255).contains(&key.len())
        && key.starts_with(|c: char| c.is_ascii_uppercase())
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// A project key such as `ABC` (upper-cased). Only these characters can
/// appear, so it is safe inside a search.
pub fn project_key(text: &str) -> Result<String, Failure> {
    let key = text.trim().to_ascii_uppercase();
    if project_key_valid(&key) {
        Ok(key)
    } else {
        Err(fail(
            codes::INVALID_PROJECT,
            format!("“{}” is not a project key such as ABC", clean(text, 40)),
        ))
    }
}

/// The site's address from the `siteUrl` setting: an https address
/// without a user name, query or fragment. The agent normalizes the
/// setting when it is saved and allows requests only to the server the
/// user allowed; a plain `http` address on this computer, which the
/// built-in accepts, is not reachable for a plugin.
pub fn site(text: &str) -> Result<Server, Failure> {
    let invalid = |reason: &str| {
        fail(
            codes::INVALID_SITE_URL,
            format!("the Jira site {reason}; set it in Settings › Plugins › Jira"),
        )
    };
    if text.trim().is_empty() {
        return Err(invalid("is not set"));
    }
    Server::parse(text).ok_or_else(|| {
        invalid(
            "must be an https address such as https://your-team.atlassian.net, without a user \
             name, query or fragment",
        )
    })
}

/// A page to open: https, on the Jira site, without credentials.
pub fn web_url(
    text: &str,
    site: &Server,
) -> Result<deckpilot_first_party_support::WebPage, Failure> {
    site.page(text).ok_or_else(|| {
        fail(
            codes::URL_NOT_ALLOWED,
            format!("only https pages on {} can be opened", site.host()),
        )
    })
}

/// How long to wait when Jira answers 429: `Retry-After` (seconds, or an
/// HTTP date counted from the answer's `Date`), else a minute.
pub fn rate_limit_wait(status: u16, response: &Response, now: Option<u64>) -> Option<u64> {
    (status == 429).then(|| {
        response
            .header("retry-after")
            .and_then(|v| retry_after(v, now))
            .unwrap_or(60)
    })
}

/// Jira's own explanation of a failure: the first of `errorMessages`,
/// else the first of `errors`, else `message`.
fn explanation(response: &Response) -> Option<String> {
    let json: Value = response.json().ok()?;
    json.get("errorMessages")
        .and_then(Value::as_array)
        .and_then(|m| m.iter().find_map(Value::as_str))
        .or_else(|| {
            json.get("errors")
                .and_then(Value::as_object)
                .and_then(|e| e.values().find_map(Value::as_str))
        })
        .or_else(|| json.get("message").and_then(Value::as_str))
        .map(|m| clean(m, 200))
        .filter(|m| !m.is_empty())
}

/// The settings, read for every action (ABI 1 has no change event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraSettings {
    pub site_url: String,
    pub deployment: Deployment,
    pub timeout: u64,
}

pub fn settings(ctx: &mut Context<'_>) -> Result<JiraSettings, Failure> {
    let settings = ctx.settings()?;
    Ok(JiraSettings {
        site_url: settings
            .text(SITE_URL)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned(),
        deployment: if settings.text(DEPLOYMENT) == Some("data-center") {
            Deployment::DataCenter
        } else {
            Deployment::Cloud
        },
        timeout: settings
            .integer(TIMEOUT)
            .unwrap_or(20)
            .clamp(1, 120)
            .unsigned_abs(),
    })
}

/// A connection to the API for one action: the site, the credential's
/// slot, and the action's time limit counted with Jira's `Date`.
pub struct Api<'c, 'h> {
    ctx: &'c mut Context<'h>,
    credential: SecretHandle,
    site: Server,
    deployment: Deployment,
    deadline: Deadline,
}

impl std::fmt::Debug for Api<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("site", &self.site)
            .field("deployment", &self.deployment)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl<'c, 'h> Api<'c, 'h> {
    /// Checks the settings and that a credential is stored.
    pub fn connect(ctx: &'c mut Context<'h>) -> Result<Self, Failure> {
        let settings = settings(ctx)?;
        let site = site(&settings.site_url)?;
        let credential = SecretHandle::slot(CREDENTIAL_SLOT)?;
        // The slot has no host of its own: the agent grants it only for
        // the site the user allowed, so asking for it is refused until
        // then (and again once the site changed or the consent was
        // revoked).
        let stored = ctx.has_credential(&credential).map_err(|e| {
            if e.kind() == CapabilityErrorKind::NotPermitted {
                fail(
                    deckpilot_plugin_sdk::codes::NOT_PERMITTED,
                    format!(
                        "the Jira site {} is not allowed for this plugin; allow it in Settings \
                         › Plugins › Jira › Configure",
                        site.host()
                    ),
                )
            } else {
                PluginError::from(e)
            }
        })?;
        if !stored {
            return Err(fail(
                deckpilot_plugin_sdk::codes::CREDENTIAL_MISSING,
                match settings.deployment {
                    Deployment::Cloud => {
                        "no Jira credential is stored for this plugin; add email:api-token in \
                         Settings › Plugins › Jira"
                    }
                    Deployment::DataCenter => {
                        "no Jira credential is stored for this plugin; add a personal access \
                         token in Settings › Plugins › Jira"
                    }
                },
            ));
        }
        Ok(Self {
            ctx,
            credential,
            site,
            deployment: settings.deployment,
            deadline: Deadline::new(settings.timeout),
        })
    }

    pub fn deployment(&self) -> Deployment {
        self.deployment
    }

    /// A REST API URL: `rest/api/3` (Cloud) or `rest/api/2` (Data Center),
    /// these path segments (each encoded) and query.
    pub fn endpoint(&self, segments: &[&str], query: &[(&str, &str)]) -> String {
        let version = match self.deployment {
            Deployment::Cloud => "3",
            Deployment::DataCenter => "2",
        };
        let mut path = vec!["rest", "api", version];
        path.extend(segments);
        self.site.endpoint(&path, query)
    }

    /// The page of an issue: `<site>/browse/KEY`.
    pub fn browse(&self, key: &str) -> String {
        format!("{}browse/{key}", self.site.root())
    }

    /// Sends `GET url` and reads a JSON answer. `what` names the thing
    /// asked for in messages, e.g. “the issue ABC-1”.
    pub fn get(&mut self, url: String, what: &str) -> Result<Value, Failure> {
        if self.deadline.passed() {
            return Err(fail(
                codes::TIMED_OUT,
                "Jira did not answer within the time limit",
            ));
        }
        let scheme = match self.deployment {
            Deployment::Cloud => AuthScheme::Basic,
            Deployment::DataCenter => AuthScheme::Bearer,
        };
        let request = Request::get(url)
            .header("Accept", "application/json")
            .credential(&self.credential, scheme);
        let response = match self.ctx.fetch(request) {
            Ok(response) => response,
            Err(e) if too_large(&e) => {
                return Err(fail(
                    codes::INVALID_RESPONSE,
                    format!("Jira's answer for {what} is larger than {MAX_ANSWER_KIB} KiB"),
                ));
            }
            Err(e) => return Err(PluginError::from(e)),
        };
        let date = self.deadline.observe(&response);
        if !response.is_success() {
            return Err(status_failure(&response, what, date));
        }
        response.json().map_err(|_| {
            fail(
                codes::INVALID_RESPONSE,
                format!("Jira's answer for {what} is not JSON"),
            )
        })
    }
}

fn status_failure(response: &Response, what: &str, now: Option<u64>) -> Failure {
    let status = response.status;
    let detail = explanation(response)
        .map(|m| format!(": {m}"))
        .unwrap_or_default();
    if let Some(wait) = rate_limit_wait(status, response, now) {
        return fail(
            codes::RATE_LIMITED,
            format!("Jira's rate limit is reached; try again in {wait} s"),
        )
        .retryable();
    }
    match status {
        401 => fail(
            codes::UNAUTHORIZED,
            format!(
                "Jira refused the credential stored for this plugin (`{CREDENTIAL_SLOT}`) \
                 (401); it may be wrong, expired or revoked"
            ),
        ),
        403 => fail(
            codes::FORBIDDEN,
            format!("the account may not read {what} (403){detail}"),
        ),
        404 => fail(
            codes::NOT_FOUND,
            format!("{what} was not found, or the account cannot see it (404)"),
        ),
        _ => fail(
            codes::JIRA_ERROR,
            format!("Jira answered {status} for {what}{detail}"),
        ),
    }
}
