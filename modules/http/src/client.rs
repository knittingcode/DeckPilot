//! Sending one request: resolving and checking every address against the
//! [`NetworkPolicy`], connecting only to the checked addresses (so DNS
//! cannot change between the check and the connection), following
//! redirects one hop at a time with the same checks, reading a bounded
//! body, and stopping when cancelled.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use deckpilot_core::{CancelReason, CancellationToken, Secret};

use crate::codes;
use crate::policy::{self, NetworkPolicy};

/// Most redirects followed.
pub const MAX_REDIRECTS: u32 = 5;

/// How a stored credential is sent.
#[derive(Debug, Clone)]
pub enum Auth {
    /// `Authorization: Bearer <secret>`.
    Bearer(Secret),
    /// `Authorization: Basic …`; the secret is `user:password`.
    Basic(Secret),
    /// A header of its own, e.g. `X-API-Key`.
    Header(String, Secret),
    /// A query parameter, e.g. `api_key`.
    Query(String, Secret),
}

#[derive(Debug, Clone)]
pub struct Request {
    pub method: reqwest::Method,
    pub url: url::Url,
    pub headers: Vec<(String, String)>,
    /// Body and its content type.
    pub body: Option<(String, &'static str)>,
    pub auth: Option<Auth>,
    pub timeout: Duration,
    pub follow_redirects: bool,
    pub max_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    /// `name: value`, with secrets (cookies, tokens) replaced.
    pub headers: Vec<(String, String)>,
    pub content_type: String,
    pub body: Vec<u8>,
    pub truncated: bool,
    pub final_url: url::Url,
    pub redirects: u32,
    pub elapsed: Duration,
}

/// A failure: a stable code and a message without secrets or queries.
pub type Failure = (&'static str, String);

/// Sends `request`. Blocks the calling thread (an action's own thread);
/// returns at once when `cancel` is cancelled.
pub fn send(
    request: Request,
    network: &NetworkPolicy,
    cancel: Option<&CancellationToken>,
) -> Result<Response, Failure> {
    // The TLS stack's cryptography; a no-op once installed. Without it the
    // client would panic, so it is ensured before every client is built.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            (
                codes::REQUEST_FAILED,
                format!("could not start the request: {e}"),
            )
        })?;
    let timeout = request.timeout;
    let begun = Instant::now();
    runtime.block_on(async {
        tokio::select! {
            result = run(request, network, begun) => result,
            reason = cancelled(cancel) => Err(match reason {
                CancelReason::Cancelled => ("cancelled", "cancelled; the request was stopped".to_owned()),
                CancelReason::DeadlineExceeded => ("timed-out", "the node's time limit passed".to_owned()),
            }),
            () = tokio::time::sleep(timeout) => Err((
                "timed-out",
                format!("no answer within {} s", timeout.as_secs()),
            )),
        }
    })
}

async fn cancelled(token: Option<&CancellationToken>) -> CancelReason {
    let Some(token) = token else {
        return std::future::pending().await;
    };
    loop {
        if let Some(reason) = token.reason() {
            return reason;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn addresses(url: &url::Url) -> Result<(String, Vec<IpAddr>, u16), Failure> {
    let host = url.host_str().unwrap_or_default().to_owned();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| (codes::INVALID_URL, "the URL has no port".to_owned()))?;
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<IpAddr>() {
        return Ok((host, vec![ip], port));
    }
    let resolved = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| {
            (
                codes::DNS_FAILED,
                format!("{host} could not be resolved: {e}"),
            )
        })?;
    let mut ips: Vec<IpAddr> = resolved.map(|a| a.ip()).collect();
    ips.sort_unstable();
    ips.dedup();
    Ok((host, ips, port))
}

fn same_origin(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// The error's message and its sources, without the URL.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        let part = inner.to_string();
        if !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = inner.source();
    }
    text
}

