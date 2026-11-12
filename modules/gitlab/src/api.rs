//! Talking to the GitLab REST API: which URLs are used, how the token is
//! sent, how answers and failures are read, how far lists are paged, and
//! how results are kept free of the token.
//!
//! Requests go through the HTTP module's [`client::send`] with its
//! [`NetworkPolicy`]; nothing here opens a connection itself.

use std::net::IpAddr;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use deckpilot_core::{
    ActionParameters, ActionResult, CancellationToken, Capability, CredentialError, CredentialId,
    ExecutionContext, Secret,
};
use deckpilot_module_http::client::{self, Auth};
use deckpilot_module_http::policy::{self, NetworkPolicy};
use serde_json::Value;

use crate::{GitlabSettings, codes};

/// A failed action.
pub type Failure = ActionResult;

pub fn fail(code: &str, message: impl Into<String>) -> Failure {
    ActionResult::error(code, message)
}

/// Largest response read, per page.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Most pages read for one list.
pub const MAX_PAGES: usize = 5;
/// Items asked for per page.
pub const PER_PAGE: usize = 100;
/// The header that carries the token.
pub const TOKEN_HEADER: &str = "PRIVATE-TOKEN";

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

fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

/// The API's base URL: https (http only on this computer), with a host,
/// without credentials, query or fragment.
pub fn api_base(text: &str) -> Result<url::Url, Failure> {
    let invalid = |reason: &str| {
        fail(
            codes::INVALID_API_URL,
            format!("the GitLab API URL {reason}; set it in Settings › Modules › GitLab"),
        )
    };
    let url = policy::request_url(text).map_err(|(_, message)| invalid(&message))?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("must not have a query or fragment"));
    }
    if url.scheme() != "https" && !is_loopback(&url) {
        return Err(invalid("must be an https address"));
    }
    Ok(url)
}

/// The web address that belongs to an API: the API URL without its
/// `/api/v4` (GitLab may be installed under a path such as `/gitlab`).
pub fn web_base(api: &url::Url) -> url::Url {
    let mut web = api.clone();
    let path = api.path().trim_end_matches('/');
    let prefix = path.strip_suffix("/api/v4").unwrap_or(path);
    web.set_path(&format!("{prefix}/"));
    web.set_query(None);
    web.set_fragment(None);
    web
}

/// A page to open: https, on the GitLab web host, without credentials.
pub fn web_url(text: &str, web: &url::Url) -> Result<url::Url, Failure> {
    let refused = || {
        fail(
            codes::URL_NOT_ALLOWED,
            format!(
                "only https pages on {} can be opened",
                web.host_str().unwrap_or_default()
            ),
        )
    };
    let url = url::Url::parse(text.trim()).map_err(|_| refused())?;
    let same_host = url
        .host_str()
        .zip(web.host_str())
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b));
    if url.scheme() != "https"
        || !same_host
        || url.port_or_known_default() != web.port_or_known_default()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with(web.path())
    {
        return Err(refused());
    }
    Ok(url)
}

/// Text from GitLab as shown in one line: control characters and
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

/// A header's value (names compared without case).
pub fn header<'h>(headers: &'h [(String, String)], name: &str) -> Option<&'h str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
}

/// How long to wait when GitLab answers 429: `Retry-After`, else the
/// limit's reset time (`RateLimit-Reset`, Unix seconds), else a minute.
pub fn rate_limit_wait(status: u16, headers: &[(String, String)], now: u64) -> Option<u64> {
    if status != 429 {
        return None;
    }
    if let Some(seconds) = header(headers, "retry-after").and_then(|v| client::retry_after(v, now))
    {
        return Some(seconds);
    }
    let reset = header(headers, "ratelimit-reset").and_then(|v| v.parse::<u64>().ok());
    Some(reset.map_or(60, |reset| reset.saturating_sub(now)))
}

