//! Talking to the Jira REST API: which URLs are used, how the credential
//! is sent, how answers and failures are read, and how results are kept
//! free of the secret.
//!
//! Requests go through the HTTP module's [`client::send`] with its
//! [`NetworkPolicy`]; nothing here opens a connection itself.

use std::net::IpAddr;
use std::time::Instant;

use base64::Engine as _;
use deckpilot_core::{
    ActionParameters, ActionResult, CancellationToken, Capability, CredentialError, CredentialId,
    ExecutionContext, Secret,
};
use deckpilot_module_http::client::{self, Auth};
use deckpilot_module_http::policy::{self, NetworkPolicy};
use serde_json::Value;

use crate::{Deployment, JiraSettings, codes};

/// A failed action.
pub type Failure = ActionResult;

pub fn fail(code: &str, message: impl Into<String>) -> Failure {
    ActionResult::error(code, message)
}

/// Largest response read, per page.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Most pages read to count a search.
pub const MAX_PAGES: usize = 5;
/// Issues asked for per page when counting.
pub const PER_PAGE: usize = 100;

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

fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

/// The site's address: https (http only on this computer), with a host,
/// without credentials, query or fragment; always ending in `/`.
pub fn site(text: &str) -> Result<url::Url, Failure> {
    let invalid = |reason: &str| {
        fail(
            codes::INVALID_SITE_URL,
            format!("the Jira site {reason}; set it in Settings › Modules › Jira"),
        )
    };
    if text.trim().is_empty() {
        return Err(invalid("is not set"));
    }
    let mut url = policy::request_url(text).map_err(|(_, message)| invalid(&message))?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("must not have a query or fragment"));
    }
    if url.scheme() != "https" && !is_loopback(&url) {
        return Err(invalid("must be an https address"));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// A page to open: https, on the Jira site, without credentials.
pub fn web_url(text: &str, site: &url::Url) -> Result<url::Url, Failure> {
    let refused = || {
        fail(
            codes::URL_NOT_ALLOWED,
            format!(
                "only https pages on {} can be opened",
                site.host_str().unwrap_or_default()
            ),
        )
    };
    let url = url::Url::parse(text.trim()).map_err(|_| refused())?;
    let same_host = url
        .host_str()
        .zip(site.host_str())
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b));
    if url.scheme() != "https"
        || !same_host
        || url.port_or_known_default() != site.port_or_known_default()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with(site.path())
    {
        return Err(refused());
    }
    Ok(url)
}

/// Text from Jira as shown in one line: control characters and
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

/// How long to wait when Jira answers 429: `Retry-After` (seconds or an
/// HTTP date), else a minute.
pub fn rate_limit_wait(status: u16, headers: &[(String, String)]) -> Option<u64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    (status == 429).then(|| {
        header(headers, "retry-after")
            .and_then(|v| client::retry_after(v, now))
            .unwrap_or(60)
    })
}

/// Jira's own explanation of a failure: the first of `errorMessages`,
/// else the first of `errors`, else `message`.
fn explanation(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
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

/// A connection to the API for one action: the site, the network policy,
/// the credential and the action's deadline.
#[derive(Debug)]
pub struct Api<'a> {
    site: url::Url,
    deployment: Deployment,
    network: NetworkPolicy,
    credential: CredentialId,
    secret: Secret,
    deadline: Instant,
    cancel: Option<&'a CancellationToken>,
}

