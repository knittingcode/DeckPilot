//! Talking to the GitHub REST API: which URLs are used, how the token is
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

use crate::{GithubSettings, codes};

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
/// The REST API version asked for.
pub const API_VERSION: &str = "2022-11-28";

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
            format!("the GitHub API URL {reason}; set it in Settings › Modules › GitHub"),
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

/// The web address that belongs to an API: `github.com` for
/// `api.github.com`, `<x>.ghe.com` for `api.<x>.ghe.com`, the same host
/// for GitHub Enterprise Server (`https://<host>/api/v3`).
pub fn web_base(api: &url::Url) -> url::Url {
    let mut web = api.clone();
    web.set_path("/");
    web.set_query(None);
    web.set_fragment(None);
    let host = api.host_str().unwrap_or_default().to_ascii_lowercase();
    let web_host = if host == "api.github.com" {
        Some("github.com".to_owned())
    } else {
        host.strip_prefix("api.")
            .filter(|rest| rest.ends_with(".ghe.com"))
            .map(str::to_owned)
    };
    if let Some(web_host) = web_host {
        let _ = web.set_host(Some(&web_host));
    }
    web
}

/// A page to open: https, on the GitHub web host, without credentials.
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
    {
        return Err(refused());
    }
    Ok(url)
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

fn header<'h>(headers: &'h [(String, String)], name: &str) -> Option<&'h str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
}

