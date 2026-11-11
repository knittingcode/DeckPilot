//! What a request may do: which URLs and addresses it may reach, which
//! headers a workflow may set, and what is hidden in results. Pure, so it
//! is tested without a network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::codes;

/// The network boundary, from the module's machine-specific settings.
#[derive(Debug, Clone, Default)]
pub struct NetworkPolicy {
    /// Requests may reach this computer and private networks.
    pub allow_private: bool,
    /// Hosts that may resolve to private addresses even when
    /// `allow_private` is off, e.g. `localhost` or `api.internal`. A
    /// leading `*.` matches subdomains.
    pub private_hosts: Vec<String>,
}

impl NetworkPolicy {
    fn host_allowed_private(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.private_hosts.iter().any(|allowed| {
            let allowed = allowed.trim().trim_end_matches('.').to_ascii_lowercase();
            match allowed.strip_prefix("*.") {
                Some(domain) => host.ends_with(&format!(".{domain}")),
                None => host == allowed,
            }
        })
    }

    /// Checks every address `host` resolved to. All must be allowed: a
    /// name that resolves to a public and a private address is refused,
    /// so DNS cannot be used to reach inside.
    pub fn check(&self, host: &str, addresses: &[IpAddr]) -> Result<(), (&'static str, String)> {
        if addresses.is_empty() {
            return Err((codes::DNS_FAILED, format!("{host} did not resolve")));
        }
        if self.allow_private || self.host_allowed_private(host) {
            return Ok(());
        }
        match addresses.iter().find(|a| !is_public(**a)) {
            Some(address) => Err((
                codes::BLOCKED_ADDRESS,
                format!(
                    "{host} is on this computer or a private network ({address}); allow it in \
                     Settings › HTTP"
                ),
            )),
            None => Ok(()),
        }
    }
}

/// Whether an address is on the public internet: not this computer, a
/// private, shared, link-local (including cloud metadata services),
/// multicast, reserved or documentation network.
pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let segments = v6.segments();
            // NAT64 (64:ff9b::/96) embeds an IPv4 address.
            if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [a, b] = segments[6].to_be_bytes();
                let [c, d] = segments[7].to_be_bytes();
                return public_v4(Ipv4Addr::new(a, b, c, d));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fc00::/7 unique local, fe80::/10 link-local.
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                // 2001:db8::/32 documentation.
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                // ::/96 IPv4-compatible (deprecated).
                || segments[..6] == [0; 6]
                || v6 == Ipv6Addr::LOCALHOST)
        }
    }
}

fn public_v4(v4: Ipv4Addr) -> bool {
    let [a, b, c, _] = v4.octets();
    !(v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        || v4.is_multicast()
        || a == 0
        // 100.64.0.0/10 shared address space (carrier NAT).
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24 protocol assignments.
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15 benchmarking.
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4 reserved.
        || a >= 240)
}

/// A request URL: `http` or `https`, with a host, without credentials.
pub fn request_url(text: &str) -> Result<url::Url, (&'static str, String)> {
    let text = text.trim();
    let invalid = |reason: &str| (codes::INVALID_URL, format!("“{}” {reason}", shown(text)));
    if text.len() > 4096 {
        return Err(invalid("is too long"));
    }
    let url = url::Url::parse(text).map_err(|_| invalid("is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("must be an http or https address"));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid("has no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err((
            codes::CREDENTIALS_IN_URL,
            "the URL contains a user name or password; use a stored credential instead".into(),
        ));
    }
    Ok(url)
}

/// A URL as shown in messages and outputs: without query and fragment,
/// which often carry tokens.
pub fn shown_url(url: &url::Url) -> String {
    let mut url = url.clone();
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.to_string()
}

fn shown(text: &str) -> String {
    match url::Url::parse(text) {
        Ok(url) => shown_url(&url),
        Err(_) => text.chars().take(80).collect(),
    }
}

/// Headers whose values are secrets: never set from a workflow, never
/// shown from a response.
pub fn sensitive_header(name: &str) -> bool {
    deckpilot_core::looks_secret(name)
}

/// Headers the client sets itself.
fn managed_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "content-length"
            | "transfer-encoding"
            | "connection"
            | "upgrade"
            | "te"
            | "trailer"
            | "keep-alive"
            | "proxy-connection"
    )
}

/// `Name: value` lines, refusing secrets and headers the client manages.
pub fn headers(text: &str) -> Result<Vec<(String, String)>, (&'static str, String)> {
    let mut headers = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (name, value) = line.split_once(':').ok_or_else(|| {
            (
                codes::INVALID_HEADER,
                format!(
                    "“{}” is not a `Name: value` header",
                    line.chars().take(40).collect::<String>()
                ),
            )
        })?;
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c))
            || value.chars().any(char::is_control)
        {
            return Err((
                codes::INVALID_HEADER,
                format!("the header {name} is not valid"),
            ));
        }
        if sensitive_header(name) {
            return Err((
                codes::SENSITIVE_HEADER,
                format!(
                    "{name} holds a secret and cannot be part of a workflow; use a stored credential"
                ),
            ));
        }
        if managed_header(name) {
            return Err((
                codes::INVALID_HEADER,
                format!("{name} is set automatically"),
            ));
        }
        headers.push((name.to_owned(), value.to_owned()));
    }
    if headers.len() > 50 {
        return Err((
            codes::INVALID_HEADER,
            "at most 50 headers are allowed".into(),
        ));
    }
    Ok(headers)
}

