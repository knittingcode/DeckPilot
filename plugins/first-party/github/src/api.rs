//! Talking to the GitHub REST API through the agent's broker: which URLs
//! are used, how the token is named, how answers and failures are read,
//! how far lists are paged, and the action's time limit.
//!
//! A port of the built-in module's `modules/github/src/api.rs`, with the
//! differences a sandboxed plugin has (`docs/marketplace/module-migration.md`
//! §6 and `plugins/first-party/github/README.md`):
//!
//! * only `https://api.github.com` (the manifest's one `network` origin)
//!   and `https://github.com` pages (its one `openUrl` origin);
//! * the token is the credential slot [`TOKEN_SLOT`], injected by the
//!   agent as a bearer token; it never enters this module;
//! * lists are read 50 items per page (an answer is at most 512 KiB) for
//!   up to 10 pages, the same 500 items the built-in reads in 5 pages of
//!   100;
//! * there is no clock in a guest: the time limit and rate-limit waits are
//!   counted with the `Date` headers of GitHub's answers.

use deckpilot_plugin_sdk::prelude::*;
use serde_json::Value;

use crate::urls;

/// The only API origin: the manifest's `network` permission.
pub const API: &str = "https://api.github.com";
/// The only web origin pages are opened on: the `openUrl` permission.
pub const WEB: &str = urls::WEB_BASE;
/// The credential slot (`contributes.credentials[0].key`).
pub const TOKEN_SLOT: &str = "token";
/// Settings (`contributes.settings.fields`).
pub const API_URL: &str = "apiUrl";
pub const TIMEOUT: &str = "timeoutSeconds";
/// Most pages read for one list.
pub const MAX_PAGES: usize = 10;
/// Items asked for per page: well below the 512 KiB an answer may have.
pub const PER_PAGE: usize = 50;
/// The REST API version asked for.
pub const API_VERSION: &str = "2022-11-28";
/// GitHub refuses requests without a User-Agent; the agent sets none for
/// plugins.
const USER_AGENT: &str = concat!("DeckPilot-GitHub/", env!("CARGO_PKG_VERSION"));
/// Largest answer the agent hands to a plugin.
const MAX_ANSWER_KIB: usize = 512;

/// Stable error codes, the built-in module's (`deckpilot_module_github::codes`);
/// each is declared in `manifest.json` `contributes.errorCodes`.
pub mod codes {
    pub const INVALID_API_URL: &str = "invalid-api-url";
    pub const INVALID_REPOSITORY: &str = "invalid-repository";
    pub const INVALID_REF: &str = "invalid-ref";
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const FORBIDDEN: &str = "forbidden";
    pub const NOT_FOUND: &str = "not-found";
    pub const RATE_LIMITED: &str = "rate-limited";
    pub const GITHUB_ERROR: &str = "github-error";
    pub const INVALID_RESPONSE: &str = "invalid-response";
    pub const URL_NOT_ALLOWED: &str = "url-not-allowed";
    pub const TIMED_OUT: &str = "timed-out";
    /// Every code above.
    pub const ALL: &[&str] = &[
        INVALID_API_URL,
        INVALID_REPOSITORY,
        INVALID_REF,
        UNAUTHORIZED,
        FORBIDDEN,
        NOT_FOUND,
        RATE_LIMITED,
        GITHUB_ERROR,
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

/// A repository named `owner/name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// `owner/name`, optionally ending in `.git`.
    pub fn parse(text: &str) -> Result<Self, Failure> {
        let text = text.trim();
        let invalid = || {
            fail(
                codes::INVALID_REPOSITORY,
                format!(
                    "“{}” is not a repository such as owner/name",
                    clean(text, 60)
                ),
            )
        };
        let text = text.strip_suffix(".git").unwrap_or(text);
        let (owner, name) = text.split_once('/').ok_or_else(invalid)?;
        let owner_ok = (1..=39).contains(&owner.len())
            && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        let name_ok = (1..=100).contains(&name.len())
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && name != "."
            && name != "..";
        if owner_ok && name_ok {
            Ok(Self {
                owner: owner.to_owned(),
                name: name.to_owned(),
            })
        } else {
            Err(invalid())
        }
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// A branch, tag or commit name, safe to put into a URL path.
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

/// Checks the API URL setting: `https://api.github.com` only. GitHub
/// Enterprise (Cloud or Server) and every other host keep the built-in
/// module until user-entered origins exist (G2).
pub fn api_base(text: &str) -> Result<(), Failure> {
    if urls::is_api(text) {
        return Ok(());
    }
    Err(fail(
        codes::INVALID_API_URL,
        "the GitHub API URL must be https://api.github.com; GitHub Enterprise and other hosts \
         use the built-in GitHub module (Settings › Modules › GitHub); set it in Settings › \
         Plugins › GitHub",
    ))
}

/// A page to open: https, on github.com, without credentials.
pub fn web_url(text: &str) -> Result<urls::WebPage, Failure> {
    urls::web_page(text).ok_or_else(|| {
        fail(
            codes::URL_NOT_ALLOWED,
            "only https pages on github.com can be opened",
        )
    })
}

/// Text from GitHub as shown in one line: control characters and
/// bidirectional overrides become spaces, at most `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    let mut out: String = text
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                c
            }
        })
        .take(max)
        .collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out.trim().to_owned()
}