impl<'a> Api<'a> {
    /// Checks the capability and the settings and looks the secret up.
    pub fn connect(
        settings: &JiraSettings,
        parameters: &ActionParameters,
        context: &ExecutionContext<'a>,
    ) -> Result<Self, Failure> {
        context
            .permit(Capability::Network)
            .map_err(ActionResult::from)?;
        let site = site(&settings.site_url)?;
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
                    "no credential named `{credential}` is stored on this computer; add it in \
                     Settings › HTTP › Credentials"
                ),
            ),
            CredentialError::NotPermitted => fail(e.code(), e.to_string()),
        })?;
        let expected = match settings.deployment {
            Deployment::Cloud => secret.expose().contains(':'),
            Deployment::DataCenter => !secret.expose().trim().is_empty(),
        };
        if !expected {
            return Err(fail(
                "credential-missing",
                match settings.deployment {
                    Deployment::Cloud => format!(
                        "the credential `{credential}` must be email:api-token for Jira Cloud"
                    ),
                    Deployment::DataCenter => format!("the credential `{credential}` is empty"),
                },
            ));
        }
        let host = site.host_str().unwrap_or_default().to_owned();
        let network = NetworkPolicy {
            allow_private: false,
            private_hosts: if settings.allow_private {
                vec![host]
            } else {
                Vec::new()
            },
        };
        Ok(Self {
            site,
            deployment: settings.deployment,
            network,
            credential,
            secret,
            deadline: Instant::now() + settings.timeout,
            cancel: context.cancellation(),
        })
    }

    pub fn site(&self) -> &url::Url {
        &self.site
    }

    pub fn deployment(&self) -> Deployment {
        self.deployment
    }

    /// A REST API URL: `rest/api/3` (Cloud) or `rest/api/2` (Data Center),
    /// these path segments (each encoded) and query.
    pub fn endpoint(&self, segments: &[&str], query: &[(&str, &str)]) -> url::Url {
        let version = match self.deployment {
            Deployment::Cloud => "3",
            Deployment::DataCenter => "2",
        };
        let mut url = self.site.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty();
            path.extend(["rest", "api", version]);
            path.extend(segments);
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url
    }

    /// The page of an issue: `<site>/browse/KEY`.
    pub fn browse(&self, key: &str) -> String {
        format!("{}browse/{key}", self.site)
    }

    fn network_failure(&self, (code, message): client::Failure) -> Failure {
        if code == deckpilot_module_http::codes::BLOCKED_ADDRESS {
            return fail(
                code,
                format!(
                    "the Jira site {} is on this computer or a private network; allow it in \
                     Settings › Modules › Jira",
                    self.site.host_str().unwrap_or_default()
                ),
            );
        }
        fail(code, message)
    }

    /// Sends `GET url` and reads a JSON answer. `what` names the thing
    /// asked for in messages, e.g. “the issue ABC-1”.
    pub fn get(&self, url: url::Url, what: &str) -> Result<Value, Failure> {
        if self.cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(fail("cancelled", "cancelled"));
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(fail(
                "timed-out",
                "Jira did not answer within the time limit",
            ));
        }
        let auth = match self.deployment {
            Deployment::Cloud => Auth::Basic(self.secret.clone()),
            Deployment::DataCenter => Auth::Bearer(Secret::new(self.secret.expose().trim())),
        };
        let request = client::Request {
            method: reqwest::Method::GET,
            url,
            headers: vec![("Accept".into(), "application/json".into())],
            body: None,
            auth: Some(auth),
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
                    "Jira's answer for {what} is larger than {} MiB",
                    MAX_RESPONSE_BYTES / 1024 / 1024
                ),
            ));
        }
        serde_json::from_slice(&response.body).map_err(|_| {
            fail(
                codes::INVALID_RESPONSE,
                format!("Jira's answer for {what} is not JSON"),
            )
        })
    }

    fn status_failure(&self, response: &client::Response, what: &str) -> Failure {
        let status = response.status;
        let detail = explanation(&response.body)
            .map(|m| format!(": {m}"))
            .unwrap_or_default();
        if let Some(wait) = rate_limit_wait(status, &response.headers) {
            return fail(
                codes::RATE_LIMITED,
                format!("Jira's rate limit is reached; try again in {wait} s"),
            )
            .with_data("retryAfterSeconds", wait.to_string());
        }
        match status {
            401 => fail(
                codes::UNAUTHORIZED,
                format!(
                    "Jira refused the credential `{}` (401); it may be wrong, expired or revoked",
                    self.credential
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

    /// `result` with every appearance of the secret replaced: the whole
    /// credential, the token part of `email:token`, and its Basic
    /// encoding. Results never contain them by construction; this makes
    /// sure.
    pub fn scrub(&self, mut result: ActionResult) -> ActionResult {
        let whole = self.secret.expose();
        let mut secrets = vec![whole.to_owned(), whole.trim().to_owned()];
        if let Some((_, token)) = whole.split_once(':') {
            secrets.push(token.to_owned());
        }
        secrets.push(base64::engine::general_purpose::STANDARD.encode(whole));
        for secret in secrets.iter().filter(|s| s.len() >= 4) {
            if result.message.contains(secret.as_str()) {
                result.message = result.message.replace(secret.as_str(), "[redacted]");
            }
            for value in result.data.values_mut() {
                if value.contains(secret.as_str()) {
                    *value = value.replace(secret.as_str(), "[redacted]");
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_validated() {
        assert_eq!(issue_key(" abc-12 ").unwrap(), "ABC-12");
        assert_eq!(issue_key("A_B2-1").unwrap(), "A_B2-1");
        for bad in [
            "",
            "ABC",
            "ABC-",
            "-1",
            "1A-2",
            "AB C-1",
            "ABC-1x",
            "ABC-1/../x",
        ] {
            assert_eq!(
                issue_key(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_ISSUE_KEY),
                "{bad}"
            );
        }
        assert_eq!(project_key("web").unwrap(), "WEB");
        for bad in ["", "1A", "A B", "A\" OR 1=1", "A-B"] {
            assert!(project_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sites_and_pages() {
        assert_eq!(
            site("https://team.atlassian.net").unwrap().as_str(),
            "https://team.atlassian.net/"
        );
        assert_eq!(
            site("https://intra.example.com/jira").unwrap().as_str(),
            "https://intra.example.com/jira/"
        );
        assert!(site("http://127.0.0.1:8080").is_ok());
        for bad in [
            "",
            "http://team.atlassian.net",
            "https://u:p@team.atlassian.net",
            "https://team.atlassian.net/?x=1",
            "ftp://x",
        ] {
            assert_eq!(
                site(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_SITE_URL),
                "{bad}"
            );
        }
        let jira = site("https://intra.example.com/jira").unwrap();
        assert!(web_url("https://intra.example.com/jira/browse/ABC-1", &jira).is_ok());
        for bad in [
            "http://intra.example.com/jira/browse/ABC-1",
            "https://intra.example.com/other",
            "https://intra.example.com.evil.test/jira/",
            "https://u@intra.example.com/jira/",
            "javascript:alert(1)",
        ] {
            assert_eq!(
                web_url(bad, &jira).unwrap_err().code.as_deref(),
                Some(codes::URL_NOT_ALLOWED),
                "{bad}"
            );
        }
    }

    #[test]
    fn failures_are_explained() {
        assert_eq!(
            explanation(br#"{"errorMessages":["The filter does not exist."],"errors":{}}"#),
            Some("The filter does not exist.".into())
        );
        assert_eq!(
            explanation(br#"{"errorMessages":[],"errors":{"jql":"bad"}}"#),
            Some("bad".into())
        );
        assert_eq!(explanation(b"<html>"), None);
        let headers = vec![("Retry-After".to_owned(), "12".to_owned())];
        assert_eq!(rate_limit_wait(429, &headers), Some(12));
        assert_eq!(rate_limit_wait(429, &[]), Some(60));
        assert_eq!(rate_limit_wait(503, &headers), None);
        // An HTTP date: in the past it is 0, far ahead it is counted.
        let past = vec![(
            "Retry-After".to_owned(),
            "Thu, 01 Jan 2015 00:00:00 GMT".to_owned(),
        )];
        assert_eq!(rate_limit_wait(429, &past), Some(0));
        let ahead = vec![(
            "Retry-After".to_owned(),
            "Fri, 31 Dec 9999 23:59:59 GMT".to_owned(),
        )];
        assert!(rate_limit_wait(429, &ahead).unwrap() > 60);
    }
}