/// `name=value` lines for the query string. Values are encoded; names
/// that look like secrets are refused like headers.
pub fn query(text: &str) -> Result<Vec<(String, String)>, (&'static str, String)> {
    let mut pairs = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (name, value) = line.split_once('=').unwrap_or((line, ""));
        let name = name.trim();
        if name.is_empty() {
            return Err((codes::INVALID_QUERY, format!("“{line}” has no name")));
        }
        if sensitive_header(name) {
            return Err((
                codes::SENSITIVE_HEADER,
                format!("the query parameter {name} looks like a secret; use a stored credential"),
            ));
        }
        pairs.push((name.to_owned(), value.trim().to_owned()));
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn only_public_addresses_are_public() {
        for public in [
            "93.184.216.34",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public(ip(public)), "{public}");
        }
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "192.0.0.8",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::a00:1",
            "2001:db8::1",
        ] {
            assert!(!is_public(ip(private)), "{private}");
        }
    }

    #[test]
    fn private_addresses_need_permission() {
        let strict = NetworkPolicy::default();
        assert!(strict.check("example.com", &[ip("93.184.216.34")]).is_ok());
        let (code, _) = strict.check("localhost", &[ip("127.0.0.1")]).unwrap_err();
        assert_eq!(code, codes::BLOCKED_ADDRESS);
        // A name resolving to both is refused.
        assert!(
            strict
                .check("rebind.example", &[ip("93.184.216.34"), ip("10.0.0.1")])
                .is_err()
        );
        assert_eq!(strict.check("x", &[]).unwrap_err().0, codes::DNS_FAILED);

        let hosts = NetworkPolicy {
            allow_private: false,
            private_hosts: vec!["localhost".into(), "*.internal".into()],
        };
        assert!(hosts.check("LOCALHOST.", &[ip("127.0.0.1")]).is_ok());
        assert!(hosts.check("api.internal", &[ip("10.0.0.5")]).is_ok());
        assert!(hosts.check("internal", &[ip("10.0.0.5")]).is_err());
        assert!(hosts.check("evil.com", &[ip("10.0.0.5")]).is_err());
        let open = NetworkPolicy {
            allow_private: true,
            private_hosts: Vec::new(),
        };
        assert!(open.check("db", &[ip("10.0.0.5")]).is_ok());
    }

    #[test]
    fn urls_are_web_addresses_without_credentials() {
        assert!(request_url("https://api.example.com/v1?x=1").is_ok());
        for (bad, code) in [
            ("ftp://example.com", codes::INVALID_URL),
            ("file:///etc/passwd", codes::INVALID_URL),
            ("not a url", codes::INVALID_URL),
            ("https://user:pw@example.com", codes::CREDENTIALS_IN_URL),
        ] {
            assert_eq!(request_url(bad).unwrap_err().0, code, "{bad}");
        }
        let url = url::Url::parse("https://example.com/a?token=s3cret#x").unwrap();
        assert_eq!(shown_url(&url), "https://example.com/a");
    }

    #[test]
    fn secrets_cannot_be_written_into_headers_or_queries() {
        assert_eq!(
            headers("Accept: application/json\nX-Request-Id: 42").unwrap(),
            [
                ("Accept".to_owned(), "application/json".to_owned()),
                ("X-Request-Id".to_owned(), "42".to_owned())
            ]
        );
        for bad in [
            "Authorization: Bearer x",
            "X-Api-Key: x",
            "Cookie: a=b",
            "X-Auth-Token: x",
        ] {
            assert_eq!(
                headers(bad).unwrap_err().0,
                codes::SENSITIVE_HEADER,
                "{bad}"
            );
        }
        for bad in ["Host: evil", "no colon", "Bad Name: x", "X: a\u{7}b"] {
            assert_eq!(headers(bad).unwrap_err().0, codes::INVALID_HEADER, "{bad}");
        }
        assert_eq!(
            query("page=2\nsort = name\nflag").unwrap(),
            [
                ("page".to_owned(), "2".to_owned()),
                ("sort".to_owned(), "name".to_owned()),
                ("flag".to_owned(), String::new())
            ]
        );
        for bad in ["api_key=x", "access_token=x", "key=x", "password=x"] {
            assert_eq!(query(bad).unwrap_err().0, codes::SENSITIVE_HEADER, "{bad}");
        }
        assert!(sensitive_header("Set-Cookie") && !sensitive_header("Content-Type"));
    }
}
