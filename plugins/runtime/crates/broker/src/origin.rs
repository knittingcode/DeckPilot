//! Strict network origins.
//!
//! A granted origin is exactly `https://<dns-name>[:port]`: no wildcard, no
//! IP literal, no userinfo, no path, query or fragment, no single-label or
//! local-only name. Names are compared after WHATWG URL parsing and IDNA
//! (UTS 46) mapping, i.e. in lowercase ASCII (punycode), so the consent
//! dialog and the broker see the same name.
//!
//! Request URLs go through the same parser. The broker hands the
//! **normalized** URL to the network layer, which must use it instead of
//! the plugin's string, so any parser differential between the check and
//! the request is impossible by construction.
//!
//! **User-entered origins** (G2, [`UserOrigin`]) are parsed by the same
//! rules, plus an optional path prefix. Only they may name a host on a
//! private network (an IP literal of a private range, or a local name such
//! as `ghe.internal` or `gitlab`), and only with the user's separate
//! consent for that origin; `localhost`, loopback, link-local and other
//! special addresses are refused even then.

use std::fmt;
use std::net::IpAddr;

use url::{Host, Url};

/// `https://host:port`, normalized.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Origin {
    host: String,
    port: u16,
}

impl Origin {
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Parses a granted origin (from a manifest/consent record).
    pub fn parse_grant(s: &str) -> Result<Self, OriginError> {
        let url = parse_strict(s)?;
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(OriginError::NotAnOrigin);
        }
        // `https://a.example/` is fine; `https://a.example/x` is not. The
        // URL parser turns an empty path into "/", so reject an explicit
        // path segment by looking at the input as well.
        let after_scheme = s.trim_start_matches("https://");
        if after_scheme.trim_end_matches('/').contains('/') {
            return Err(OriginError::NotAnOrigin);
        }
        origin_of(&url)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.port == 443 {
            write!(f, "https://{}", self.host)
        } else {
            write!(f, "https://{}:{}", self.host, self.port)
        }
    }
}

/// Why a URL or origin was refused. Never contains the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginError {
    Unparsable,
    /// Control characters, whitespace or backslashes in the input.
    IllegalCharacters,
    SchemeNotAllowed,
    UserinfoNotAllowed,
    IpLiteralNotAllowed,
    /// Not a public DNS name (wildcard, single label, `localhost`, `.local` ...).
    InvalidHost,
    /// A grant with a path, query or fragment.
    NotAnOrigin,
    TooLong,
    /// A user origin's path prefix with `.`/`..` segments, empty segments
    /// or encoded slashes.
    InvalidPath,
}

impl fmt::Display for OriginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Unparsable => "unparsable url",
            Self::IllegalCharacters => "illegal characters in url",
            Self::SchemeNotAllowed => "scheme not allowed",
            Self::UserinfoNotAllowed => "userinfo not allowed",
            Self::IpLiteralNotAllowed => "ip literal not allowed",
            Self::InvalidHost => "invalid host",
            Self::NotAnOrigin => "not an origin",
            Self::TooLong => "url too long",
            Self::InvalidPath => "invalid path prefix",
        };
        f.write_str(s)
    }
}

impl std::error::Error for OriginError {}

/// Parses and checks a request URL; returns the normalized URL and origin.
pub fn parse_request_url(s: &str) -> Result<(Url, Origin), OriginError> {
    let url = parse_strict(s)?;
    let origin = origin_of(&url)?;
    Ok((url, origin))
}

/// Parses a request URL whose host is on a private network: an IP literal
/// or a local name ([`parse_request_url`] refuses both). Only a granted
/// private user origin can match the result; for everything else the
/// error of [`parse_request_url`] stands.
pub fn parse_private_request_url(s: &str) -> Result<(Url, Origin), OriginError> {
    let url = parse_strict(s)?;
    let (origin, _) = private_origin_of(&url)?;
    Ok((url, origin))
}

/// Longest path prefix of a user origin.
pub const MAX_PATH_PREFIX_LEN: usize = 512;

/// An origin the user entered in a plugin setting (G2): exactly
/// `https://host[:port]`, optionally with a path prefix. Requests match it
/// only with the same scheme, host and port, and a path that is the prefix
/// or below it (`/api/v3`, `/api/v3/…`, never `/api/v30`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserOrigin {
    origin: Origin,
    /// Normalized, starting with `/`, without a trailing `/`.
    path_prefix: Option<String>,
    /// The host is an IP literal or a local name: reachable only with the
    /// user's private-network consent for this origin.
    private_host: bool,
}