/// An IMF-fixdate (RFC 9110 §5.6.7, `Sun, 06 Nov 1994 08:49:37 GMT`) as
/// Unix seconds.
pub fn http_date(value: &str) -> Option<u64> {
    let (_, rest) = value.trim().split_once(", ")?;
    let parts: Vec<&str> = rest.split(' ').collect();
    let [day, month, year, time, "GMT"] = parts.as_slice() else {
        return None;
    };
    let day: u64 = day.parse().ok().filter(|d| (1..=31).contains(d))?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| m == month)?;
    let year: u64 = year.parse().ok().filter(|y| *y >= 1970 && *y < 10_000)?;
    let clock: Vec<u64> = time
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let [hour, minute, second] = clock.as_slice() else {
        return None;
    };
    if *hour > 23 || *minute > 59 || *second > 60 {
        return None;
    }
    let leap = |y: u64| (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400);
    let days_before_month = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut days: u64 = (1970..year).map(|y| if leap(y) { 366 } else { 365 }).sum();
    days += days_before_month[month];
    if month > 1 && leap(year) {
        days += 1;
    }
    days += day - 1;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// How long to wait when an answer says the rate limit is reached: a
/// secondary limit's `Retry-After`, or the primary limit's reset time
/// when no requests remain, or a minute for a 429 or a 403 whose
/// `message` is about the rate limit. `None` when the answer is not about
/// the rate limit. `now` is the answer's `Date`; without it a reset time
/// or an HTTP date cannot be counted from, and a minute is assumed.
pub fn rate_limit_wait(
    status: u16,
    response: &Response,
    message: Option<&str>,
    now: Option<u64>,
) -> Option<u64> {
    if status != 403 && status != 429 {
        return None;
    }
    let header = |name: &str| response.header(name).map(str::trim);
    if let Some(value) = header("retry-after") {
        if let Ok(seconds) = value.parse::<u64>() {
            return Some(seconds);
        }
        if let Some(at) = http_date(value) {
            return Some(now.map_or(60, |now| at.saturating_sub(now)));
        }
    }
    if header("x-ratelimit-remaining") == Some("0") {
        let reset = header("x-ratelimit-reset").and_then(|v| v.parse::<u64>().ok());
        return Some(match (reset, now) {
            (Some(reset), Some(now)) => reset.saturating_sub(now),
            _ => 60,
        });
    }
    let about_the_limit = message.is_some_and(|m| m.to_ascii_lowercase().contains("rate limit"));
    (status == 429 || about_the_limit).then_some(60)
}

/// One successful answer.
#[derive(Debug)]
pub struct Reply {
    pub json: Value,
    /// The next page (`link: rel="next"`), on the API.
    pub next: Option<String>,
    pub rate_remaining: Option<u64>,
}

/// The items of a paged list.
#[derive(Debug)]
pub struct List {
    pub items: Vec<Value>,
    /// The first page's answer, for fields beside the list.
    pub first: Value,
    /// `total_count`, when GitHub gives one.
    pub total: Option<u64>,
    /// More pages exist than were read.
    pub truncated: bool,
    pub pages: usize,
}

/// A connection to the API for one action: the base URL, the token's
/// slot, and the action's time limit counted with GitHub's `Date`.
pub struct Api<'c, 'h> {
    ctx: &'c mut Context<'h>,
    token: SecretHandle,
    timeout: u64,
    /// The `Date` of the first answer and of the latest one.
    first_date: Option<u64>,
    latest_date: Option<u64>,
}

impl std::fmt::Debug for Api<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// The settings, read for every action (ABI 1 has no change event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubSettings {
    pub api_url: String,
    pub timeout: u64,
}

