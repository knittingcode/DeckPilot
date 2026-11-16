//! Talking to the GitLab REST API through the agent's broker: which URLs
//! are used, how the token is named, how answers and failures are read,
//! how far lists are paged, and the action's time limit.
//!
//! A port of the built-in module's `modules/gitlab/src/api.rs`, with the
//! differences a sandboxed plugin has (`plugins/first-party/gitlab/README.md`):
//!
//! * the API is `https://gitlab.com/api/v4` (the manifest's `network`
//!   host) or the server the user entered in `apiUrl` and allowed (its
//!   user-origin slot); pages are opened on the same server;
//! * the token is the credential slot [`TOKEN_SLOT`], injected by the
//!   agent in the `PRIVATE-TOKEN` header; it never enters this module;
//! * lists are read 50 items per page (an answer is at most 512 KiB), and
//!   10 per page when 50 are too large, for the same 500 items the
//!   built-in reads in 5 pages of 100;
//! * there is no clock in a guest: the time limit and rate-limit waits are
//!   counted with the `Date` headers of GitLab's answers.

use deckpilot_first_party_support::{Deadline, MAX_ANSWER_KIB, Server, retry_after, too_large};
use deckpilot_plugin_sdk::prelude::*;
use serde_json::Value;

pub use deckpilot_first_party_support::clean;

/// GitLab.com's API: the default of the `apiUrl` setting.
pub const DEFAULT_API_URL: &str = "https://gitlab.com/api/v4";
/// The path of the API under a GitLab's web address.
pub const API_SUFFIX: &str = "/api/v4";
/// The credential slot (`contributes.credentials[0].key`).
pub const TOKEN_SLOT: &str = "token";
/// Settings (`contributes.settings.fields`).
pub const API_URL: &str = "apiUrl";
pub const TIMEOUT: &str = "timeoutSeconds";
/// Items asked for per page: well below the 512 KiB an answer may have.
pub const PER_PAGE: usize = 50;
/// Items per page when a page of [`PER_PAGE`] is too large (merge
/// requests with very long descriptions).
pub const SMALL_PAGE: usize = 10;
/// Most items read to count a list or the jobs of a pipeline: the
/// built-in's 5 pages of 100.
pub const MAX_ITEMS: usize = 500;

