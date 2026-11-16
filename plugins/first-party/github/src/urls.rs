//! The few URLs the plugin builds and checks, without a general URL
//! parser: the `url` crate (with its IDNA tables) would more than double
//! the module, which ships inside every installer
//! (`docs/marketplace/module-migration.md` §8). Every URL here is on one of
//! two fixed origins, and the agent's broker parses and checks each one
//! again before anything is sent or opened.
//!
//! The unit tests compare these functions with the `url` crate (a
//! dev-dependency only), which the built-in module uses.

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

/// The API's base, with the path's first slash.
pub const API_BASE: &str = "https://api.github.com/";
/// The web origin, with the path's first slash.
pub const WEB_BASE: &str = "https://github.com/";

/// WHATWG percent-encode sets, as the `url` crate defines them.
const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');
const PATH: &AsciiSet = &FRAGMENT.add(b'#').add(b'?').add(b'{').add(b'}');
/// A path segment of a special URL (`https`): `/`, `%` and `\` too.
const PATH_SEGMENT: &AsciiSet = &PATH.add(b'/').add(b'%').add(b'\\');
const SPECIAL_QUERY: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'\'');

/// An API URL: these path segments, each encoded, and this query
/// (`application/x-www-form-urlencoded`), as the built-in builds it with
/// `path_segments_mut().extend` and `query_pairs_mut().extend_pairs`.
pub fn endpoint(segments: &[&str], query: &[(&str, &str)]) -> String {
    let path: Vec<String> = segments
        .iter()
        .filter(|s| !matches!(**s, "." | ".."))
        .map(|s| utf8_percent_encode(s, PATH_SEGMENT).to_string())
        .collect();
    let mut url = format!("{API_BASE}{}", path.join("/"));
    if !query.is_empty() {
        url.push('?');
        url.push_str(
            &form_urlencoded::Serializer::new(String::new())
                .extend_pairs(query)
                .finish(),
        );
    }
    url
}

/// Whether `link` (GitHub's next page) is on the API: `https`, the host
/// `api.github.com` on the default port, no user name or password.
pub fn on_api(link: &str) -> bool {
    let link = link.trim();
    link.len() >= API_BASE.len()
        && link.is_char_boundary(API_BASE.len())
        && link[..API_BASE.len()].eq_ignore_ascii_case(API_BASE)
        && !link
            .chars()
            .any(|c| c.is_ascii_control() || c.is_whitespace())
}

/// Whether the API URL setting is `https://api.github.com`.
pub fn is_api(text: &str) -> bool {
    let text = text.trim();
    let text = text.strip_suffix('/').unwrap_or(text);
    text.eq_ignore_ascii_case(API_BASE.trim_end_matches('/'))
}

/// A GitHub page to open, normalized as a browser would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPage {
    /// The page with its query and fragment: what is opened.
    pub url: String,
    /// The page without query and fragment (which often carry tokens):
    /// what messages and outputs show.
    pub shown: String,
}

/// `text` as an https page on `github.com` (default port, no user name or
/// password), normalized like the WHATWG URL parser does for these: the
/// scheme and host in lower case, the default port dropped, `.` and `..`
/// path segments resolved, and characters percent-encoded where a URL
/// needs it. `None` for anything else.
pub fn web_page(text: &str) -> Option<WebPage> {
    // The URL parser drops tabs and newlines anywhere, and leading and
    // trailing spaces and control characters.
    let text: String = text
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let (scheme, rest) = text.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    // A special URL ignores any number of slashes and backslashes here.
    let rest = rest.trim_start_matches(['/', '\\']);
    let end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let (authority, rest) = rest.split_at(end);
    let host_port = match authority.rsplit_once('@') {
        // An empty user name and password are dropped; any other is
        // refused.
        Some(("", host_port)) => host_port,
        Some(_) => return None,
        None => authority,
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_port, None),
    };
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    match port {
        None | Some("") => {}
        Some(port) => {
            if !port.bytes().all(|b| b.is_ascii_digit()) || port.trim_start_matches('0') != "443" {
                return None;
            }
        }
    }
    let (rest, fragment) = match rest.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (rest, None),
    };
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    let shown = format!("https://github.com{}", normalized_path(path));
    let mut url = shown.clone();
    if let Some(query) = query {
        url.push('?');
        url.push_str(&utf8_percent_encode(query, SPECIAL_QUERY).to_string());
    }
    if let Some(fragment) = fragment {
        url.push('#');
        url.push_str(&utf8_percent_encode(fragment, FRAGMENT).to_string());
    }
    Some(WebPage { url, shown })
}

fn single_dot(segment: &str) -> bool {
    segment == "." || segment.eq_ignore_ascii_case("%2e")
}

fn double_dot(segment: &str) -> bool {
    matches!(
        segment.to_ascii_lowercase().as_str(),
        ".." | ".%2e" | "%2e." | "%2e%2e"
    )
}

