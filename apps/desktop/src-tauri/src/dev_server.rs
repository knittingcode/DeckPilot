//! Development builds only: what a window shows when the Vite development
//! server (`devUrl`) is not running.
//!
//! A debug build loads its interface from `devUrl` instead of the embedded
//! `frontendDist`. Started without that server (for example by the tray
//! after `tauri dev` ended), the window would stay white. Instead the
//! window opens a built-in page that names the missing server and how to
//! start it, with a button that checks again and continues to the real
//! interface once the server answers. Packaged builds (`tauri build`,
//! `custom-protocol` feature) never register the scheme and never check:
//! [`fallback_url`] returns `None` there.

use std::borrow::Cow;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::http::{Request, Response, header};
use tauri::{Config, Url, WebviewUrl};

/// The scheme serving the diagnostic page.
pub const SCHEME: &str = "deckpilot-dev";

/// How long a connection attempt to the development server may take.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);

/// What the diagnostic page reports besides the server address.
#[derive(Debug, Clone)]
pub struct Session {
    pub product: String,
    pub identifier: String,
    pub data_dir: PathBuf,
}

/// The development server of this build, if it is a development build.
pub fn dev_url(config: &Config) -> Option<&Url> {
    if tauri::is_dev() {
        config.build.dev_url.as_ref()
    } else {
        None
    }
}

/// Whether something accepts connections at the host and port of `url`.
/// Every address the host resolves to is tried (`localhost` may be served
/// on IPv6 only).
pub fn reachable(url: &Url, timeout: Duration) -> bool {
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|address| TcpStream::connect_timeout(&address, timeout).is_ok())
}

/// The URL a window should load instead of the app page `path` (for example
/// `index.html?window=security.unlock`): the diagnostic page, when this is a
/// development build whose server does not answer. `None` otherwise.
pub fn fallback_url(config: &Config, path: &str) -> Option<WebviewUrl> {
    let dev = dev_url(config)?;
    if reachable(dev, CONNECT_TIMEOUT) {
        return None;
    }
    eprintln!("the development server {dev} is not running; showing the diagnostic page");
    page_url(path).map(WebviewUrl::CustomProtocol)
}

/// The diagnostic page for the app page `path`.
fn page_url(path: &str) -> Option<Url> {
    let base = if cfg!(any(windows, target_os = "android")) {
        format!("http://{SCHEME}.localhost/")
    } else {
        format!("{SCHEME}://localhost/")
    };
    let mut url = Url::parse(&base).ok()?;
    if !path.is_empty() {
        url.query_pairs_mut().append_pair("path", path);
    }
    Some(url)
}

/// Where to continue once the server answers: `path` relative to the
/// development server. Anything that would leave its origin is refused.
pub fn target(dev: &Url, path: &str) -> Option<Url> {
    let url = dev.join(path).ok()?;
    (url.origin() == dev.origin()).then_some(url)
}

/// Serves the diagnostic page. Every request checks the server again: if
/// it answers, the page forwards to the app page; otherwise it explains.
pub fn respond(
    dev: &Url,
    session: &Session,
    request: &Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    let path = Url::parse(&request.uri().to_string())
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "path")
                .map(|(_, value)| value.into_owned())
        })
        .unwrap_or_default();
    let target = target(dev, &path).unwrap_or_else(|| dev.clone());
    let html = if reachable(dev, CONNECT_TIMEOUT) {
        forward_html(&target)
    } else {
        diagnostic_html(dev, session)
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Cow::Owned(html.into_bytes()))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&b"DeckPilot"[..])))
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// Continues to the app page (no script: a meta refresh and a link).
pub fn forward_html(target: &Url) -> String {
    let target = escape(target.as_str());
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"refresh\" content=\"0;url={target}\">\
         <title>DeckPilot</title></head><body>\
         <p><a href=\"{target}\">Continue to DeckPilot</a></p></body></html>"
    )
}