fn owned_failure(error: reqwest::Error) -> Failure {
    let timeout = error.is_timeout();
    let connect = error.is_connect();
    let error = error.without_url();
    let text = error_chain(&error);
    if timeout {
        ("timed-out", "the server did not answer in time".to_owned())
    } else if text.to_lowercase().contains("certificate")
        || text.contains("TLS")
        || text.contains("tls handshake")
    {
        (
            codes::TLS_FAILED,
            format!("the secure connection failed: {text}"),
        )
    } else if connect {
        (
            codes::CONNECTION_FAILED,
            format!("could not connect: {text}"),
        )
    } else {
        (codes::REQUEST_FAILED, text)
    }
}

/// One hop: checks the URL's addresses and sends the request to exactly
/// those addresses.
async fn hop(
    request: &Request,
    network: &NetworkPolicy,
    url: &url::Url,
    method: &reqwest::Method,
    body: Option<&(String, &'static str)>,
    auth: Option<&Auth>,
) -> Result<reqwest::Response, Failure> {
    let (host, ips, port) = addresses(url).await?;
    network.check(&host, &ips)?;
    if auth.is_some() && url.scheme() != "https" && !ips.iter().all(IpAddr::is_loopback) {
        return Err((
            codes::INSECURE_CREDENTIALS,
            format!(
                "credentials are only sent over https; {} is not",
                policy::shown_url(url)
            ),
        ));
    }
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("DeckPilot/", env!("CARGO_PKG_VERSION")));
    if host.parse::<IpAddr>().is_err() && !host.starts_with('[') {
        let pinned: Vec<SocketAddr> = ips.iter().map(|ip| SocketAddr::new(*ip, port)).collect();
        builder = builder.resolve_to_addrs(&host, &pinned);
    }
    let client = builder
        .build()
        .map_err(|e| (codes::REQUEST_FAILED, error_chain(&e.without_url())))?;
    let mut target = url.clone();
    if let Some(Auth::Query(name, secret)) = auth {
        target.query_pairs_mut().append_pair(name, secret.expose());
    }
    let mut builder = client.request(method.clone(), target);
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder = match auth {
        Some(Auth::Bearer(secret)) => builder.bearer_auth(secret.expose()),
        Some(Auth::Basic(secret)) => {
            let (user, password) = secret
                .expose()
                .split_once(':')
                .unwrap_or((secret.expose(), ""));
            builder.basic_auth(user, Some(password))
        }
        Some(Auth::Header(name, secret)) => builder.header(name.as_str(), secret.expose()),
        Some(Auth::Query(..)) | None => builder,
    };
    if let Some((text, content_type)) = body {
        builder = builder
            .header(reqwest::header::CONTENT_TYPE, *content_type)
            .body(text.clone());
    }
    builder.send().await.map_err(owned_failure)
}

/// Where a redirect leads, checked like a new request: never from https to
/// http.
fn redirect_target(response: &reqwest::Response, url: &url::Url) -> Result<url::Url, Failure> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                codes::REQUEST_FAILED,
                "a redirect without a location".to_owned(),
            )
        })?;
    let next = url.join(location).map_err(|_| {
        (
            codes::INVALID_URL,
            "the redirect location is not a valid URL".to_owned(),
        )
    })?;
    let next = policy::request_url(next.as_str())?;
    if url.scheme() == "https" && next.scheme() == "http" {
        return Err((
            codes::INSECURE_REDIRECT,
            format!(
                "refused a redirect from https to {}",
                policy::shown_url(&next)
            ),
        ));
    }
    Ok(next)
}

/// The response's headers (secrets replaced), content type and body, up
/// to `max_bytes`.
async fn read(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<(Vec<(String, String)>, String, Vec<u8>, bool), Failure> {
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            let value = if policy::sensitive_header(name.as_str()) {
                "[redacted]".to_owned()
            } else {
                value.to_str().unwrap_or("[binary]").to_owned()
            };
            (name.as_str().to_owned(), value)
        })
        .collect();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let mut collected = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = response.chunk().await.map_err(owned_failure)? {
        let room = max_bytes.saturating_sub(collected.len());
        if chunk.len() > room {
            collected.extend_from_slice(&chunk[..room]);
            truncated = true;
            break;
        }
        collected.extend_from_slice(&chunk);
    }
    Ok((headers, content_type, collected, truncated))
}