impl UserOrigin {
    /// Parses what the user entered: `https://`, no userinfo, query or
    /// fragment, a public DNS name or (as a private host) an IP literal of
    /// a public or private range or a local name; a path is kept as the
    /// prefix (normalized, without a trailing `/`).
    pub fn parse(s: &str) -> Result<Self, OriginError> {
        let url = parse_strict(s)?;
        if url.query().is_some() || url.fragment().is_some() {
            return Err(OriginError::NotAnOrigin);
        }
        let (origin, public) = match origin_of(&url) {
            Ok(origin) => (origin, true),
            Err(OriginError::IpLiteralNotAllowed | OriginError::InvalidHost) => {
                (private_origin_of(&url)?.0, false)
            }
            Err(e) => return Err(e),
        };
        let path_prefix = path_prefix(s, &url)?;
        Ok(Self {
            origin,
            path_prefix,
            private_host: !public,
        })
    }

    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    pub fn path_prefix(&self) -> Option<&str> {
        self.path_prefix.as_deref()
    }

    /// Whether the host itself is on a private network (an IP literal or
    /// a local name). A public name may still resolve to a private
    /// address; that also needs the private-network consent, which the
    /// connection check enforces.
    pub fn private_host(&self) -> bool {
        self.private_host
    }

    /// Whether a request to `origin` with the normalized `url` is within
    /// this user origin.
    pub fn matches(&self, origin: &Origin, url: &Url) -> bool {
        if *origin != self.origin {
            return false;
        }
        match &self.path_prefix {
            None => true,
            Some(prefix) => {
                let path = url.path();
                path == prefix
                    || path
                        .strip_prefix(prefix.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            }
        }
    }
}

impl fmt::Display for UserOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.origin,
            self.path_prefix.as_deref().unwrap_or("")
        )
    }
}

/// The path prefix of a user origin: `None` for `/`, otherwise the
/// normalized path without a trailing `/`. Refused: `.` and `..` segments
/// (the parser would resolve them, so the user would consent to a path
/// they did not type), empty segments, and encoded `/`, `\` or `.`.
fn path_prefix(input: &str, url: &Url) -> Result<Option<String>, OriginError> {
    let after_scheme = &input[8..];
    let raw = after_scheme
        .find('/')
        .map_or("", |start| &after_scheme[start..]);
    let raw = raw.trim_end_matches('/');
    if raw.is_empty() {
        return Ok(None);
    }
    let lowered = raw.to_ascii_lowercase();
    let bad = raw
        .split('/')
        .skip(1)
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || ["%2f", "%5c", "%2e"].iter().any(|e| lowered.contains(e));
    if bad {
        return Err(OriginError::InvalidPath);
    }
    let path = url.path().trim_end_matches('/');
    if path.len() > MAX_PATH_PREFIX_LEN {
        return Err(OriginError::TooLong);
    }
    Ok(Some(path.to_owned()))
}

/// The origin of a URL whose host is an IP literal of a reachable range
/// or a local name; `bool` is whether it is an IP literal.
fn private_origin_of(url: &Url) -> Result<(Origin, bool), OriginError> {
    let (host, literal) = match url.host() {
        Some(Host::Domain(d)) => {
            check_local_name(d)?;
            (d.to_owned(), false)
        }
        Some(Host::Ipv4(ip)) => {
            crate::addr::check_private_network_addr(IpAddr::V4(ip))
                .map_err(|_| OriginError::IpLiteralNotAllowed)?;
            (ip.to_string(), true)
        }
        Some(Host::Ipv6(ip)) => {
            crate::addr::check_private_network_addr(IpAddr::V6(ip))
                .map_err(|_| OriginError::IpLiteralNotAllowed)?;
            (format!("[{ip}]"), true)
        }
        None => return Err(OriginError::InvalidHost),
    };
    let port = url.port_or_known_default().ok_or(OriginError::Unparsable)?;
    if port == 0 {
        return Err(OriginError::InvalidHost);
    }
    Ok((Origin { host, port }, literal))
}

/// Local names a private user origin may use: DNS label syntax, one label
/// or more, but never this computer (`localhost`) and never a name that
/// cannot resolve (`.invalid`, `.onion`).
fn check_local_name(host: &str) -> Result<(), OriginError> {
    if host.is_empty() || host.len() > 253 || host.ends_with('.') {
        return Err(OriginError::InvalidHost);
    }
    for label in host.split('.') {
        let b = label.as_bytes();
        let ok = !b.is_empty()
            && b.len() <= 63
            && b[0] != b'-'
            && b[b.len() - 1] != b'-'
            && b.iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-');
        if !ok {
            return Err(OriginError::InvalidHost);
        }
    }
    if host
        .rsplit('.')
        .next()
        .is_some_and(|tld| tld.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err(OriginError::InvalidHost);
    }
    for never in ["localhost", "invalid", "onion"] {
        if host == never || host.ends_with(&format!(".{never}")) {
            return Err(OriginError::InvalidHost);
        }
    }
    Ok(())
}