/// Stable error codes, the built-in module's (`deckpilot_module_gitlab::codes`);
/// each is declared in `manifest.json` `contributes.errorCodes`.
pub mod codes {
    pub const INVALID_API_URL: &str = "invalid-api-url";
    pub const INVALID_PROJECT: &str = "invalid-project";
    pub const INVALID_REF: &str = "invalid-ref";
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const FORBIDDEN: &str = "forbidden";
    pub const NOT_FOUND: &str = "not-found";
    pub const RATE_LIMITED: &str = "rate-limited";
    pub const GITLAB_ERROR: &str = "gitlab-error";
    pub const INVALID_RESPONSE: &str = "invalid-response";
    pub const URL_NOT_ALLOWED: &str = "url-not-allowed";
    pub const TIMED_OUT: &str = "timed-out";
    /// Every code above.
    pub const ALL: &[&str] = &[
        INVALID_API_URL,
        INVALID_PROJECT,
        INVALID_REF,
        UNAUTHORIZED,
        FORBIDDEN,
        NOT_FOUND,
        RATE_LIMITED,
        GITLAB_ERROR,
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

/// A project: its full path such as `group/subgroup/project`, or its
/// numeric id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project(String);

impl Project {
    pub fn parse(text: &str) -> Result<Self, Failure> {
        let text = text.trim();
        let invalid = || {
            fail(
                codes::INVALID_PROJECT,
                format!(
                    "“{}” is not a project such as group/project",
                    clean(text, 60)
                ),
            )
        };
        let text = text.strip_suffix(".git").unwrap_or(text);
        if !text.is_empty() && text.len() <= 20 && text.chars().all(|c| c.is_ascii_digit()) {
            return Ok(Self(text.to_owned()));
        }
        let segments: Vec<&str> = text.split('/').collect();
        let valid = text.len() <= 1000
            && (2..=21).contains(&segments.len())
            && segments.iter().all(|s| {
                (1..=255).contains(&s.len())
                    && *s != "."
                    && *s != ".."
                    && !s.starts_with('-')
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            });
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(invalid())
        }
    }

    /// The path or id, as GitLab takes it in one (encoded) URL segment.
    pub fn id(&self) -> &str {
        &self.0
    }

    pub fn is_numeric(&self) -> bool {
        self.0.chars().all(|c| c.is_ascii_digit())
    }
}

/// A branch, tag or commit name.
pub fn check_ref(text: &str) -> Result<&str, Failure> {
    let text = text.trim();
    let valid = (1..=250).contains(&text.len())
        && !text.starts_with(['/', '-', '.'])
        && !text.ends_with(['/', '.'])
        && !text.contains("..")
        && !text.contains("//")
        && !text
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\@{}".contains(c));
    if valid {
        Ok(text)
    } else {
        Err(fail(
            codes::INVALID_REF,
            format!("“{}” is not a branch, tag or commit name", clean(text, 60)),
        ))
    }
}

/// The API's base from the `apiUrl` setting: an https address without a
/// user name, query or fragment. The agent normalizes the setting when it
/// is saved and allows requests only to GitLab.com or to the server the
/// user allowed; a plain `http` address on this computer, which the
/// built-in accepts, is not reachable for a plugin.
pub fn api_base(text: &str) -> Result<Server, Failure> {
    Server::parse(text).ok_or_else(|| {
        fail(
            codes::INVALID_API_URL,
            "the GitLab API URL must be an https address such as https://gitlab.com/api/v4, \
             without a user name, query or fragment; set it in Settings › Plugins › GitLab",
        )
    })
}

/// The web address that belongs to an API: the API URL without its
/// `/api/v4` (GitLab may be installed under a path such as `/gitlab`).
pub fn web_base(api: &Server) -> Server {
    api.without_suffix(API_SUFFIX)
}

/// A page to open: https, on the GitLab web address, without credentials.
pub fn web_url(
    text: &str,
    web: &Server,
) -> Result<deckpilot_first_party_support::WebPage, Failure> {
    web.page(text).ok_or_else(|| {
        fail(
            codes::URL_NOT_ALLOWED,
            format!("only https pages on {} can be opened", web.host()),
        )
    })
}

/// How long to wait when GitLab answers 429: `Retry-After`, else the
/// limit's reset time (`RateLimit-Reset`, Unix seconds), else a minute.
/// `now` is the answer's `Date`; without it a reset time or an HTTP date
/// cannot be counted from, and a minute is assumed.
pub fn rate_limit_wait(status: u16, response: &Response, now: Option<u64>) -> Option<u64> {
    if status != 429 {
        return None;
    }
    let header = |name: &str| response.header(name).map(str::trim);
    if let Some(seconds) = header("retry-after").and_then(|v| retry_after(v, now)) {
        return Some(seconds);
    }
    let reset = header("ratelimit-reset").and_then(|v| v.parse::<u64>().ok());
    Some(match (reset, now) {
        (Some(reset), Some(now)) => reset.saturating_sub(now),
        _ => 60,
    })
}

/// One successful answer.
#[derive(Debug)]
pub struct Reply {
    pub json: Value,
    /// The next page (`link: rel="next"`), on the API.
    pub next: Option<String>,
    /// `X-Total`: how many items the whole list has, when GitLab says.
    pub total: Option<u64>,
    pub rate_remaining: Option<u64>,
}

/// The items of a paged list.
#[derive(Debug)]
pub struct List {
    pub items: Vec<Value>,
    /// `X-Total` of the first page, when given.
    pub total: Option<u64>,
    /// More items exist than were read.
    pub more: bool,
    pub pages: usize,
}

/// Why a request gave no answer to read.
#[derive(Debug)]
enum Refused {
    /// The answer is larger than the agent hands to a plugin.
    TooLarge,
    Failed(Failure),
}

impl From<Failure> for Refused {
    fn from(failure: Failure) -> Self {
        Self::Failed(failure)
    }
}

fn too_large_failure(what: &str) -> Failure {
    fail(
        codes::INVALID_RESPONSE,
        format!("GitLab's answer for {what} is larger than {MAX_ANSWER_KIB} KiB"),
    )
}

/// A connection to the API for one action: the base URL, the token's
/// slot, and the action's time limit counted with GitLab's `Date`.
pub struct Api<'c, 'h> {
    ctx: &'c mut Context<'h>,
    token: SecretHandle,
    api: Server,
    web: Server,
    deadline: Deadline,
}

impl std::fmt::Debug for Api<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("api", &self.api)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

/// The settings, read for every action (ABI 1 has no change event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabSettings {
    pub api_url: String,
    pub timeout: u64,
}

