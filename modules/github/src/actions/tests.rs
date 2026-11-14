//! Requests go to a small GitHub stand-in on 127.0.0.1 started by the
//! test; no other host is contacted and no real account is used.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use deckpilot_core::FeatureModule;
use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{
    CancellationToken, ExecutionControl, MachineConfig, MemoryCredentials, Os, Progress, Secret,
};

use super::*;
use crate::api::MAX_PAGES;

const TOKEN: &str = "ghp_s3cretTOKENvalue";

/// Request heads the server received.
type Seen = Arc<Mutex<Vec<String>>>;

struct Server {
    port: u16,
    seen: Seen,
    /// Stops the server when the test ends.
    _acceptor: local::Acceptor,
}

fn json(status: &str, body: &str) -> (String, Vec<String>, Vec<u8>) {
    (
        status.to_owned(),
        vec!["Content-Type: application/json".into()],
        body.as_bytes().to_vec(),
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Year, month (1–12) and day of a day number since 1970-01-01
/// (H. Hinnant's `civil_from_days`).
fn civil(days: u64) -> (u64, usize, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, usize::try_from(month).unwrap(), day)
}

/// `at` (Unix seconds) as an HTTP date; the weekday is not checked.
fn http_date(at: u64) -> String {
    let (days, rest) = (at / 86_400, at % 86_400);
    let (year, month, day) = civil(days);
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "Mon, {day:02} {} {year} {:02}:{:02}:{:02} GMT",
        months[month - 1],
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

fn search(query: &str) -> (String, Vec<String>, Vec<u8>) {
    let per_page: usize = query
        .split('&')
        .find_map(|p| p.strip_prefix("per_page="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let items: Vec<String> = (1..=3)
        .take(per_page)
        .map(|n| {
            format!(
                r#"{{"number":{n},"title":"Fix\nthing {n}","html_url":"https://github.com/octo/site/pull/{n}","repository_url":"https://api.github.com/repos/octo/site","user":{{"login":"dev{n}"}},"draft":{}}}"#,
                n == 2
            )
        })
        .collect();
    json(
        "200 OK",
        &format!(
            r#"{{"total_count":42,"incomplete_results":false,"items":[{}]}}"#,
            items.join(",")
        ),
    )
}

#[allow(clippy::too_many_lines)] // One table of the stand-in's answers.
fn route(
    path: &str,
    query: &str,
    authorization: &str,
    port: u16,
) -> (String, Vec<String>, Vec<u8>) {
    if path == "/repos/octo/leak" {
        // A server that repeats what it was sent.
        return json(
            "403 Forbidden",
            &format!(r#"{{"message":"refused {authorization}"}}"#),
        );
    }
    if authorization != format!("Bearer {TOKEN}") {
        return json("401 Unauthorized", r#"{"message":"Bad credentials"}"#);
    }
    let page: u32 = query
        .split('&')
        .find_map(|p| p.strip_prefix("page="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    match path {
        "/user" => {
            let (status, mut headers, body) = json(
                "200 OK",
                r#"{"login":"octocat","name":"The Octocat","html_url":"https://github.com/octocat"}"#,
            );
            headers.push("X-RateLimit-Remaining: 4999".into());
            (status, headers, body)
        }
        "/repos/octo/site" => json(
            "200 OK",
            r#"{"full_name":"octo/site","description":"Our web\u202esite","default_branch":"main","private":true,"archived":false,"open_issues_count":12,"stargazers_count":5,"forks_count":2,"html_url":"https://github.com/octo/site","pushed_at":"2026-11-01T10:00:00Z"}"#,
        ),
        "/repos/octo/forbidden" => json(
            "403 Forbidden",
            r#"{"message":"Resource not accessible by personal access token"}"#,
        ),
        "/repos/octo/limited" => {
            let (_, mut headers, body) =
                json("403 Forbidden", r#"{"message":"API rate limit exceeded"}"#);
            headers.push("X-RateLimit-Remaining: 0".into());
            headers.push(format!("X-RateLimit-Reset: {}", now() + 120));
            ("403 Forbidden".into(), headers, body)
        }
        "/repos/octo/secondary" => {
            let (_, mut headers, body) = json("429 Too Many Requests", "{}");
            headers.push("Retry-After: 30".into());
            ("429 Too Many Requests".into(), headers, body)
        }
        "/repos/octo/dated" => {
            // Retry-After as an HTTP date, 90 s ahead.
            let (_, mut headers, body) = json("429 Too Many Requests", "{}");
            headers.push(format!("Retry-After: {}", http_date(now() + 90)));
            ("429 Too Many Requests".into(), headers, body)
        }
        "/repos/octo/broken" => json("502 Bad Gateway", r#"{"message":"Server Error"}"#),
        "/repos/octo/slow" => {
            std::thread::sleep(Duration::from_secs(5));
            json("200 OK", "{}")
        }
        "/repos/octo/big" => (
            "200 OK".into(),
            vec!["Content-Type: application/json".into()],
            vec![b' '; crate::api::MAX_RESPONSE_BYTES + 10],
        ),
        "/search/issues" => search(query),
        "/repos/octo/site/pulls/7" => json(
            "200 OK",
            r#"{"number":7,"head":{"sha":"abc123def"},"html_url":"https://github.com/octo/site/pull/7"}"#,
        ),
        p if p.starts_with("/repos/octo/site/commits/") && p.ends_with("/check-runs") => json(
            "200 OK",
            r#"{"total_count":4,"check_runs":[
                {"name":"build","status":"completed","conclusion":"success"},
                {"name":"lint","status":"completed","conclusion":"failure"},
                {"name":"docs","status":"completed","conclusion":"skipped"},
                {"name":"e2e","status":"in_progress","conclusion":null}]}"#,
        ),
        p if p.starts_with("/repos/octo/site/commits/") && p.ends_with("/status") => json(
            "200 OK",
            r#"{"state":"failure","sha":"abc123def","total_count":2,"statuses":[
                {"context":"ci/a","state":"success"},
                {"context":"ci/b","state":"error"}]}"#,
        ),
        "/repos/octo/green/commits/main/check-runs" => json(
            "200 OK",
            r#"{"total_count":1,"check_runs":[{"name":"build","status":"completed","conclusion":"success"}]}"#,
        ),
        "/repos/octo/endless/commits/main/check-runs" => {
            let (status, mut headers, body) = json(
                "200 OK",
                r#"{"total_count":1000,"check_runs":[{"name":"x","status":"completed","conclusion":"success"}]}"#,
            );
            headers.push(format!(
                "Link: <http://127.0.0.1:{port}/repos/octo/endless/commits/main/check-runs?page={}>; rel=\"next\"",
                page + 1
            ));
            (status, headers, body)
        }
        "/repos/octo/escape/commits/main/check-runs" => {
            let (status, mut headers, body) = json("200 OK", r#"{"check_runs":[]}"#);
            headers.push("Link: <https://collector.example/steal>; rel=\"next\"".into());
            (status, headers, body)
        }
        p if p.ends_with("/status") => json(
            "200 OK",
            r#"{"state":"pending","sha":"fff000","total_count":0,"statuses":[]}"#,
        ),
        _ => json("404 Not Found", r#"{"message":"Not Found"}"#),
    }
}

fn respond(mut stream: TcpStream, seen: &Seen, port: u16) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line == "\r\n" {
            break;
        }
        head.push_str(&line);
    }
    seen.lock().unwrap().push(head.clone());
    let target = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let authorization = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default();
    let (status, headers, body) = route(path, query, &authorization, port);
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for header in headers {
        response.push_str(&header);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(&body);
}

/// Local stand-in servers that stop with the test: the listener is closed
/// when the [`local::Acceptor`] is dropped, at most a few connections are
/// served at once, and binding retries the short-lived errors of a busy
/// machine (Windows runs out of ephemeral ports or socket buffers for a
/// moment: WSAENOBUFS, WSAEADDRINUSE).
mod local {
    use std::io;
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    /// Connections served at the same time; more wait to be accepted.
    const MAX_CONNECTIONS: usize = 16;
    /// How often the accept loop looks for a connection or the stop.
    const POLL: Duration = Duration::from_millis(2);
    const BIND_ATTEMPTS: u32 = 8;

    /// An error that goes away once ports or buffers are freed again.
    fn transient(error: &io::Error) -> bool {
        const NO_BUFFERS: i32 = if cfg!(windows) {
            10_055 // WSAENOBUFS
        } else if cfg!(target_os = "linux") {
            105 // ENOBUFS
        } else {
            55 // ENOBUFS (macOS, BSD)
        };
        matches!(
            error.kind(),
            io::ErrorKind::AddrInUse | io::ErrorKind::AddrNotAvailable
        ) || error.raw_os_error() == Some(NO_BUFFERS)
    }

    /// A listener on a free port of 127.0.0.1, retrying transient errors
    /// with a growing pause.
    pub fn bind() -> TcpListener {
        let mut pause = Duration::from_millis(50);
        for _ in 1..BIND_ATTEMPTS {
            match TcpListener::bind("127.0.0.1:0") {
                Ok(listener) => return listener,
                Err(error) if transient(&error) => {
                    std::thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_secs(1));
                }
                Err(error) => panic!("binding 127.0.0.1 failed: {error}"),
            }
        }
        TcpListener::bind("127.0.0.1:0").expect("binding 127.0.0.1")
    }

    /// Accepts connections on its own thread and serves each with
    /// `serve` (given the port) on a thread of its own until dropped.
    pub struct Acceptor {
        pub port: u16,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Acceptor {
        pub fn start(serve: impl Fn(TcpStream, u16) + Send + Sync + 'static) -> Self {
            let listener = bind();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let serve = Arc::new(serve);
            let thread = std::thread::spawn(move || {
                let active = Arc::new(AtomicUsize::new(0));
                while !flag.load(Ordering::SeqCst) {
                    let Ok((stream, _)) = listener.accept() else {
                        // Nothing waiting, or a failed accept.
                        std::thread::sleep(POLL);
                        continue;
                    };
                    // Accepted sockets may inherit non-blocking mode.
                    let _ = stream.set_nonblocking(false);
                    while active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                        std::thread::sleep(POLL);
                    }
                    active.fetch_add(1, Ordering::SeqCst);
                    let done = Done(active.clone());
                    let serve = serve.clone();
                    std::thread::spawn(move || {
                        let _done = done;
                        serve(stream, port);
                    });
                }
                // The listener closes here.
            });
            Self {
                port,
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for Acceptor {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// Counts a connection as finished, even when serving it panicked.
    struct Done(Arc<AtomicUsize>);

    impl Drop for Done {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn server() -> Server {
    let seen: Seen = Arc::default();
    let shared = seen.clone();
    let acceptor = local::Acceptor::start(move |stream, port| respond(stream, &shared, port));
    Server {
        port: acceptor.port,
        seen,
        _acceptor: acceptor,
    }
}

impl Server {
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    fn last(&self) -> String {
        self.seen().pop().expect("a request reached the server")
    }

    fn settings(&self) -> Arc<GithubSettings> {
        settings_for(&format!("http://127.0.0.1:{}", self.port), true, 20)
    }
}

fn settings_for(api_url: &str, allow_private: bool, timeout: i64) -> Arc<GithubSettings> {
    Arc::new(crate::settings(
        &ActionParameters::new()
            .with(crate::API_URL, ParameterValue::Text(api_url.into()))
            .with(crate::ALLOW_PRIVATE, ParameterValue::Boolean(allow_private))
            .with(crate::TIMEOUT, ParameterValue::Integer(timeout)),
    ))
}

struct Fixture {
    profile: Profile,
    machine: MachineConfig,
    credentials: MemoryCredentials,
}

fn fixture() -> Fixture {
    Fixture {
        profile: Profile::new("Work"),
        machine: MachineConfig::new(Os::current()),
        credentials: MemoryCredentials::new()
            .with(
                CredentialId::new("github.token").unwrap(),
                Secret::new(TOKEN),
            )
            .with(
                CredentialId::new("github.old").unwrap(),
                Secret::new("ghp_revokedTOKEN"),
            ),
    }
}

impl Fixture {
    fn context(&self) -> ExecutionContext<'_> {
        ExecutionContext::new(&self.profile, None, &self.machine)
            .with_credentials(&self.credentials)
    }
}

fn handler_with(
    kind: &str,
    settings: &Arc<GithubSettings>,
    launcher: &Arc<dyn Launcher>,
) -> Box<dyn ActionHandler> {
    handlers(settings, launcher)
        .into_iter()
        .find(|h| h.definition().id.as_str() == kind)
        .unwrap()
}

fn recording() -> Arc<dyn Launcher> {
    Arc::new(RecordingLauncher::new())
}

fn run_with(
    kind: &str,
    settings: &Arc<GithubSettings>,
    parameters: &ActionParameters,
    context: &ExecutionContext<'_>,
) -> ActionResult {
    let handler = handler_with(kind, settings, &recording());
    let normalized = handler.definition().normalize(parameters).unwrap();
    handler.execute(&normalized, context)
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.into())
}

fn repo(name: &str) -> ActionParameters {
    ActionParameters::new().with("repository", text(name))
}

/// The token is nowhere in a result.
fn assert_no_token(result: &ActionResult) {
    let shown = serde_json::to_string(result).unwrap();
    assert!(
        !shown.contains(TOKEN) && !shown.contains("ghp_revoked"),
        "{shown}"
    );
}

#[test]
fn the_module_and_its_contract() {
    let module = crate::GithubModule::new();
    let manifest = module.manifest();
    assert_eq!(manifest.id.as_str(), "github");
    assert_eq!(
        manifest.capabilities,
        [
            Capability::Network,
            Capability::Credentials,
            Capability::OpenExternal
        ]
    );
    let defaults = crate::settings(&ActionParameters::new());
    assert_eq!(defaults.api_url, crate::DEFAULT_API_URL);
    assert_eq!(defaults.credential, "github.token");
    assert!(!defaults.allow_private);
    assert_eq!(
        manifest.portability.settings[crate::API_URL],
        deckpilot_core::SettingScope::MachineSpecific
    );
    assert_eq!(
        manifest.portability.settings[crate::ALLOW_PRIVATE],
        deckpilot_core::SettingScope::MachineSpecific
    );

    let settings = settings_for(crate::DEFAULT_API_URL, false, 20);
    let all = handlers(
        &settings,
        &(Arc::new(RecordingLauncher::new()) as Arc<dyn Launcher>),
    );
    let ids: Vec<_> = all.iter().map(|h| h.definition().id.to_string()).collect();
    assert_eq!(
        ids,
        [
            CURRENT_USER,
            REPOSITORY,
            PULL_REQUESTS,
            REVIEW_REQUESTS,
            CHECKS,
            OPEN
        ]
    );
    for handler in &all {
        let definition = handler.definition();
        let node = &definition.node;
        if definition.id.as_str() == OPEN {
            assert_eq!(node.effect, ActionEffect::Opens);
            assert!(definition.parameters.iter().all(|p| p.key != "credential"));
        } else {
            assert_eq!(node.effect, ActionEffect::ReadOnly, "{}", definition.id);
            assert!(node.cancellable && node.retry_safe);
            assert!(definition.parameters.iter().any(|p| p.key == "credential"));
        }
    }
    let sensitive = |kind: &str| -> Vec<String> {
        all.iter()
            .find(|h| h.definition().id.as_str() == kind)
            .unwrap()
            .definition()
            .node
            .outputs
            .iter()
            .filter(|o| o.sensitive)
            .map(|o| o.key.clone())
            .collect()
    };
    assert_eq!(sensitive(REPOSITORY), ["description"]);
    assert_eq!(sensitive(PULL_REQUESTS), ["items"]);
    assert_eq!(sensitive(REVIEW_REQUESTS), ["items"]);
}

#[test]
fn the_current_user_with_the_token_as_bearer() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        CURRENT_USER,
        &server.settings(),
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["login"], "octocat");
    assert_eq!(result.data["name"], "The Octocat");
    assert_eq!(result.data["url"], "https://github.com/octocat");
    assert_eq!(result.data["rateLimitRemaining"], "4999");
    let request = server.last();
    assert!(request.starts_with("GET /user "), "{request}");
    assert!(
        request.contains(&format!("authorization: Bearer {TOKEN}")),
        "{request}"
    );
    let lower = request.to_lowercase();
    assert!(lower.contains("accept: application/vnd.github+json"));
    assert!(lower.contains("x-github-api-version: 2022-11-28"));
    assert_no_token(&result);
}

#[test]
fn private_api_hosts_need_permission() {
    let server = server();
    let fixture = fixture();
    let strict = settings_for(&format!("http://127.0.0.1:{}", server.port), false, 20);
    let refused = run_with(
        CURRENT_USER,
        &strict,
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert_eq!(
        refused.code.as_deref(),
        Some("blocked-address"),
        "{}",
        refused.message
    );
    assert!(refused.message.contains("Settings › Modules › GitHub"));
    assert_eq!(server.seen().len(), 0, "nothing reached the server");

    // Plain http is only for this computer.
    let insecure = settings_for("http://api.example.com", true, 20);
    let result = run_with(
        CURRENT_USER,
        &insecure,
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert_eq!(result.code.as_deref(), Some(codes::INVALID_API_URL));
}

#[test]
fn repository_information() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        REPOSITORY,
        &server.settings(),
        &repo("octo/site"),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["fullName"], "octo/site");
    assert_eq!(result.data["defaultBranch"], "main");
    assert_eq!(result.data["private"], "true");
    assert_eq!(result.data["archived"], "false");
    assert_eq!(result.data["openIssues"], "12");
    assert_eq!(result.data["stars"], "5");
    assert_eq!(result.data["forks"], "2");
    assert_eq!(result.data["pushedAt"], "2026-11-01T10:00:00Z");
    // Bidirectional overrides do not survive.
    assert_eq!(result.data["description"], "Our web site");
    assert!(server.last().starts_with("GET /repos/octo/site "));

    let invalid = run_with(
        REPOSITORY,
        &server.settings(),
        &repo("octo/../../user"),
        &fixture.context(),
    );
    assert_eq!(invalid.code.as_deref(), Some(codes::INVALID_REPOSITORY));
}

#[test]
fn github_failures_have_codes() {
    let server = server();
    let fixture = fixture();
    let settings = server.settings();
    let run = |name: &str| run_with(REPOSITORY, &settings, &repo(name), &fixture.context());

    let missing = run("octo/private");
    assert_eq!(missing.code.as_deref(), Some(codes::NOT_FOUND));
    assert!(
        missing.message.contains("octo/private"),
        "{}",
        missing.message
    );

    let forbidden = run("octo/forbidden");
    assert_eq!(forbidden.code.as_deref(), Some(codes::FORBIDDEN));
    assert!(forbidden.message.contains("not accessible"));

    let limited = run("octo/limited");
    assert_eq!(limited.code.as_deref(), Some(codes::RATE_LIMITED));
    let wait: u64 = limited.data["retryAfterSeconds"].parse().unwrap();
    assert!((100..=120).contains(&wait), "{wait}");
    assert!(limited.message.contains("try again in"));

    let secondary = run("octo/secondary");
    assert_eq!(secondary.code.as_deref(), Some(codes::RATE_LIMITED));
    assert_eq!(secondary.data["retryAfterSeconds"], "30");

    let dated = run("octo/dated");
    assert_eq!(dated.code.as_deref(), Some(codes::RATE_LIMITED));
    let wait: u64 = dated.data["retryAfterSeconds"].parse().unwrap();
    assert!((85..=90).contains(&wait), "{wait}");

    let broken = run("octo/broken");
    assert_eq!(broken.code.as_deref(), Some(codes::GITHUB_ERROR));
    assert!(broken.message.contains("502"));

    let big = run("octo/big");
    assert_eq!(big.code.as_deref(), Some(codes::INVALID_RESPONSE));

    for result in [&missing, &forbidden, &limited, &secondary, &broken, &big] {
        assert_no_token(result);
    }
}

#[test]
fn credentials_missing_refused_or_chosen() {
    let server = server();
    let fixture = fixture();
    let settings = server.settings();
    let revoked = run_with(
        CURRENT_USER,
        &settings,
        &ActionParameters::new().with("credential", text("github.old")),
        &fixture.context(),
    );
    assert_eq!(revoked.code.as_deref(), Some(codes::UNAUTHORIZED));
    assert!(revoked.message.contains("github.old"));
    assert!(server.last().contains("Bearer ghp_revokedTOKEN"));
    assert_no_token(&revoked);

    let missing = run_with(
        CURRENT_USER,
        &settings,
        &ActionParameters::new().with("credential", text("github.other")),
        &fixture.context(),
    );
    assert_eq!(missing.code.as_deref(), Some("credential-missing"));
    let before = server.seen().len();

    // Without any stored credentials.
    let empty = MemoryCredentials::new();
    let context =
        ExecutionContext::new(&fixture.profile, None, &fixture.machine).with_credentials(&empty);
    let none = run_with(CURRENT_USER, &settings, &ActionParameters::new(), &context);
    assert_eq!(none.code.as_deref(), Some("credential-missing"));
    assert!(none.message.contains("github.token"));
    assert_eq!(server.seen().len(), before, "nothing sent without a token");

    // Refused when saved (a credential parameter) and, should it get
    // through, when run.
    let parameters = ActionParameters::new().with("credential", text("Not A Name"));
    let handler = handler_with(CURRENT_USER, &settings, &recording());
    assert!(handler.definition().normalize(&parameters).is_err());
    let invalid = handler.execute(&parameters, &fixture.context());
    assert_eq!(invalid.code.as_deref(), Some("invalid-parameters"));
}

#[test]
fn a_token_repeated_by_the_server_is_redacted() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        REPOSITORY,
        &server.settings(),
        &repo("octo/leak"),
        &fixture.context(),
    );
    assert_eq!(result.code.as_deref(), Some(codes::FORBIDDEN));
    assert!(result.message.contains("[redacted]"), "{}", result.message);
    assert_no_token(&result);
}

#[test]
fn open_pull_requests_with_filters() {
    let server = server();
    let fixture = fixture();
    let settings = server.settings();
    let all = run_with(
        PULL_REQUESTS,
        &settings,
        &repo("octo/site").with("limit", ParameterValue::Integer(2)),
        &fixture.context(),
    );
    assert!(all.is_success(), "{}", all.message);
    assert_eq!(all.data["count"], "42");
    assert_eq!(all.data["listed"], "2");
    assert_eq!(all.data["truncated"], "true");
    assert_eq!(
        all.data["items"],
        "octo/site#1 Fix thing 1 (@dev1)\nocto/site#2 Fix thing 2 (@dev2) [draft]"
    );
    assert_eq!(all.data["firstUrl"], "https://github.com/octo/site/pull/1");
    // Titles stay out of the message, which is kept in history.
    assert!(!all.message.contains("Fix"));
    let request = server.last();
    assert!(request.starts_with("GET /search/issues?"), "{request}");
    assert!(
        request.contains("q=is%3Apr+is%3Aopen+repo%3Aocto%2Fsite&"),
        "{request}"
    );
    assert!(request.contains("per_page=2"));

    let mine = run_with(
        PULL_REQUESTS,
        &settings,
        &ActionParameters::new().with("filter", ParameterValue::Choice("author-me".into())),
        &fixture.context(),
    );
    assert!(mine.is_success(), "{}", mine.message);
    assert!(
        server
            .last()
            .contains("q=is%3Apr+is%3Aopen+archived%3Afalse+author%3A%40me&")
    );
    let review = run_with(
        PULL_REQUESTS,
        &settings,
        &repo("octo/site").with("filter", ParameterValue::Choice("review-requested".into())),
        &fixture.context(),
    );
    assert!(review.is_success());
    assert!(server.last().contains("+review-requested%3A%40me&"));

    let everything = run_with(
        PULL_REQUESTS,
        &settings,
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert_eq!(everything.code.as_deref(), Some(codes::INVALID_REPOSITORY));
}

#[test]
fn review_requests_across_repositories() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        REVIEW_REQUESTS,
        &server.settings(),
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["count"], "42");
    assert_eq!(result.data["listed"], "3");
    assert!(result.message.contains("wait for your review"));
    let request = server.last();
    assert!(
        request.contains("q=is%3Apr+is%3Aopen+archived%3Afalse+review-requested%3A%40me&"),
        "{request}"
    );
    assert!(request.contains("per_page=20"));
}

#[test]
fn checks_of_a_ref_and_of_a_pull_request() {
    let server = server();
    let fixture = fixture();
    let settings = server.settings();
    let by_ref = run_with(
        CHECKS,
        &settings,
        &repo("octo/site").with("ref", text("feature/login")),
        &fixture.context(),
    );
    assert!(by_ref.is_success(), "{}", by_ref.message);
    assert_eq!(by_ref.data["state"], "failing");
    assert_eq!(by_ref.data["total"], "6");
    assert_eq!(by_ref.data["passed"], "3");
    assert_eq!(by_ref.data["failed"], "2");
    assert_eq!(by_ref.data["pending"], "1");
    assert_eq!(by_ref.data["failing"], "lint\nci/b");
    assert_eq!(by_ref.data["sha"], "abc123def");
    assert_eq!(by_ref.data["truncated"], "false");
    assert_eq!(
        by_ref.data["url"],
        format!(
            "http://127.0.0.1:{}/octo/site/commit/abc123def",
            server.port
        )
    );
    let seen = server.seen();
    assert!(
        seen.iter()
            .any(|r| r.starts_with("GET /repos/octo/site/commits/feature%2Flogin/check-runs?")),
        "{seen:?}"
    );

    let by_pr = run_with(
        CHECKS,
        &settings,
        &repo("octo/site").with("pullRequest", ParameterValue::Integer(7)),
        &fixture.context(),
    );
    assert!(by_pr.is_success(), "{}", by_pr.message);
    assert_eq!(by_pr.data["url"], "https://github.com/octo/site/pull/7");
    assert!(
        server
            .seen()
            .iter()
            .any(|r| r.starts_with("GET /repos/octo/site/commits/abc123def/status?"))
    );

    let green = run_with(
        CHECKS,
        &settings,
        &repo("octo/green").with("ref", text("main")),
        &fixture.context(),
    );
    assert_eq!(green.data["state"], "ok", "{}", green.message);
    assert_eq!(green.data["failing"], "");

    let none = run_with(CHECKS, &settings, &repo("octo/site"), &fixture.context());
    assert_eq!(none.code.as_deref(), Some("invalid-parameters"));
    let bad_ref = run_with(
        CHECKS,
        &settings,
        &repo("octo/site").with("ref", text("../../user")),
        &fixture.context(),
    );
    assert_eq!(bad_ref.code.as_deref(), Some(codes::INVALID_REF));
}

#[test]
fn pages_are_bounded_and_stay_on_the_api() {
    let server = server();
    let fixture = fixture();
    let settings = server.settings();
    let endless = run_with(
        CHECKS,
        &settings,
        &repo("octo/endless").with("ref", text("main")),
        &fixture.context(),
    );
    assert!(endless.is_success(), "{}", endless.message);
    assert_eq!(endless.data["truncated"], "true");
    assert_eq!(endless.data["total"], MAX_PAGES.to_string());
    let pages = server
        .seen()
        .iter()
        .filter(|r| r.contains("/endless/commits/main/check-runs"))
        .count();
    assert_eq!(pages, MAX_PAGES);

    let escape = run_with(
        CHECKS,
        &settings,
        &repo("octo/escape").with("ref", text("main")),
        &fixture.context(),
    );
    assert_eq!(
        escape.code.as_deref(),
        Some(codes::INVALID_RESPONSE),
        "{}",
        escape.message
    );
}

#[test]
fn time_limits_and_cancellation() {
    let server = server();
    let fixture = fixture();
    let quick = settings_for(&format!("http://127.0.0.1:{}", server.port), true, 1);
    let begun = Instant::now();
    let slow = run_with(REPOSITORY, &quick, &repo("octo/slow"), &fixture.context());
    assert_eq!(slow.code.as_deref(), Some("timed-out"), "{}", slow.message);
    assert!(begun.elapsed() < Duration::from_secs(4));

    let token = CancellationToken::new();
    let progress = |_: Progress| {};
    let control = ExecutionControl::new(&token, &progress);
    let context = fixture.context().with_control(&control);
    let canceller = {
        let token = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            token.cancel();
        })
    };
    let begun = Instant::now();
    let stopped = run_with(REPOSITORY, &server.settings(), &repo("octo/slow"), &context);
    canceller.join().unwrap();
    assert_eq!(
        stopped.code.as_deref(),
        Some("cancelled"),
        "{}",
        stopped.message
    );
    assert!(begun.elapsed() < Duration::from_secs(3));

    // Already cancelled: nothing is sent.
    let before = server.seen().len();
    let again = run_with(REPOSITORY, &server.settings(), &repo("octo/site"), &context);
    assert_eq!(again.code.as_deref(), Some("cancelled"));
    assert_eq!(server.seen().len(), before);
}

#[test]
fn capabilities_are_required() {
    let server = server();
    let fixture = fixture();
    let credentials_only = BTreeSet::from([Capability::Credentials]);
    let denied = run_with(
        CURRENT_USER,
        &server.settings(),
        &ActionParameters::new(),
        &fixture.context().restricted(&credentials_only),
    );
    assert_eq!(denied.code.as_deref(), Some("not-permitted"));
    let network_only = BTreeSet::from([Capability::Network]);
    let no_token = run_with(
        CURRENT_USER,
        &server.settings(),
        &ActionParameters::new(),
        &fixture.context().restricted(&network_only),
    );
    assert_eq!(no_token.code.as_deref(), Some("not-permitted"));
    assert_eq!(server.seen().len(), 0);

    let settings = settings_for(crate::DEFAULT_API_URL, false, 20);
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = handler_with(OPEN, &settings, &(launcher.clone() as Arc<dyn Launcher>));
    let parameters = handler.definition().normalize(&repo("octo/site")).unwrap();
    let opened = handler.execute(&parameters, &fixture.context().restricted(&network_only));
    assert_eq!(opened.code.as_deref(), Some("not-permitted"));
    assert_eq!(launcher.plans().len(), 0);
}

#[test]
fn only_pages_on_the_github_host_are_opened() {
    let fixture = fixture();
    let settings = settings_for("https://ghe.example.com/api/v3", false, 20);
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = handler_with(OPEN, &settings, &(launcher.clone() as Arc<dyn Launcher>));
    let run = |parameters: &ActionParameters| {
        let parameters = handler.definition().normalize(parameters).unwrap();
        handler.execute(&parameters, &fixture.context())
    };

    let pull = run(&repo("octo/site").with("pullRequest", ParameterValue::Integer(7)));
    assert!(pull.is_success(), "{}", pull.message);
    assert_eq!(pull.data["url"], "https://ghe.example.com/octo/site/pull/7");
    let repository = run(&repo("octo/site"));
    assert_eq!(repository.data["url"], "https://ghe.example.com/octo/site");
    let page = run(&ActionParameters::new().with(
        "url",
        text("https://ghe.example.com/octo/site/pull/9/checks"),
    ));
    assert!(page.is_success(), "{}", page.message);
    let plans = launcher.plans();
    assert_eq!(plans.len(), 3);
    assert!(
        plans[0]
            .args
            .iter()
            .any(|a| a == "https://ghe.example.com/octo/site/pull/7"),
        "{plans:?}"
    );

    for refused in [
        "https://github.com/octo/site",
        "http://ghe.example.com/octo/site",
        "https://ghe.example.com.evil.test/x",
        "file:///etc/passwd",
    ] {
        let result = run(&ActionParameters::new().with("url", text(refused)));
        assert_eq!(
            result.code.as_deref(),
            Some(codes::URL_NOT_ALLOWED),
            "{refused}"
        );
    }
    let nothing = run(&ActionParameters::new());
    assert_eq!(nothing.code.as_deref(), Some(codes::INVALID_REPOSITORY));
    assert_eq!(launcher.plans().len(), 3);

    // A test API on this computer has no https web host to open.
    let local = settings_for("http://127.0.0.1:9", true, 20);
    let handler = handler_with(OPEN, &local, &recording());
    let parameters = handler.definition().normalize(&repo("octo/site")).unwrap();
    let result = handler.execute(&parameters, &fixture.context());
    assert_eq!(result.code.as_deref(), Some(codes::URL_NOT_ALLOWED));
}

#[test]
fn problems_are_found_when_saving() {
    let fixture = fixture();
    let settings = settings_for(crate::DEFAULT_API_URL, false, 20);
    let launcher: Arc<dyn Launcher> = Arc::new(RecordingLauncher::new());
    let checks = handler_with(CHECKS, &settings, &launcher);
    let bad = repo("not a repo")
        .with("ref", text("a..b"))
        .with("credential", text("Bad Name"));
    assert_eq!(checks.check(&bad, &fixture.profile).len(), 3);
    let templated = repo("{{ steps.a.output.repo }}").with("ref", text("{{ steps.a.output.ref }}"));
    assert_eq!(
        checks.check(&templated, &fixture.profile),
        Vec::<String>::new()
    );

    let open = handler_with(OPEN, &settings, &launcher);
    assert_eq!(
        open.check(
            &ActionParameters::new().with("url", text("https://evil.example/")),
            &fixture.profile
        )
        .len(),
        1
    );
    assert_eq!(
        open.check(&ActionParameters::new(), &fixture.profile).len(),
        1
    );
    assert_eq!(
        open.check(
            &ActionParameters::new().with("url", text("https://github.com/octo/site")),
            &fixture.profile
        ),
        Vec::<String>::new()
    );
}

#[test]
fn check_outcomes() {
    let run = |status: &str, conclusion: Value| {
        check_run_outcome(&serde_json::json!({"status": status, "conclusion": conclusion}))
    };
    assert_eq!(run("completed", "success".into()), Outcome::Passed);
    assert_eq!(run("completed", "neutral".into()), Outcome::Passed);
    assert_eq!(run("completed", "skipped".into()), Outcome::Passed);
    for failed in [
        "failure",
        "cancelled",
        "timed_out",
        "action_required",
        "stale",
    ] {
        assert_eq!(run("completed", failed.into()), Outcome::Failed, "{failed}");
    }
    assert_eq!(run("queued", Value::Null), Outcome::Pending);
    assert_eq!(
        status_outcome(&serde_json::json!({"state": "error"})),
        Outcome::Failed
    );
    assert_eq!(overall(Counts::default()), "none");
    assert_eq!(
        overall(Counts {
            passed: 2,
            failed: 0,
            pending: 1
        }),
        "pending"
    );
}
