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

use std::fmt;

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
}