pub fn settings(ctx: &mut Context<'_>) -> Result<GitlabSettings, Failure> {
    let settings = ctx.settings()?;
    Ok(GitlabSettings {
        api_url: settings
            .text(API_URL)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(DEFAULT_API_URL)
            .to_owned(),
        timeout: settings
            .integer(TIMEOUT)
            .unwrap_or(20)
            .clamp(1, 120)
            .unsigned_abs(),
    })
}

impl<'c, 'h> Api<'c, 'h> {
    /// Checks the settings and that a token is stored.
    pub fn connect(ctx: &'c mut Context<'h>) -> Result<Self, Failure> {
        let settings = settings(ctx)?;
        let api = api_base(&settings.api_url)?;
        let token = SecretHandle::slot(TOKEN_SLOT)?;
        if !ctx.has_credential(&token)? {
            return Err(fail(
                deckpilot_plugin_sdk::codes::CREDENTIAL_MISSING,
                "no GitLab token is stored for this plugin; add one in Settings › Plugins › \
                 GitLab",
            ));
        }
        Ok(Self {
            ctx,
            token,
            web: web_base(&api),
            api,
            deadline: Deadline::new(settings.timeout),
        })
    }

    /// The web address of the GitLab this API belongs to, ending in `/`.
    pub fn web(&self) -> String {
        self.web.root()
    }

    /// An API URL: the base, these path segments (each encoded, so a
    /// project path becomes one segment) and query.
    pub fn endpoint(&self, segments: &[&str], query: &[(&str, &str)]) -> String {
        self.api.endpoint(segments, query)
    }

    fn fetch(&mut self, url: String, what: &str) -> Result<Reply, Refused> {
        if self.deadline.passed() {
            return Err(fail(
                codes::TIMED_OUT,
                "GitLab did not answer within the time limit",
            )
            .into());
        }
        let request = Request::get(url)
            .header("Accept", "application/json")
            .credential(&self.token, AuthScheme::Header);
        let response = match self.ctx.fetch(request) {
            Ok(response) => response,
            Err(e) if too_large(&e) => return Err(Refused::TooLarge),
            Err(e) => return Err(PluginError::from(e).into()),
        };
        let date = self.deadline.observe(&response);
        if !response.is_success() {
            return Err(status_failure(&response, what, date).into());
        }
        let json: Value = response.json().map_err(|_| {
            fail(
                codes::INVALID_RESPONSE,
                format!("GitLab's answer for {what} is not JSON"),
            )
        })?;
        let next = match response.link("next") {
            Some(link) if self.api.contains(link) => Some(link.trim().to_owned()),
            Some(_) => {
                return Err(fail(
                    codes::INVALID_RESPONSE,
                    "GitLab pointed to a next page outside its API",
                )
                .into());
            }
            None => None,
        };
        let number = |name: &str| response.header(name).and_then(|v| v.trim().parse().ok());
        Ok(Reply {
            json,
            next,
            total: number("x-total"),
            rate_remaining: number("ratelimit-remaining"),
        })
    }