pub fn settings(ctx: &mut Context<'_>) -> Result<GithubSettings, Failure> {
    let settings = ctx.settings()?;
    Ok(GithubSettings {
        api_url: settings
            .text(API_URL)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(API)
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
        api_base(&settings.api_url)?;
        let token = SecretHandle::slot(TOKEN_SLOT)?;
        if !ctx.has_credential(&token)? {
            return Err(fail(
                deckpilot_plugin_sdk::codes::CREDENTIAL_MISSING,
                "no GitHub token is stored for this plugin; add one in Settings › Plugins › \
                 GitHub",
            ));
        }
        Ok(Self {
            ctx,
            token,
            timeout: settings.timeout,
            first_date: None,
            latest_date: None,
        })
    }

    /// An API URL: these path segments (each encoded) and query.
    #[allow(clippy::unused_self)] // Read as `api.endpoint(…)`, like the built-in.
    pub fn endpoint(&self, segments: &[&str], query: &[(&str, &str)]) -> String {
        urls::endpoint(segments, query)
    }

    /// Fails once the time limit has passed since the first answer.
    fn check_time(&self) -> Result<(), Failure> {
        if let (Some(first), Some(latest)) = (self.first_date, self.latest_date)
            && latest.saturating_sub(first) >= self.timeout
        {
            return Err(fail(
                codes::TIMED_OUT,
                "GitHub did not answer within the time limit",
            ));
        }
        Ok(())
    }

    /// Sends `GET url` and reads a JSON answer. `what` names the thing
    /// asked for in messages, e.g. “the repository a/b”.
    pub fn get(&mut self, url: String, what: &str) -> Result<Reply, Failure> {
        self.check_time()?;
        let request = Request::get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .header("User-Agent", USER_AGENT)
            .bearer(&self.token);
        let response = self
            .ctx
            .fetch(request)
            .map_err(|e| fetch_failure(&e, what))?;
        let date = response.header("date").and_then(http_date);
        if date.is_some() {
            self.first_date = self.first_date.or(date);
            self.latest_date = date;
        }
        if !response.is_success() {
            return Err(self.status_failure(&response, what, date));
        }
        let json: Value = response.json().map_err(|_| {
            fail(
                codes::INVALID_RESPONSE,
                format!("GitHub's answer for {what} is not JSON"),
            )
        })?;
        let next = match response.link("next") {
            Some(link) if urls::on_api(link) => Some(link.trim().to_owned()),
            Some(_) => {
                return Err(fail(
                    codes::INVALID_RESPONSE,
                    "GitHub pointed to a next page outside its API",
                ));
            }
            None => None,
        };
        Ok(Reply {
            json,
            next,
            rate_remaining: response
                .header("x-ratelimit-remaining")
                .and_then(|v| v.trim().parse().ok()),
        })
    }

    #[allow(clippy::unused_self)] // Kept beside `get`, like the built-in's.
    fn status_failure(&self, response: &Response, what: &str, now: Option<u64>) -> Failure {
        let status = response.status;
        let message = response
            .json::<Value>()
            .ok()
            .and_then(|v| {
                v.get("message")
                    .and_then(Value::as_str)
                    .map(|m| clean(m, 200))
            })
            .filter(|m| !m.is_empty());
        let detail = message
            .as_deref()
            .map(|m| format!(": {m}"))
            .unwrap_or_default();
        if let Some(wait) = rate_limit_wait(status, response, message.as_deref(), now) {
            return fail(
                codes::RATE_LIMITED,
                format!("GitHub's rate limit is reached; try again in {wait} s"),
            )
            .retryable();
        }
        match status {
            401 => fail(
                codes::UNAUTHORIZED,
                format!(
                    "GitHub refused the token stored for this plugin (`{TOKEN_SLOT}`) (401); it \
                     may be expired or revoked"
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
                codes::GITHUB_ERROR,
                format!("GitHub answered {status} for {what}{detail}"),
            ),
        }
    }

    /// Reads a list from `key` of each page (or the page itself when it
    /// is an array), following `link: rel="next"` for at most
    /// [`MAX_PAGES`] pages.
    pub fn get_list(
        &mut self,
        url: String,
        what: &str,
        key: Option<&str>,
    ) -> Result<List, Failure> {
        let mut url = url;
        let mut list = List {
            items: Vec::new(),
            first: Value::Null,
            total: None,
            truncated: false,
            pages: 0,
        };
        loop {
            let reply = self.get(url, what)?;
            list.pages += 1;
            let items = match key {
                Some(key) => reply.json.get(key),
                None => Some(&reply.json),
            }
            .and_then(Value::as_array)
            .ok_or_else(|| {
                fail(
                    codes::INVALID_RESPONSE,
                    format!("GitHub's answer for {what} has no list"),
                )
            })?;
            list.items.extend(items.iter().cloned());
            if list.total.is_none() {
                list.total = reply.json.get("total_count").and_then(Value::as_u64);
            }
            if list.pages == 1 {
                list.first = reply.json;
            }
            match reply.next {
                Some(next) if list.pages < MAX_PAGES => url = next,
                Some(_) => {
                    list.truncated = true;
                    return Ok(list);
                }
                None => return Ok(list),
            }
        }
    }
}

/// A request that did not get an answer.
fn fetch_failure(e: &CapabilityError, what: &str) -> Failure {
    if e.kind() == CapabilityErrorKind::Failed && e.detail().contains("larger than") {
        return fail(
            codes::INVALID_RESPONSE,
            format!("GitHub's answer for {what} is larger than {MAX_ANSWER_KIB} KiB"),
        );
    }
    PluginError::from(e.clone())
}
