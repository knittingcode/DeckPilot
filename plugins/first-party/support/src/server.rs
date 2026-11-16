//! The few URLs a plugin for a user-entered server builds and checks,
//! without a general URL parser: the `url` crate (with its IDNA tables)
//! would more than double a module, which ships inside every installer
//! (`docs/marketplace/module-migration.md` §8).
//!
//! The server is what the user entered in the plugin's origin setting. The
//! agent normalized it when it was saved (`https://host[:port][/path]`,
//! lower case, punycode, no default port, no trailing slash,
//! `docs/marketplace/sdk/permissions.md`), and its broker parses and checks
//! every URL again before anything is sent or opened: nothing here is a
//! security boundary. [`Server::parse`] still refuses anything that is not
//! of that form, so a plugin never builds a URL from something else.
//!
//! The unit tests compare these functions with the `url` crate (a
//! dev-dependency only), which the built-in modules use.

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

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

/// An `https` server, optionally with a path: an API's base
/// (`https://code.example.com/gitlab/api/v4`) or a site
/// (`https://jira.example.com/jira`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// Lower case; an IPv6 literal in brackets.
    host: String,
    /// `None` for 443.
    port: Option<u16>,
    /// Empty, or starting with `/` and not ending with one.
    path: String,
}

/// A page to open, normalized as a browser would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPage {
    /// The page with its query and fragment: what is opened.
    pub url: String,
    /// The page without query and fragment (which often carry tokens):
    /// what messages and outputs show.
    pub shown: String,
}

/// `host[:port]` split; an IPv6 literal keeps its brackets.
fn split_port(host_port: &str) -> (&str, Option<&str>) {
    if host_port.starts_with('[') {
        return match host_port.find(']') {
            Some(end) => {
                let (host, rest) = host_port.split_at(end + 1);
                (host, rest.strip_prefix(':'))
            }
            None => (host_port, None),
        };
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_port, None),
    }
}

/// A port as written: `None` for none, an empty one or 443; `Err` for
/// anything that is not a port.
fn port_of(port: Option<&str>) -> Result<Option<u16>, ()> {
    match port {
        None | Some("") => Ok(None),
        Some(port) => {
            if !port.bytes().all(|b| b.is_ascii_digit()) {
                return Err(());
            }
            let digits = port.trim_start_matches('0');
            let number: u16 = if digits.is_empty() {
                0
            } else {
                digits.parse().map_err(|_| ())?
            };
            Ok((number != 443).then_some(number))
        }
    }
}

fn host_valid(host: &str) -> bool {
    let name = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_');
    let literal = |c: char| c.is_ascii_hexdigit() || matches!(c, ':' | '.');
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return !inner.is_empty() && inner.chars().all(literal);
    }
    !host.is_empty() && host.len() <= 253 && host.chars().all(name)
}