    /// Sends `GET url` and reads a JSON answer. `what` names the thing
    /// asked for in messages, e.g. “the project a/b”.
    pub fn get(&mut self, url: String, what: &str) -> Result<Reply, Failure> {
        self.fetch(url, what).map_err(|refused| match refused {
            Refused::TooLarge => too_large_failure(what),
            Refused::Failed(failure) => failure,
        })
    }

    /// Reads at most `limit` items of a list (each page an array) in pages
    /// of `per_page`, following `link: rel="next"`.
    fn pages(
        &mut self,
        segments: &[&str],
        query: &[(&str, &str)],
        what: &str,
        limit: usize,
        per_page: usize,
    ) -> Result<List, Refused> {
        let size = per_page.to_string();
        let mut query = query.to_vec();
        query.push(("per_page", &size));
        let mut url = self.endpoint(segments, &query);
        let mut list = List {
            items: Vec::new(),
            total: None,
            more: false,
            pages: 0,
        };
        let max_pages = limit.div_ceil(per_page).max(1);
        loop {
            let reply = self.fetch(url, what)?;
            list.pages += 1;
            let items = reply.json.as_array().ok_or_else(|| {
                fail(
                    codes::INVALID_RESPONSE,
                    format!("GitLab's answer for {what} has no list"),
                )
            })?;
            list.items.extend(items.iter().cloned());
            if list.pages == 1 {
                list.total = reply.total;
            }
            match reply.next {
                Some(next) if list.pages < max_pages && list.items.len() < limit => url = next,
                Some(_) => {
                    list.more = true;
                    break;
                }
                None => break,
            }
        }
        if list.items.len() > limit {
            list.items.truncate(limit);
            list.more = true;
        }
        Ok(list)
    }

    /// Reads at most `limit` items of a list: [`PER_PAGE`] per page (all
    /// in one page when `limit` is smaller), and [`SMALL_PAGE`] per page
    /// when such a page is larger than an answer may be.
    pub fn get_list(
        &mut self,
        segments: &[&str],
        query: &[(&str, &str)],
        what: &str,
        limit: usize,
    ) -> Result<List, Failure> {
        let limit = limit.max(1);
        let first = limit.min(PER_PAGE);
        match self.pages(segments, query, what, limit, first) {
            Ok(list) => Ok(list),
            Err(Refused::Failed(failure)) => Err(failure),
            Err(Refused::TooLarge) if first > SMALL_PAGE => {
                match self.pages(segments, query, what, limit, SMALL_PAGE) {
                    Ok(list) => Ok(list),
                    Err(Refused::Failed(failure)) => Err(failure),
                    Err(Refused::TooLarge) => Err(too_large_failure(what)),
                }
            }
            Err(Refused::TooLarge) => Err(too_large_failure(what)),
        }
    }
}

fn status_failure(response: &Response, what: &str, now: Option<u64>) -> Failure {
    let status = response.status;
    let detail = response
        .json::<Value>()
        .ok()
        .and_then(|v| {
            ["message", "error_description", "error"]
                .iter()
                .find_map(|key| v.get(*key).and_then(Value::as_str).map(|m| clean(m, 200)))
        })
        .filter(|m| !m.is_empty())
        .map(|m| format!(": {m}"))
        .unwrap_or_default();
    if let Some(wait) = rate_limit_wait(status, response, now) {
        return fail(
            codes::RATE_LIMITED,
            format!("GitLab's rate limit is reached; try again in {wait} s"),
        )
        .retryable();
    }
    match status {
        401 => fail(
            codes::UNAUTHORIZED,
            format!(
                "GitLab refused the token stored for this plugin (`{TOKEN_SLOT}`) (401); it may \
                 be expired or revoked"
            ),
        ),
        403 => fail(
            codes::FORBIDDEN,
            format!("the token may not read {what} (403){detail}"),
        ),
        404 => fail(
            codes::NOT_FOUND,
            format!("{what} was not found, or the token cannot see it (404)"),
        ),
        _ => fail(
            codes::GITLAB_ERROR,
            format!("GitLab answered {status} for {what}{detail}"),
        ),
    }
}