/// The `rel="next"` target of a `Link` header.
pub fn next_link(value: &str) -> Option<&str> {
    value.split(',').find_map(|part| {
        let (target, params) = part.split_once(';')?;
        params
            .split(';')
            .any(|p| p.trim().replace(' ', "") == "rel=\"next\"")
            .then(|| target.trim().trim_start_matches('<').trim_end_matches('>'))
    })
}

/// One successful answer.
#[derive(Debug)]
pub struct Reply {
    pub json: Value,
    /// The next page, on the API host.
    pub next: Option<url::Url>,
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
    /// More pages exist than were read.
    pub truncated: bool,
    pub pages: usize,
}

/// A connection to the API for one action: the base URL, the network
/// policy, the token and the action's deadline.
#[derive(Debug)]
pub struct Api<'a> {
    base: url::Url,
    web: url::Url,
    network: NetworkPolicy,
    credential: CredentialId,
    secret: Secret,
    deadline: Instant,
    cancel: Option<&'a CancellationToken>,
}

impl<'a> Api<'a> {
    /// Checks the capability and the settings and looks the token up.
    pub fn connect(
        settings: &GitlabSettings,
        parameters: &ActionParameters,
        context: &ExecutionContext<'a>,
    ) -> Result<Self, Failure> {
        context
            .permit(Capability::Network)
            .map_err(ActionResult::from)?;
        let base = api_base(&settings.api_url)?;
        let name = parameters
            .text("credential")
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&settings.credential);
        let credential = CredentialId::new(name).map_err(|_| {
            fail(
                "invalid-parameters",
                format!("“{}” is not a credential name", clean(name, 64)),
            )
        })?;
        let secret = context.credential(&credential).map_err(|e| match e {
            CredentialError::Missing(_) => fail(
                e.code(),
                format!(
                    "no credential named `{credential}` is stored on this computer; add a GitLab \
                     token in Settings › HTTP › Credentials"
                ),
            ),
            CredentialError::NotPermitted => fail(e.code(), e.to_string()),
        })?;
        if secret.expose().trim().is_empty() {
            return Err(fail(
                "credential-missing",
                format!("the credential `{credential}` is empty"),
            ));
        }
        let host = base.host_str().unwrap_or_default().to_owned();
        let network = NetworkPolicy {
            allow_private: false,
            private_hosts: if settings.allow_private {
                vec![host]
            } else {
                Vec::new()
            },
        };
        Ok(Self {
            web: web_base(&base),
            base,
            network,
            credential,
            secret,
            deadline: Instant::now() + settings.timeout,
            cancel: context.cancellation(),
        })
    }

    /// The web address of the GitLab this API belongs to.
    pub fn web(&self) -> &url::Url {
        &self.web
    }

    /// An API URL: the base, these path segments (each encoded, so a
    /// project path becomes one segment) and query.
    pub fn endpoint(&self, segments: &[&str], query: &[(&str, &str)]) -> url::Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty();
            path.extend(segments);
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url
    }

    /// A link GitLab gave for the next page: only on the API itself.
    fn on_api(&self, link: &str) -> Result<url::Url, Failure> {
        let invalid = || {
            fail(
                codes::INVALID_RESPONSE,
                "GitLab pointed to a next page outside its API",
            )
        };
        let url = url::Url::parse(link).map_err(|_| invalid())?;
        let prefix = self.base.path().trim_end_matches('/');
        if url.scheme() != self.base.scheme()
            || url.host_str() != self.base.host_str()
            || url.port_or_known_default() != self.base.port_or_known_default()
            || !url.username().is_empty()
            || url.password().is_some()
            || !(url.path() == prefix || url.path().starts_with(&format!("{prefix}/")))
        {
            return Err(invalid());
        }
        Ok(url)
    }

    fn network_failure(&self, (code, message): client::Failure) -> Failure {
        if code == deckpilot_module_http::codes::BLOCKED_ADDRESS {
            return fail(
                code,
                format!(
                    "the GitLab API host {} is on this computer or a private network; allow it \
                     in Settings › Modules › GitLab",
                    self.base.host_str().unwrap_or_default()
                ),
            );
        }
        fail(code, message)
    }

    /// Sends `GET url` and reads a JSON answer. `what` names the thing
    /// asked for in messages, e.g. “the project a/b”.
    pub fn get(&self, url: url::Url, what: &str) -> Result<Reply, Failure> {
        if self.cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(fail("cancelled", "cancelled"));
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(fail(
                "timed-out",
                "GitLab did not answer within the time limit",
            ));
        }
        let request = client::Request {
            method: reqwest::Method::GET,
            url,
            headers: vec![("Accept".into(), "application/json".into())],
            body: None,
            auth: Some(Auth::Header(TOKEN_HEADER.into(), self.secret.clone())),
            timeout: remaining,
            follow_redirects: true,
            max_bytes: MAX_RESPONSE_BYTES,
        };
        let response = client::send(request, &self.network, self.cancel)
            .map_err(|failure| self.network_failure(failure))?;
        if !(200..300).contains(&response.status) {
            return Err(self.status_failure(&response, what));
        }
        if response.truncated {
            return Err(fail(
                codes::INVALID_RESPONSE,
                format!(
                    "GitLab's answer for {what} is larger than {} MiB",
                    MAX_RESPONSE_BYTES / 1024 / 1024
                ),
            ));
        }
        let json = serde_json::from_slice(&response.body).map_err(|_| {
            fail(
                codes::INVALID_RESPONSE,
                format!("GitLab's answer for {what} is not JSON"),
            )
        })?;
        let next = match header(&response.headers, "link").and_then(next_link) {
            Some(link) => Some(self.on_api(link)?),
            None => None,
        };
        Ok(Reply {
            json,
            next,
            total: header(&response.headers, "x-total").and_then(|v| v.parse().ok()),
            rate_remaining: header(&response.headers, "ratelimit-remaining")
                .and_then(|v| v.parse().ok()),
        })
    }

    fn status_failure(&self, response: &client::Response, what: &str) -> Failure {
        let status = response.status;
        let detail = serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|v| {
                ["message", "error_description", "error"]
                    .iter()
                    .find_map(|key| v.get(*key).and_then(Value::as_str).map(|m| clean(m, 200)))
            })
            .filter(|m| !m.is_empty())
            .map(|m| format!(": {m}"))
            .unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Some(wait) = rate_limit_wait(status, &response.headers, now) {
            return fail(
                codes::RATE_LIMITED,
                format!("GitLab's rate limit is reached; try again in {wait} s"),
            )
            .with_data("retryAfterSeconds", wait.to_string());
        }
        match status {
            401 => fail(
                codes::UNAUTHORIZED,
                format!(
                    "GitLab refused the token in the credential `{}` (401); it may be expired or \
                     revoked",
                    self.credential
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

    /// Reads a list (each page an array), following `Link: rel="next"`
    /// for at most `max_pages` pages.
    pub fn get_list(&self, url: url::Url, what: &str, max_pages: usize) -> Result<List, Failure> {
        let mut url = url;
        let mut list = List {
            items: Vec::new(),
            total: None,
            truncated: false,
            pages: 0,
        };
        loop {
            let reply = self.get(url, what)?;
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
                Some(next) if list.pages < max_pages.max(1) => url = next,
                Some(_) => {
                    list.truncated = true;
                    return Ok(list);
                }
                None => return Ok(list),
            }
        }
    }

    /// `result` with every appearance of the token replaced. Results never
    /// contain it by construction; this makes sure.
    pub fn scrub(&self, mut result: ActionResult) -> ActionResult {
        for secret in [self.secret.expose(), self.secret.expose().trim()] {
            if secret.is_empty() {
                continue;
            }
            if result.message.contains(secret) {
                result.message = result.message.replace(secret, "[redacted]");
            }
            for value in result.data.values_mut() {
                if value.contains(secret) {
                    *value = value.replace(secret, "[redacted]");
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn projects_are_paths_or_ids() {
        assert_eq!(
            Project::parse(" group/sub/project.git ").unwrap().id(),
            "group/sub/project"
        );
        assert!(Project::parse("42").unwrap().is_numeric());
        for bad in [
            "", "group", "a//b", "/b", "a/", "a/..", "a b/c", "a/c?x=1", "a/%2e", "-a/b", "../etc",
        ] {
            assert_eq!(
                Project::parse(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_PROJECT),
                "{bad}"
            );
        }
    }

    #[test]
    fn refs_are_names_safe_for_a_query() {
        for good in ["main", "feature/login", "v1.2.3", "0123abcd"] {
            assert_eq!(check_ref(good).unwrap(), good);
        }
        for bad in [
            "", "a..b", "-x", "/main", "main/", "a b", "a?b", "a\nb", "@{u}",
        ] {
            assert!(check_ref(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn api_and_web_addresses() {
        let public = api_base("https://gitlab.com/api/v4").unwrap();
        assert_eq!(web_base(&public).as_str(), "https://gitlab.com/");
        let sub = api_base("https://code.example.com/gitlab/api/v4/").unwrap();
        assert_eq!(web_base(&sub).as_str(), "https://code.example.com/gitlab/");
        assert!(api_base("http://127.0.0.1:8080/api/v4").is_ok());
        for bad in [
            "http://gitlab.com/api/v4",
            "ftp://x",
            "https://u:p@gitlab.com/api/v4",
            "https://gitlab.com/api/v4?x=1",
            "not a url",
        ] {
            assert_eq!(
                api_base(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_API_URL),
                "{bad}"
            );
        }

        let web = web_base(&sub);
        assert!(
            web_url(
                "https://code.example.com/gitlab/g/p/-/merge_requests/1",
                &web
            )
            .is_ok()
        );
        for bad in [
            "http://code.example.com/gitlab/g/p",
            "https://code.example.com/other/g/p",
            "https://code.example.com.evil.test/gitlab/",
            "https://user@code.example.com/gitlab/g",
            "javascript:alert(1)",
        ] {
            assert_eq!(
                web_url(bad, &web).unwrap_err().code.as_deref(),
                Some(codes::URL_NOT_ALLOWED),
                "{bad}"
            );
        }
    }

    #[test]
    fn rate_limits_and_links() {
        assert_eq!(
            rate_limit_wait(429, &headers(&[("RateLimit-Reset", "1100")]), 1000),
            Some(100)
        );
        assert_eq!(
            rate_limit_wait(429, &headers(&[("Retry-After", "30")]), 0),
            Some(30)
        );
        assert_eq!(rate_limit_wait(429, &[], 0), Some(60));
        // An HTTP date, counted from now (1794466800 is 07:00:00).
        assert_eq!(
            rate_limit_wait(
                429,
                &headers(&[("Retry-After", "Thu, 12 Nov 2026 07:00:45 GMT")]),
                1_794_466_800
            ),
            Some(45)
        );
        // A form not understood: the reset time, then a minute.
        assert_eq!(
            rate_limit_wait(429, &headers(&[("Retry-After", "later")]), 0),
            Some(60)
        );
        assert_eq!(
            rate_limit_wait(403, &headers(&[("Retry-After", "1")]), 0),
            None
        );
        let link = "<https://gitlab.com/api/v4/x?page=2>; rel=\"next\", \
                    <https://gitlab.com/api/v4/x?page=1>; rel=\"first\"";
        assert_eq!(next_link(link), Some("https://gitlab.com/api/v4/x?page=2"));
        assert_eq!(next_link("<https://x>; rel=\"prev\""), None);
    }

    #[test]
    fn text_is_one_clean_line() {
        assert_eq!(clean("a\nb\u{202e}c", 10), "a b c");
        assert_eq!(clean("abcdef", 3), "abc…");
    }
}