async fn run(
    request: Request,
    network: &NetworkPolicy,
    begun: Instant,
) -> Result<Response, Failure> {
    let original = request.url.clone();
    let mut url = request.url.clone();
    let mut method = request.method.clone();
    let mut body = request.body.clone();
    let mut auth = request.auth.clone();
    let mut redirects = 0;
    loop {
        let response = hop(
            &request,
            network,
            &url,
            &method,
            body.as_ref(),
            auth.as_ref(),
        )
        .await?;
        let status = response.status();
        if request.follow_redirects
            && status.is_redirection()
            && status != reqwest::StatusCode::NOT_MODIFIED
        {
            if redirects >= MAX_REDIRECTS {
                return Err((
                    codes::TOO_MANY_REDIRECTS,
                    format!("more than {MAX_REDIRECTS} redirects"),
                ));
            }
            let next = redirect_target(&response, &url)?;
            // A credential is for the server it was meant for.
            if !same_origin(&original, &next) {
                auth = None;
            }
            // 303, and 301/302 after a POST, continue as GET without body.
            if status == reqwest::StatusCode::SEE_OTHER
                || ((status == reqwest::StatusCode::MOVED_PERMANENTLY
                    || status == reqwest::StatusCode::FOUND)
                    && method == reqwest::Method::POST)
            {
                method = reqwest::Method::GET;
                body = None;
            }
            url = next;
            redirects += 1;
            continue;
        }
        let (headers, content_type, body, truncated) = read(response, request.max_bytes).await?;
        return Ok(Response {
            status: status.as_u16(),
            headers,
            content_type,
            body,
            truncated,
            final_url: url,
            redirects,
            elapsed: begun.elapsed(),
        });
    }
}

/// Seconds to wait from a `Retry-After` value: a number of seconds, or an
/// HTTP date (`Sun, 06 Nov 1994 08:49:37 GMT`) counted from `now` (Unix
/// seconds; a date in the past is 0). `None` when it is neither.
pub fn retry_after(value: &str, now: u64) -> Option<u64> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    http_date(value).map(|at| at.saturating_sub(now))
}

/// An IMF-fixdate (RFC 9110 §5.6.7) as Unix seconds.
fn http_date(value: &str) -> Option<u64> {
    // "Sun, 06 Nov 1994 08:49:37 GMT"
    let (_, rest) = value.split_once(", ")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_is_seconds_or_a_date() {
        assert_eq!(retry_after(" 120 ", 0), Some(120));
        assert_eq!(
            http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            http_date("Thu, 29 Feb 2024 12:00:00 GMT"),
            Some(1_709_208_000)
        );
        assert_eq!(
            http_date("Wed, 12 Nov 2026 07:00:00 GMT"),
            Some(1_794_466_800)
        );
        let now = 1_794_466_800;
        assert_eq!(retry_after("Wed, 12 Nov 2026 07:01:30 GMT", now), Some(90));
        assert_eq!(retry_after("Wed, 12 Nov 2026 06:00:00 GMT", now), Some(0));
        for bad in [
            "",
            "-5",
            "soon",
            "Wed, 12 Nov 2026 07:01:30 UTC",
            "Wed, 32 Nov 2026 07:01:30 GMT",
            "Wed, 12 Foo 2026 07:01:30 GMT",
            "Wed, 12 Nov 2026 25:01:30 GMT",
            "Wednesday, 12-Nov-26 07:01:30 GMT",
            "Wed Nov 12 07:01:30 2026",
        ] {
            assert_eq!(retry_after(bad, now), None, "{bad}");
        }
    }
}