impl Server {
    /// Parses a server as the agent stores an origin setting:
    /// `https://host[:port][/path]`, without a user name, query or
    /// fragment. Upper case and a trailing slash are accepted (a default
    /// from the manifest, a value from before the agent normalized them).
    /// `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '?' | '#' | '@'))
        {
            return None;
        }
        let scheme = text.get(..8)?;
        if !scheme.eq_ignore_ascii_case("https://") {
            return None;
        }
        let rest = &text[8..];
        let end = rest.find('/').unwrap_or(rest.len());
        let (authority, path) = rest.split_at(end);
        let (host, port) = split_port(authority);
        let host = host.to_ascii_lowercase();
        if !host_valid(&host) {
            return None;
        }
        let port = port_of(port).ok()?;
        if port == Some(0) {
            return None;
        }
        let path = path.trim_end_matches('/');
        let segments_ok = path
            .split('/')
            .skip(1)
            .all(|s| !s.is_empty() && s != "." && s != "..");
        if !segments_ok {
            return None;
        }
        Some(Self {
            host,
            port,
            path: path.to_owned(),
        })
    }

    /// `host[:port]`, as messages name the server.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// `https://host[:port]`.
    pub fn origin(&self) -> String {
        match self.port {
            Some(port) => format!("https://{}:{port}", self.host),
            None => format!("https://{}", self.host),
        }
    }

    /// The path: empty, or starting with `/` and not ending with one.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `https://host[:port][/path]`, without a trailing slash.
    pub fn base(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }

    /// `https://host[:port][/path]/`: what pages and endpoints are under.
    pub fn root(&self) -> String {
        format!("{}/", self.base())
    }

    /// The same server with its path without `suffix` (`/api/v4`), if the
    /// path ends with it: the web address that belongs to an API.
    #[must_use]
    pub fn without_suffix(&self, suffix: &str) -> Self {
        let path = self.path.strip_suffix(suffix).unwrap_or(&self.path);
        Self {
            host: self.host.clone(),
            port: self.port,
            path: path.to_owned(),
        }
    }

    /// A URL under the server: these path segments, each encoded (so a
    /// value with `/` stays one segment), and this query
    /// (`application/x-www-form-urlencoded`), as the built-ins build it
    /// with `path_segments_mut().extend` and
    /// `query_pairs_mut().extend_pairs`.
    pub fn endpoint(&self, segments: &[&str], query: &[(&str, &str)]) -> String {
        let path: Vec<String> = segments
            .iter()
            .filter(|s| !matches!(**s, "." | ".."))
            .map(|s| utf8_percent_encode(s, PATH_SEGMENT).to_string())
            .collect();
        let mut url = format!("{}{}", self.root(), path.join("/"));
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

    /// The parts of an `https` URL on this server's host and port, without
    /// a user name or password: `(path, query, fragment)` as written.
    fn on_origin<'t>(&self, text: &'t str) -> Option<(&'t str, Option<&'t str>, Option<&'t str>)> {
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
        let (host, port) = split_port(host_port);
        if !host.eq_ignore_ascii_case(&self.host) || port_of(port).ok()? != self.port {
            return None;
        }
        let (rest, fragment) = match rest.split_once('#') {
            Some((rest, fragment)) => (rest, Some(fragment)),
            None => (rest, None),
        };
        let (path, query) = match rest.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (rest, None),
        };
        Some((path, query, fragment))
    }

    /// Whether `link` (a server's next page) is on this server: `https`,
    /// its host and port, no user name or password, and a path that is the
    /// server's or below it.
    pub fn contains(&self, link: &str) -> bool {
        let link = link.trim();
        if link
            .chars()
            .any(|c| c.is_ascii_control() || c.is_whitespace())
            || !link
                .get(..8)
                .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        {
            return false;
        }
        let Some((path, _, _)) = self.on_origin(link) else {
            return false;
        };
        path == self.path
            || path
                .strip_prefix(self.path.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// `text` as an https page under this server ([`Server::root`]),
    /// normalized like the WHATWG URL parser does: the scheme and host in
    /// lower case, the default port dropped, `.` and `..` path segments
    /// resolved, and characters percent-encoded where a URL needs it.
    /// `None` for anything else.
    pub fn page(&self, text: &str) -> Option<WebPage> {
        // The URL parser drops tabs and newlines anywhere, and leading and
        // trailing spaces and control characters.
        let text: String = text
            .trim_matches(|c: char| c <= ' ')
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        let (path, query, fragment) = self.on_origin(&text)?;
        let path = normalized_path(path);
        if !path.starts_with(&format!("{}/", self.path)) {
            return None;
        }
        let shown = format!("{}{path}", self.origin());
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

    const SERVERS: [&str; 6] = [
        "https://gitlab.com/api/v4",
        "https://code.example.com/gitlab/api/v4",
        "https://jira.example.com",
        "https://jira.example.com:8443/jira",
        "https://10.0.0.5/api/v4",
        "https://[fd00::5]:8443/api/v4",
    ];

    #[test]
    fn servers_are_what_the_agent_stores() {
        for text in SERVERS {
            let server = Server::parse(text).unwrap();
            assert_eq!(server.base(), text, "{text}");
            assert_eq!(server.root(), format!("{text}/"));
            // The same origin and path as the URL parser sees.
            let url = url::Url::parse(text).unwrap();
            assert_eq!(
                server.origin(),
                url.origin().ascii_serialization(),
                "{text}"
            );
            assert_eq!(server.path(), url.path().trim_end_matches('/'), "{text}");
        }
        // Forms from before the agent normalized the setting.
        for (text, base) in [
            ("https://gitlab.com/api/v4/", "https://gitlab.com/api/v4"),
            (
                " HTTPS://GitLab.com:443/api/v4 ",
                "https://gitlab.com/api/v4",
            ),
            ("https://jira.example.com/", "https://jira.example.com"),
        ] {
            assert_eq!(Server::parse(text).unwrap().base(), base, "{text}");
        }
        for bad in [
            "",
            "gitlab.com/api/v4",
            "http://gitlab.com/api/v4",
            "https://",
            "https://u:p@gitlab.com/api/v4",
            "https://gitlab.com/api/v4?x=1",
            "https://gitlab.com/api/v4#x",
            "https://gitlab.com/a b",
            "https://gitlab.com/a//b",
            "https://gitlab.com/a/../b",
            "https://gitlab.com/./a",
            "https://gitlab.com:x/a",
            "https://gitlab.com:0/a",
            "https://gitlab.com:99999/a",
            "https://git lab.com/a",
            "https://gitlab.com\\a",
            "https://*.example.com",
        ] {
            assert_eq!(Server::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_web_address_of_an_api() {
        for (api, web) in [
            ("https://gitlab.com/api/v4", "https://gitlab.com/"),
            (
                "https://code.example.com/gitlab/api/v4",
                "https://code.example.com/gitlab/",
            ),
            // Not an API path: the same address.
            ("https://code.example.com/x", "https://code.example.com/x/"),
        ] {
            assert_eq!(
                Server::parse(api).unwrap().without_suffix("/api/v4").root(),
                web
            );
        }
    }

    /// The built-in modules' way: `url::Url` from the base.
    fn builtin_endpoint(base: &str, segments: &[&str], query: &[(&str, &str)]) -> String {
        let mut url = url::Url::parse(base).unwrap();
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
    fn endpoints_are_built_as_the_builtins_build_them() {
        let values = [
            "main",
            "group/sub/project",
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
            "42",
        ];
        for base in SERVERS {
            let server = Server::parse(base).unwrap();
            for value in values {
                let segments = ["projects", value, "merge_requests"];
                let query = [
                    ("state", "opened"),
                    ("ref", value),
                    ("jql", "assignee = currentUser() AND statusCategory != Done"),
                ];
                assert_eq!(
                    server.endpoint(&segments, &query),
                    builtin_endpoint(base, &segments, &query),
                    "{base} {value}"
                );
            }
            assert_eq!(
                server.endpoint(&["user"], &[]),
                builtin_endpoint(base, &["user"], &[])
            );
        }
        // A base with a trailing slash, as a site address is kept by the
        // Jira built-in.
        assert_eq!(
            Server::parse("https://jira.example.com/jira/")
                .unwrap()
                .endpoint(&["rest", "api", "3", "myself"], &[]),
            builtin_endpoint(
                "https://jira.example.com/jira/",
                &["rest", "api", "3", "myself"],
                &[]
            )
        );
    }

    #[test]
    fn only_links_on_the_server_are_followed() {
        let server = Server::parse("https://code.example.com/gitlab/api/v4").unwrap();
        for good in [
            "https://code.example.com/gitlab/api/v4/projects/1/merge_requests?page=2",
            "HTTPS://CODE.EXAMPLE.COM/gitlab/api/v4/x",
            "https://code.example.com:443/gitlab/api/v4/x",
            "https://code.example.com/gitlab/api/v4",
            "https://code.example.com/gitlab/api/v4?page=2",
        ] {
            assert!(server.contains(good), "{good}");
        }
        for bad in [
            "http://code.example.com/gitlab/api/v4/x",
            "https://code.example.com.evil.com/gitlab/api/v4/x",
            "https://evil.com/code.example.com/gitlab/api/v4/x",
            "https://user@code.example.com/gitlab/api/v4/x",
            "https://code.example.com:8443/gitlab/api/v4/x",
            "https://code.example.com/gitlab/api/v40/x",
            "https://code.example.com/gitlab/admin",
            "https://code.example.com/gitlab/api/v4/x y",
            "https:/code.example.com/gitlab/api/v4/x",
            "/gitlab/api/v4/x",
            "",
        ] {
            assert!(!server.contains(bad), "{bad}");
        }
        let literal = Server::parse("https://[fd00::5]:8443/api/v4").unwrap();
        assert!(literal.contains("https://[fd00::5]:8443/api/v4/user?page=2"));
        assert!(!literal.contains("https://[fd00::5]/api/v4/user"));
        assert!(!literal.contains("https://[fd00::6]:8443/api/v4/user"));
    }

    /// The built-in modules' check (`web_url` with a web base), and what
    /// it opens and shows.
    fn builtin_page(web: &str, text: &str) -> Option<WebPage> {
        let web = url::Url::parse(web).unwrap();
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
            || !url.path().starts_with(web.path())
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
    fn pages_are_checked_and_normalized_as_the_builtins_do() {
        let paths = [
            "/group/app/-/merge_requests/12",
            "/group/app/-/merge_requests/12?tab=files&token=q-s3cret#note_1",
            "",
            "/",
            "/a/../b",
            "/a/./b/.",
            "/a/%2e%2E/b",
            "/a/b/..",
            "/a b/c\"d<e>",
            "/naïve/ü?q=ä#ö",
            "/a?x='y'",
            "/a#b'c d",
            "/a%20b",
            "/a/%2E",
            "/../a",
            "?x",
            "#f",
            "/a//b",
            "/a/b?c=d?e#f#g",
            "/{x}|`y`",
            "/gitlab",
            "/gitlab/",
            "/gitlab/group/app",
            "/gitlab/../admin",
            "/gitlab/a/../../admin",
            "/gitlabx/a",
            "/browse/ABC-1",
            "/jira/browse/ABC-1",
        ];
        let origins = [
            "https://code.example.com",
            "https://CODE.Example.com",
            "HTTPS://code.example.com",
            "https://code.example.com:443",
            "https://code.example.com:",
            "https://code.example.com:0443",
            "https://code.example.com:8443",
            "https://code.example.com:443:443",
            "https:/code.example.com",
            "https:code.example.com",
            "https:\\\\code.example.com",
            "https:///code.example.com",
            "https://@code.example.com",
            "http://code.example.com",
            "https://code.example.com.evil.com",
            "https://evil.com/code.example.com",
            "https://user@code.example.com",
            "https://user:pw@code.example.com",
            "https://x.code.example.com",
            "https://10.0.0.5",
            "https://[fd00::5]:8443",
            "https://[fd00::5]",
        ];
        let webs = [
            "https://code.example.com/",
            "https://code.example.com/gitlab/",
            "https://code.example.com:8443/jira/",
            "https://10.0.0.5/",
            "https://[fd00::5]:8443/gitlab/",
        ];
        for web in webs {
            let server = Server::parse(web).unwrap();
            assert_eq!(server.root(), web);
            for origin in origins {
                for path in paths {
                    let case = format!("{origin}{path}");
                    assert_eq!(
                        server.page(&case),
                        builtin_page(web, &case),
                        "{web} {case:?}"
                    );
                }
            }
            for case in [
                "  https://code.example.com/a\t/b\n  ",
                "javascript:alert(1)",
                "file:///etc/passwd",
                "code.example.com/a",
                "",
            ] {
                assert_eq!(server.page(case), builtin_page(web, case), "{web} {case:?}");
            }
        }
    }
}