/// A special URL's path: backslashes are slashes, dot segments resolved,
/// each segment percent-encoded (an existing `%` stays).
fn normalized_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix('/').unwrap_or(&path);
    let parts: Vec<&str> = path.split('/').collect();
    let mut out: Vec<String> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        if double_dot(part) {
            out.pop();
            if last {
                out.push(String::new());
            }
        } else if single_dot(part) {
            if last {
                out.push(String::new());
            }
        } else {
            out.push(utf8_percent_encode(part, PATH).to_string());
        }
    }
    format!("/{}", out.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in module's way: `url::Url` from the API base.
    fn builtin_endpoint(segments: &[&str], query: &[(&str, &str)]) -> String {
        let mut url = url::Url::parse("https://api.github.com").unwrap();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty();
            path.extend(segments);
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url.to_string()
    }

    #[test]
    fn endpoints_are_built_as_the_builtin_builds_them() {
        let refs = [
            "main",
            "feature/login",
            "v1.2.3",
            "a#b",
            "a%b",
            "a<b>c",
            "a\"b`c",
            "naïve",
            "a'b",
            "a;b=c&d",
            "a|b",
        ];
        for reference in refs {
            let segments = [
                "repos", "octo-org", "site.web", "commits", reference, "status",
            ];
            let query = [("per_page", "50"), ("filter", "latest")];
            assert_eq!(
                endpoint(&segments, &query),
                builtin_endpoint(&segments, &query),
                "{reference}"
            );
        }
        let query = [
            ("q", "is:pr is:open repo:octo/site review-requested:@me"),
            ("sort", "updated"),
            ("order", "desc"),
            ("per_page", "20"),
        ];
        assert_eq!(
            endpoint(&["search", "issues"], &query),
            builtin_endpoint(&["search", "issues"], &query)
        );
        assert_eq!(endpoint(&["user"], &[]), builtin_endpoint(&["user"], &[]));
        assert_eq!(endpoint(&["user"], &[]), "https://api.github.com/user");
    }

    #[test]
    fn only_api_links_are_followed() {
        assert!(on_api(
            "https://api.github.com/repositories/1/commits/main/check-runs?page=2"
        ));
        assert!(on_api("HTTPS://API.GITHUB.COM/x"));
        for bad in [
            "http://api.github.com/x",
            "https://api.github.com.evil.com/x",
            "https://evil.com/api.github.com/x",
            "https://user@api.github.com/x",
            "https://api.github.com:8443/x",
            "https://api.github.com",
            "https://api.github.com/x y",
            "/repositories/1",
        ] {
            assert!(!on_api(bad), "{bad}");
        }
    }

    #[test]
    fn the_api_setting_is_the_public_api() {
        for good in [
            "https://api.github.com",
            "https://api.github.com/",
            " HTTPS://API.GITHUB.COM ",
        ] {
            assert!(is_api(good), "{good}");
        }
        for bad in [
            "",
            "http://api.github.com",
            "https://ghe.example.com/api/v3",
            "https://api.octo.ghe.com",
            "https://api.github.com/api/v3",
            "https://api.github.com:8443",
            "https://api.github.com?x=1",
        ] {
            assert!(!is_api(bad), "{bad}");
        }
    }

    /// The built-in module's check (`web_url` with the web base
    /// `https://github.com/`), and what it opens and shows.
    fn builtin_page(text: &str) -> Option<WebPage> {
        let web = url::Url::parse("https://github.com/").unwrap();
        let url = url::Url::parse(text.trim()).ok()?;
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
            return None;
        }
        let mut shown = url.clone();
        shown.set_query(None);
        shown.set_fragment(None);
        Some(WebPage {
            url: url.to_string(),
            shown: shown.to_string(),
        })
    }

    #[test]
    fn pages_are_checked_and_normalized_as_the_builtin_does() {
        let cases = [
            "https://github.com/octo/site/pull/12",
            "https://github.com/octo/site/pull/12?tab=files&token=q-s3cret#diff-1",
            "https://GitHub.com/octo/site",
            "HTTPS://github.com",
            "https://github.com",
            "https://github.com/",
            "https://github.com:443/a",
            "https://github.com:/a",
            "https://github.com/a/../b",
            "https://github.com/a/./b/.",
            "https://github.com/a/%2e%2E/b",
            "https://github.com/a/b/..",
            "https://github.com/a b/c\"d<e>",
            "https://github.com/naïve/ü?q=ä#ö",
            "https://github.com/a?x='y'",
            "https://github.com/a#b'c d",
            "https://github.com/a%20b",
            "https://github.com/a/%2E",
            "https://github.com/../a",
            "https://github.com?x",
            "https://github.com#f",
            "https://github.com:0443/a",
            "https://github.com:443:443/a",
            "https://github.com/a//b",
            "https://github.com/a/b?c=d?e#f#g",
            "https://github.com/{x}|`y`",
            "https:/github.com/a",
            "https:github.com/a",
            "https:\\\\github.com\\a\\b",
            "https:///github.com/a",
            "  https://github.com/a\t/b\n  ",
            "https://@github.com/a",
            "http://github.com/a/b",
            "https://github.com.evil.com/a",
            "https://evil.com/github.com",
            "https://user@github.com/a",
            "https://user:pw@github.com/a",
            "https://github.com:8443/a",
            "https://gist.github.com/a",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "github.com/a",
            "",
        ];
        for case in cases {
            assert_eq!(web_page(case), builtin_page(case), "{case:?}");
        }
    }
}