/// The page shown while the development server does not answer.
pub fn diagnostic_html(dev: &Url, session: &Session) -> String {
    let url = escape(dev.as_str());
    let product = escape(&session.product);
    let identifier = escape(&session.identifier);
    let data = escape(&display(&session.data_dir));
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Development server not running</title>
<style>
:root {{ color-scheme: light dark; --fg: #1c1c1e; --muted: #5f6368; --bg: #f5f5f7; --card: #fff; --line: #d9d9de; --accent: #0a66d8; }}
@media (prefers-color-scheme: dark) {{ :root {{ --fg: #f2f2f7; --muted: #a1a1aa; --bg: #18181b; --card: #232327; --line: #3a3a40; --accent: #4c9bff; }} }}
body {{ margin: 0; background: var(--bg); color: var(--fg); font: 14px/1.5 -apple-system, "Segoe UI", system-ui, sans-serif; }}
main {{ max-width: 640px; margin: 0 auto; padding: 56px 24px 32px; }}
h1 {{ font-size: 20px; margin: 0 0 8px; }}
p {{ margin: 0 0 12px; }}
.card {{ background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 16px 18px; margin: 16px 0; }}
dl {{ display: grid; grid-template-columns: max-content 1fr; gap: 4px 16px; margin: 0; }}
dt {{ color: var(--muted); }}
dd {{ margin: 0; overflow-wrap: anywhere; }}
code {{ font: 12.5px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
ul {{ margin: 0; padding-left: 20px; }}
li {{ margin: 4px 0; }}
.button {{ display: inline-block; background: var(--accent); color: #fff; text-decoration: none; border-radius: 7px; padding: 7px 16px; font-weight: 600; }}
.muted {{ color: var(--muted); font-size: 12.5px; }}
</style></head>
<body><main>
<h1>The development frontend server is not running</h1>
<p>This is a development build of {product}. It loads its interface from the Vite
development server at <code>{url}</code>, but nothing answers there, so the
interface cannot be shown. The background agent is not affected.</p>
<div class="card"><dl>
<dt>Server</dt><dd><code>{url}</code></dd>
<dt>App</dt><dd><code>{identifier}</code></dd>
<dt>Data directory</dt><dd><code>{data}</code></dd>
</dl></div>
<div class="card">
<p><strong>Start it from the repository of this session:</strong></p>
<ul>
<li><code>make up</code> starts a development session (agent and app with live reload).
In a running session the tray's <em>Open DeckPilot</em> starts the app with its server.</li>
<li><code>make ui</code> starts only the app with its server, next to an agent that already runs.</li>
<li>Integration preview: <code>tools/preview/start.sh</code> in the preview worktree.</li>
</ul>
</div>
<p><a class="button" href="./">Retry</a></p>
<p class="muted">Retry checks the server again and opens the interface once it answers.</p>
</main></body></html>
"#
    )
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    fn session() -> Session {
        Session {
            product: "DeckPilot".into(),
            identifier: "io.github.knittingcode.deckpilot".into(),
            data_dir: PathBuf::from("/tmp/<dev> & data"),
        }
    }

    #[test]
    fn a_listening_server_is_reachable_and_a_closed_port_is_not() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        assert!(reachable(&url, CONNECT_TIMEOUT));
        drop(listener);
        assert!(!reachable(&url, CONNECT_TIMEOUT));
        // `localhost` resolves to every loopback address; one must answer.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = Url::parse(&format!("http://localhost:{port}/")).unwrap();
        assert!(reachable(&url, CONNECT_TIMEOUT));
    }

    #[test]
    fn the_diagnostic_page_names_the_server_the_session_and_how_to_start_it() {
        let dev = Url::parse("http://localhost:1430/").unwrap();
        let html = diagnostic_html(&dev, &session());
        assert!(html.contains("http://localhost:1430/"));
        assert!(html.contains("io.github.knittingcode.deckpilot"));
        assert!(html.contains("make up"));
        assert!(html.contains("make ui"));
        assert!(html.contains("tools/preview/start.sh"));
        assert!(html.contains("href=\"./\">Retry"));
        // Paths are escaped, not interpreted as markup.
        assert!(html.contains("/tmp/&lt;dev&gt; &amp; data"));
        assert!(!html.contains("<dev>"));
        assert!(!html.contains("<script"), "the page works without scripts");
    }

    #[test]
    fn the_page_continues_to_the_requested_app_page_on_the_same_server() {
        let dev = Url::parse("http://localhost:1420").unwrap();
        assert_eq!(target(&dev, "").unwrap().as_str(), "http://localhost:1420/");
        assert_eq!(
            target(&dev, "index.html?window=security.unlock")
                .unwrap()
                .as_str(),
            "http://localhost:1420/index.html?window=security.unlock"
        );
        assert!(target(&dev, "//example.com/").is_none());
        assert!(target(&dev, "https://example.com/").is_none());
        assert!(target(&dev, "http://localhost:1421/").is_none());
        let page = page_url("index.html?window=security.unlock").unwrap();
        let path = page
            .query_pairs()
            .find(|(key, _)| key == "path")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(path, "index.html?window=security.unlock");
        let html = forward_html(&target(&dev, &path).unwrap());
        assert!(html.contains("url=http://localhost:1420/index.html?window=security.unlock"));
    }

    #[test]
    fn requests_answer_with_the_diagnostic_while_the_server_is_down_and_forward_once_it_is_up() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let dev = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        let request = |uri: &str| Request::builder().uri(uri).body(Vec::new()).unwrap();
        let body = |response: Response<Cow<'static, [u8]>>| {
            String::from_utf8(response.body().to_vec()).unwrap()
        };
        let up = body(respond(
            &dev,
            &session(),
            &request("deckpilot-dev://localhost/?path=index.html%3Fwindow%3Dx"),
        ));
        assert!(up.contains(&format!("url=http://127.0.0.1:{port}/index.html?window=x")));
        drop(listener);
        let down = body(respond(
            &dev,
            &session(),
            &request("deckpilot-dev://localhost/"),
        ));
        assert!(down.contains("development frontend server is not running"));
    }

    #[test]
    fn only_development_builds_without_a_running_server_fall_back() {
        let mut config = Config::default();
        config.build.dev_url = Some(Url::parse("http://localhost:1").unwrap());
        // Packaged builds (`custom-protocol`) load the embedded frontend.
        assert_eq!(dev_url(&config).is_some(), tauri::is_dev());
        let page = fallback_url(&config, "");
        if tauri::is_dev() {
            assert!(
                matches!(page, Some(WebviewUrl::CustomProtocol(url)) if url.as_str().contains(SCHEME))
            );
        } else {
            assert!(page.is_none());
        }
        config.build.dev_url = None;
        assert!(fallback_url(&config, "").is_none());
    }
}