/// How long to wait when an answer says the rate limit is reached: a
/// secondary limit's `Retry-After`, or the primary limit's reset time
/// when no requests remain. `None` when the answer is not about the rate
/// limit.
pub fn rate_limit_wait(status: u16, headers: &[(String, String)], now: u64) -> Option<u64> {
    if status != 403 && status != 429 {
        return None;
    }
    if let Some(seconds) = header(headers, "retry-after").and_then(|v| v.parse::<u64>().ok()) {
        return Some(seconds);
    }
    if header(headers, "x-ratelimit-remaining") == Some("0") {
        let reset = header(headers, "x-ratelimit-reset").and_then(|v| v.parse::<u64>().ok());
        return Some(reset.map_or(60, |reset| reset.saturating_sub(now)));
    }
    (status == 429).then_some(60)
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
        settings: &GithubSettings,
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
                    "no credential named `{credential}` is stored on this computer; add a GitHub \
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

    /// The web address of the GitHub this API belongs to.
    pub fn web(&self) -> &url::Url {
        &self.web
    }

    /// An API URL: the base, these path segments (each encoded) and query.
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

    /// A link GitHub gave for the next page: only on the API itself.
    fn on_api(&self, link: &str) -> Result<url::Url, Failure> {
        let invalid = || {
            fail(
                codes::INVALID_RESPONSE,
                "GitHub pointed to a next page outside its API",
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
                    "the GitHub API host {} is on this computer or a private network; allow it \
                     in Settings › Modules › GitHub",
                    self.base.host_str().unwrap_or_default()
                ),
            );
        }
        fail(code, message)
    }

    /// Sends `GET url` and reads a JSON answer. `what` names the thing
    /// asked for in messages, e.g. “the repository a/b”.
    pub fn get(&self, url: url::Url, what: &str) -> Result<Reply, Failure> {
        if self.cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(fail("cancelled", "cancelled"));
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(fail(
                "timed-out",
                "GitHub did not answer within the time limit",
            ));
        }
        let request = client::Request {
            method: reqwest::Method::GET,
            url,
            headers: vec![
                ("Accept".into(), "application/vnd.github+json".into()),
                ("X-GitHub-Api-Version".into(), API_VERSION.into()),
            ],
            body: None,
            auth: Some(Auth::Bearer(self.secret.clone())),
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
                    "GitHub's answer for {what} is larger than {} MiB",
                    MAX_RESPONSE_BYTES / 1024 / 1024
                ),
            ));
        }
        let json = serde_json::from_slice(&response.body).map_err(|_| {
            fail(
                codes::INVALID_RESPONSE,
                format!("GitHub's answer for {what} is not JSON"),
            )
        })?;
        let next = match header(&response.headers, "link").and_then(next_link) {
            Some(link) => Some(self.on_api(link)?),
            None => None,
        };
        Ok(Reply {
            json,
            next,
            rate_remaining: header(&response.headers, "x-ratelimit-remaining")
                .and_then(|v| v.parse().ok()),
        })
    }

    fn status_failure(&self, response: &client::Response, what: &str) -> Failure {
        let status = response.status;
        let detail = serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|v| {
                v.get("message")
                    .and_then(Value::as_str)
                    .map(|m| clean(m, 200))
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
                format!("GitHub's rate limit is reached; try again in {wait} s"),
            )
            .with_data("retryAfterSeconds", wait.to_string());
        }
        match status {
            401 => fail(
                codes::UNAUTHORIZED,
                format!(
                    "GitHub refused the token in the credential `{}` (401); it may be expired or \
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
                codes::GITHUB_ERROR,
                format!("GitHub answered {status} for {what}{detail}"),
            ),
        }
    }

    /// Reads a list from `key` of each page (or the page itself when it
    /// is an array), following `Link: rel="next"` for at most
    /// [`MAX_PAGES`] pages.
    pub fn get_list(&self, url: url::Url, what: &str, key: Option<&str>) -> Result<List, Failure> {
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
    fn repositories_are_owner_and_name() {
        let repo = Repo::parse(" octo-org/hello.world.git ").unwrap();
        assert_eq!(repo.full_name(), "octo-org/hello.world");
        for bad in [
            "", "octo", "a/b/c", "/b", "a/", "a/..", "a b/c", "a/c?x=1", "a/%2e%2e", "../etc",
        ] {
            assert_eq!(
                Repo::parse(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_REPOSITORY),
                "{bad}"
            );
        }
    }

    #[test]
    fn refs_are_names_safe_for_a_path() {
        for good in ["main", "feature/login", "v1.2.3", "0123abcd"] {
            assert_eq!(check_ref(good).unwrap(), good);
        }
        for bad in [
            "", "a..b", "-x", "/main", "main/", "a b", "a?b", "a\nb", "a//b", "@{u}",
        ] {
            assert!(check_ref(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn api_and_web_addresses() {
        let public = api_base("https://api.github.com").unwrap();
        assert_eq!(web_base(&public).as_str(), "https://github.com/");
        let cloud = api_base("https://api.octo.ghe.com").unwrap();
        assert_eq!(web_base(&cloud).as_str(), "https://octo.ghe.com/");
        let server = api_base("https://ghe.example.com/api/v3").unwrap();
        assert_eq!(web_base(&server).as_str(), "https://ghe.example.com/");
        assert!(api_base("http://127.0.0.1:8080").is_ok());
        for bad in [
            "http://api.github.com",
            "ftp://x",
            "https://u:p@api.github.com",
            "https://api.github.com?x=1",
            "not a url",
        ] {
            assert_eq!(
                api_base(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_API_URL),
                "{bad}"
            );
        }

        let web = web_base(&public);
        assert!(web_url("https://github.com/a/b/pull/1", &web).is_ok());
        assert!(web_url("https://GitHub.com/a/b", &web).is_ok());
        for bad in [
            "http://github.com/a/b",
            "https://github.com.evil.com/a",
            "https://evil.com/github.com",
            "https://user@github.com/a",
            "https://github.com:8443/a",
            "javascript:alert(1)",
            "file:///etc/passwd",
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
            rate_limit_wait(
                403,
                &headers(&[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "1100")
                ]),
                1000
            ),
            Some(100)
        );
        assert_eq!(
            rate_limit_wait(429, &headers(&[("retry-after", "30")]), 0),
            Some(30)
        );
        assert_eq!(rate_limit_wait(429, &[], 0), Some(60));
        assert_eq!(
            rate_limit_wait(403, &headers(&[("x-ratelimit-remaining", "12")]), 0),
            None
        );
        assert_eq!(
            rate_limit_wait(500, &headers(&[("retry-after", "1")]), 0),
            None
        );

        let link = "<https://api.github.com/x?page=2>; rel=\"next\", \
                    <https://api.github.com/x?page=9>; rel=\"last\"";
        assert_eq!(next_link(link), Some("https://api.github.com/x?page=2"));
        assert_eq!(
            next_link("<https://api.github.com/x?page=1>; rel=\"prev\""),
            None
        );
    }

    #[test]
    fn text_is_one_clean_line() {
        assert_eq!(clean("a\nb\u{202e}c", 10), "a b c");
        assert_eq!(clean("abcdef", 3), "abc…");
    }
}