fn parse_strict(s: &str) -> Result<Url, OriginError> {
    if s.len() > deckpilot_plugin_abi::limits::MAX_URL_LEN {
        return Err(OriginError::TooLong);
    }
    // The WHATWG parser silently strips tabs and newlines and treats `\` as
    // `/`; refuse such input instead of guessing what the author meant.
    if s.chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Err(OriginError::IllegalCharacters);
    }
    // Scheme check on the raw input first: `HTTPS:` is fine (the parser
    // lowercases it), anything else is refused before parsing.
    let scheme_ok = s
        .get(..8)
        .is_some_and(|p| p.eq_ignore_ascii_case("https://"));
    if !scheme_ok {
        return Err(OriginError::SchemeNotAllowed);
    }
    let url = Url::parse(s).map_err(|_| OriginError::Unparsable)?;
    if url.scheme() != "https" {
        return Err(OriginError::SchemeNotAllowed);
    }
    let authority = s[8..].split(['/', '?', '#']).next().unwrap_or("");
    if !url.username().is_empty() || url.password().is_some() || authority.contains('@') {
        return Err(OriginError::UserinfoNotAllowed);
    }
    Ok(url)
}

fn origin_of(url: &Url) -> Result<Origin, OriginError> {
    let host = match url.host() {
        Some(Host::Domain(d)) => d,
        Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => return Err(OriginError::IpLiteralNotAllowed),
        None => return Err(OriginError::InvalidHost),
    };
    check_dns_name(host)?;
    let port = url.port_or_known_default().ok_or(OriginError::Unparsable)?;
    if port == 0 {
        return Err(OriginError::InvalidHost);
    }
    Ok(Origin {
        host: host.to_owned(),
        port,
    })
}

/// Names that never resolve to a public service.
const LOCAL_SUFFIXES: &[&str] = &[
    "localhost",
    "local",
    "internal",
    "intranet",
    "lan",
    "home",
    "corp",
    "home.arpa",
    "arpa",
    "localdomain",
    "test",
    "invalid",
    "example",
    "onion",
];

fn check_dns_name(host: &str) -> Result<(), OriginError> {
    // After IDNA the host is lowercase ASCII; anything else is refused.
    if host.is_empty() || host.len() > 253 || host.ends_with('.') {
        return Err(OriginError::InvalidHost);
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return Err(OriginError::InvalidHost);
    }
    for label in &labels {
        let b = label.as_bytes();
        let ok = !b.is_empty()
            && b.len() <= 63
            && b[0] != b'-'
            && b[b.len() - 1] != b'-'
            && b.iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-');
        if !ok {
            return Err(OriginError::InvalidHost);
        }
    }
    // An all-numeric last label is never a TLD (and catches IPv4 forms the
    // URL parser might not have turned into an address).
    if labels[labels.len() - 1].bytes().all(|c| c.is_ascii_digit()) {
        return Err(OriginError::InvalidHost);
    }
    for suffix in LOCAL_SUFFIXES {
        if host == *suffix || host.ends_with(&format!(".{suffix}")) {
            return Err(OriginError::InvalidHost);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_are_exact_origins() {
        let o = Origin::parse_grant("https://API.Example.com").unwrap();
        assert_eq!(o.to_string(), "https://api.example.com");
        assert_eq!(
            Origin::parse_grant("https://api.example.com:443/").unwrap(),
            o
        );
        assert_eq!(
            Origin::parse_grant("https://api.example.com:8443")
                .unwrap()
                .to_string(),
            "https://api.example.com:8443"
        );
        assert_eq!(
            Origin::parse_grant("https://bücher.example.org")
                .unwrap()
                .host(),
            "xn--bcher-kva.example.org"
        );
        for (bad, why) in [
            ("https://*.example.com", OriginError::InvalidHost),
            ("https://*", OriginError::InvalidHost),
            ("*", OriginError::SchemeNotAllowed),
            ("https://example.com/api", OriginError::NotAnOrigin),
            ("https://example.com/?q", OriginError::NotAnOrigin),
            ("https://example.com/#f", OriginError::NotAnOrigin),
            ("http://example.com", OriginError::SchemeNotAllowed),
            ("wss://example.com", OriginError::SchemeNotAllowed),
            ("https://user@example.com", OriginError::UserinfoNotAllowed),
            ("https://1.2.3.4", OriginError::IpLiteralNotAllowed),
            ("https://[::1]", OriginError::IpLiteralNotAllowed),
            ("https://localhost", OriginError::InvalidHost),
            ("https://intranet", OriginError::InvalidHost),
            ("https://printer.local", OriginError::InvalidHost),
            ("https://example.com.", OriginError::InvalidHost),
            ("https://example.com:0", OriginError::InvalidHost),
            ("https://exa mple.com", OriginError::IllegalCharacters),
            ("https://example.com:99999", OriginError::Unparsable),
        ] {
            assert_eq!(Origin::parse_grant(bad), Err(why), "{bad}");
        }
    }

    #[test]
    fn user_origins_are_exact_with_an_optional_path_prefix() {
        let o = UserOrigin::parse("https://GHE.Example.com/api/v3/").unwrap();
        assert_eq!(o.to_string(), "https://ghe.example.com/api/v3");
        assert_eq!(o.path_prefix(), Some("/api/v3"));
        assert!(!o.private_host());
        let plain = UserOrigin::parse("https://ghe.example.com:8443").unwrap();
        assert_eq!(plain.to_string(), "https://ghe.example.com:8443");
        assert_eq!(plain.path_prefix(), None);
        let url = |s: &str| parse_request_url(s).unwrap();
        for (inside, request) in [
            (true, "https://ghe.example.com/api/v3"),
            (true, "https://ghe.example.com/api/v3/repos?x=1"),
            (false, "https://ghe.example.com/api/v30"),
            (false, "https://ghe.example.com/api"),
            (false, "https://ghe.example.com/"),
            (false, "https://ghe.example.com/api/v3/../../admin"),
            (false, "https://ghe.example.com/api/v3%2F..%2Fadmin"),
            (false, "https://ghe.example.com:444/api/v3"),
            (false, "https://ghe2.example.com/api/v3"),
            (false, "https://x.ghe.example.com/api/v3"),
        ] {
            let (u, origin) = url(request);
            assert_eq!(o.matches(&origin, &u), inside, "{request}");
        }
        for (bad, why) in [
            ("http://ghe.example.com", OriginError::SchemeNotAllowed),
            (
                "https://u:p@ghe.example.com",
                OriginError::UserinfoNotAllowed,
            ),
            ("https://ghe.example.com/?q=1", OriginError::NotAnOrigin),
            ("https://ghe.example.com/#x", OriginError::NotAnOrigin),
            ("https://*.example.com", OriginError::InvalidHost),
            ("https://ghe.example.com/a/../b", OriginError::InvalidPath),
            ("https://ghe.example.com/a//b", OriginError::InvalidPath),
            ("https://ghe.example.com/a%2Fb", OriginError::InvalidPath),
            ("https://ghe.example.com/%2e%2e/b", OriginError::InvalidPath),
            ("https://localhost", OriginError::InvalidHost),
            ("https://db.localhost", OriginError::InvalidHost),
            ("https://127.0.0.1", OriginError::IpLiteralNotAllowed),
            ("https://[::1]", OriginError::IpLiteralNotAllowed),
            ("https://169.254.169.254", OriginError::IpLiteralNotAllowed),
            ("https://0.0.0.0", OriginError::IpLiteralNotAllowed),
            ("https://ghe .example.com", OriginError::IllegalCharacters),
        ] {
            assert_eq!(UserOrigin::parse(bad), Err(why), "{bad}");
        }
    }

    #[test]
    fn private_hosts_are_marked_and_only_reachable_ranges_parse() {
        for (text, shown) in [
            ("https://10.0.0.5", "https://10.0.0.5"),
            (
                "https://192.168.1.20:8443/gitlab",
                "https://192.168.1.20:8443/gitlab",
            ),
            ("https://[fd00::7]", "https://[fd00::7]"),
            ("https://ghe.internal", "https://ghe.internal"),
            ("https://gitlab", "https://gitlab"),
            ("https://jira.corp", "https://jira.corp"),
        ] {
            let o = UserOrigin::parse(text).unwrap();
            assert!(o.private_host(), "{text}");
            assert_eq!(o.to_string(), shown);
        }
        let (url, origin) = parse_private_request_url("https://10.0.0.5/api").unwrap();
        assert!(
            UserOrigin::parse("https://10.0.0.5")
                .unwrap()
                .matches(&origin, &url)
        );
        // A public IP literal is still not a public origin.
        assert!(UserOrigin::parse("https://8.8.8.8").unwrap().private_host());
        assert_eq!(
            parse_request_url("https://10.0.0.5/api"),
            Err(OriginError::IpLiteralNotAllowed)
        );
    }
}
