//! Requests go to a small Jira stand-in on 127.0.0.1 started by the test;
//! no other host is contacted and no real account is used.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use deckpilot_core::FeatureModule;
use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{
    CancellationToken, ExecutionControl, MachineConfig, MemoryCredentials, Os, Progress, Secret,
};

use super::*;
use crate::api::MAX_PAGES;

const CLOUD_SECRET: &str = "me@example.com:ATATT-s3cretTOKEN";
const CLOUD_TOKEN: &str = "ATATT-s3cretTOKEN";
const DC_TOKEN: &str = "dcPAT-s3cretTOKEN";

/// Request heads the server received.
type Seen = Arc<Mutex<Vec<String>>>;

struct Server {
    port: u16,
    seen: Seen,
    /// Stops the server when the test ends.
    _acceptor: local::Acceptor,
}

type Answer = (String, Vec<String>, Vec<u8>);

fn json(status: &str, body: &str) -> Answer {
    (
        status.to_owned(),
        vec!["Content-Type: application/json".into()],
        body.as_bytes().to_vec(),
    )
}

fn basic(secret: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(secret)
    )
}

fn issue(n: usize) -> String {
    format!(
        r#"{{"key":"ABC-{n}","fields":{{"summary":"Fix\nthing {n}","status":{{"name":"In Progress"}},"assignee":{}}}}}"#,
        if n == 2 {
            "null".to_owned()
        } else {
            r#"{"displayName":"Dev One"}"#.to_owned()
        }
    )
}

fn issues(count: usize) -> String {
    let all: Vec<String> = (1..=count).map(issue).collect();
    all.join(",")
}

fn search(query: &str, cloud: bool) -> Answer {
    let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let get = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    let jql = get("jql").unwrap_or_default();
    let size: usize = get("maxResults").and_then(|v| v.parse().ok()).unwrap_or(50);
    let counting = get("fields").as_deref() == Some("key");
    let token = get("nextPageToken");
    if jql.contains("filter = 404") {
        return json(
            "400 Bad Request",
            r#"{"errorMessages":["The value '404' does not exist for the field 'filter'."],"errors":{}}"#,
        );
    }
    if !cloud {
        return json(
            "200 OK",
            &format!(
                r#"{{"startAt":0,"maxResults":{size},"total":42,"issues":[{}]}}"#,
                issues(size.min(3))
            ),
        );
    }
    if jql.contains("filter = 1 ") {
        // Never ends.
        return json(
            "200 OK",
            &format!(
                r#"{{"issues":[{}],"nextPageToken":"more","isLast":false}}"#,
                issues(1)
            ),
        );
    }
    if jql.contains("filter = 2 ") {
        return json(
            "200 OK",
            &format!(r#"{{"issues":[{}],"isLast":true}}"#, issues(2)),
        );
    }
    let body = match (counting, token.as_deref()) {
        (false, _) => format!(
            r#"{{"issues":[{}],"nextPageToken":"t2","isLast":false}}"#,
            issues(size.min(3))
        ),
        (true, None) => format!(
            r#"{{"issues":[{}],"nextPageToken":"p2","isLast":false}}"#,
            issues(100)
        ),
        (true, Some(_)) => format!(r#"{{"issues":[{}],"isLast":true}}"#, issues(7)),
    };
    json("200 OK", &body)
}

fn route(path: &str, query: &str, authorization: &str) -> Answer {
    if path.ends_with("/issue/ABC-777") {
        // A server that repeats what it was sent.
        return json(
            "403 Forbidden",
            &format!(r#"{{"errorMessages":["refused {authorization}"]}}"#),
        );
    }
    let cloud = path.starts_with("/jira/rest/api/3/");
    let expected = if cloud {
        basic(CLOUD_SECRET)
    } else {
        format!("Bearer {DC_TOKEN}")
    };
    if authorization != expected {
        return json("401 Unauthorized", "{}");
    }
    let Some(path) = path
        .strip_prefix("/jira/rest/api/3")
        .or_else(|| path.strip_prefix("/jira/rest/api/2"))
    else {
        return json("404 Not Found", "{}");
    };
    match path {
        "/myself" if cloud => json(
            "200 OK",
            r#"{"accountId":"5b10ac8d82e05b22cc7d4ef5","displayName":"Me Myself","timeZone":"Europe/Warsaw","emailAddress":"me@example.com"}"#,
        ),
        "/myself" => json("200 OK", r#"{"name":"me","displayName":"Me DC"}"#),
        "/issue/ABC-1" => json(
            "200 OK",
            r#"{"key":"ABC-1","fields":{"summary":"Login\u202e fails","status":{"name":"In Review","statusCategory":{"key":"indeterminate"}},"issuetype":{"name":"Bug"},"priority":{"name":"High"},"assignee":{"displayName":"Dev One"},"updated":"2026-11-01T10:00:00.000+0000"}}"#,
        ),
        "/issue/ABC-404" => json(
            "404 Not Found",
            r#"{"errorMessages":["Issue does not exist or you do not have permission to see it."]}"#,
        ),
        "/issue/ABC-403" => json("403 Forbidden", r#"{"errorMessages":["No permission."]}"#),
        "/issue/ABC-429" => {
            let (status, mut headers, body) = json("429 Too Many Requests", "{}");
            headers.push("Retry-After: 30".into());
            (status, headers, body)
        }
        "/issue/ABC-500" => json("500 Internal Server Error", r#"{"message":"boom"}"#),
        "/issue/ABC-999" => {
            std::thread::sleep(Duration::from_secs(5));
            json("200 OK", "{}")
        }
        "/issue/ABC-888" => (
            "200 OK".into(),
            vec!["Content-Type: application/json".into()],
            vec![b' '; crate::api::MAX_RESPONSE_BYTES + 10],
        ),
        "/search/jql" if cloud => search(query, true),
        "/search" if !cloud => search(query, false),
        _ => json("404 Not Found", r#"{"errorMessages":["Not found"]}"#),
    }
}

fn respond(mut stream: TcpStream, seen: &Seen) {
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
    let (status, headers, body) = route(path, query, &authorization);
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
    let acceptor = local::Acceptor::start(move |stream, _| respond(stream, &shared));
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

    fn site(&self) -> String {
        format!("http://127.0.0.1:{}/jira", self.port)
    }

    fn cloud(&self) -> Arc<JiraSettings> {
        settings_for(&self.site(), "cloud", true, 20)
    }

    fn data_center(&self) -> Arc<JiraSettings> {
        Arc::new(JiraSettings {
            credential: "jira.dc".into(),
            ..(*settings_for(&self.site(), "data-center", true, 20)).clone()
        })
    }
}

fn settings_for(
    site: &str,
    deployment: &str,
    allow_private: bool,
    timeout: i64,
) -> Arc<JiraSettings> {
    Arc::new(crate::settings(
        &ActionParameters::new()
            .with(crate::SITE_URL, ParameterValue::Text(site.into()))
            .with(crate::DEPLOYMENT, ParameterValue::Choice(deployment.into()))
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
    let id = |name: &str| CredentialId::new(name).unwrap();
    Fixture {
        profile: Profile::new("Work"),
        machine: MachineConfig::new(Os::current()),
        credentials: MemoryCredentials::new()
            .with(id("jira.token"), Secret::new(CLOUD_SECRET))
            .with(id("jira.dc"), Secret::new(DC_TOKEN))
            .with(
                id("jira.old"),
                Secret::new("me@example.com:ATATT-revokedTOKEN"),
            )
            .with(id("jira.plain"), Secret::new("ATATT-withoutEmail")),
    }
}

impl Fixture {
    fn context(&self) -> ExecutionContext<'_> {
        ExecutionContext::new(&self.profile, None, &self.machine)
            .with_credentials(&self.credentials)
    }
}

fn recording() -> Arc<dyn Launcher> {
    Arc::new(RecordingLauncher::new())
}

fn handler_with(
    kind: &str,
    settings: &Arc<JiraSettings>,
    launcher: &Arc<dyn Launcher>,
) -> Box<dyn ActionHandler> {
    handlers(settings, launcher)
        .into_iter()
        .find(|h| h.definition().id.as_str() == kind)
        .unwrap()
}

fn run_with(
    kind: &str,
    settings: &Arc<JiraSettings>,
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

fn key(value: &str) -> ActionParameters {
    ActionParameters::new().with("issue", text(value))
}

fn filter(id: i64) -> ActionParameters {
    ActionParameters::new()
        .with("source", ParameterValue::Choice("filter".into()))
        .with("filterId", ParameterValue::Integer(id))
}

/// No secret, nor its Basic encoding, is in a result.
fn assert_no_secret(result: &ActionResult) {
    let shown = serde_json::to_string(result).unwrap();
    for secret in [
        CLOUD_TOKEN,
        DC_TOKEN,
        "ATATT-revoked",
        basic(CLOUD_SECRET).trim_start_matches("Basic "),
    ] {
        assert!(!shown.contains(secret), "{shown}");
    }
}

/// The search of the last request.
fn jql(request: &str) -> String {
    let target = request.split_whitespace().nth(1).unwrap();
    let query = target.split_once('?').unwrap().1;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "jql")
        .unwrap()
        .1
        .into_owned()
}

#[test]
fn the_module_and_its_contract() {
    let module = crate::JiraModule::new();
    let manifest = module.manifest();
    assert_eq!(manifest.id.as_str(), "jira");
    assert_eq!(
        manifest.capabilities,
        [
            Capability::Network,
            Capability::Credentials,
            Capability::OpenExternal
        ]
    );
    let defaults = crate::settings(&ActionParameters::new());
    assert_eq!(defaults.site_url, "");
    assert_eq!(defaults.deployment, Deployment::Cloud);
    assert_eq!(defaults.credential, "jira.token");
    for key in [crate::SITE_URL, crate::DEPLOYMENT, crate::ALLOW_PRIVATE] {
        assert_eq!(
            manifest.portability.settings[key],
            deckpilot_core::SettingScope::MachineSpecific
        );
    }

    let settings = settings_for("https://team.atlassian.net", "cloud", false, 20);
    let all = handlers(&settings, &recording());
    let ids: Vec<_> = all.iter().map(|h| h.definition().id.to_string()).collect();
    assert_eq!(ids, [MYSELF, ISSUE, ISSUES, OPEN]);
    for handler in &all {
        let definition = handler.definition();
        let node = &definition.node;
        if definition.id.as_str() == OPEN {
            assert_eq!(node.effect, ActionEffect::Opens);
            assert!(definition.parameters.iter().all(|p| p.key != "credential"));
        } else {
            assert_eq!(node.effect, ActionEffect::ReadOnly, "{}", definition.id);
            assert!(node.cancellable && node.retry_safe);
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
    assert_eq!(sensitive(ISSUE), ["summary"]);
    assert_eq!(sensitive(ISSUES), ["items"]);

    // Without a site nothing is sent.
    let fixture = fixture();
    let unset = Arc::new(crate::settings(&ActionParameters::new()));
    let result = run_with(MYSELF, &unset, &ActionParameters::new(), &fixture.context());
    assert_eq!(result.code.as_deref(), Some(codes::INVALID_SITE_URL));
}

#[test]
fn cloud_and_data_center_authentication() {
    let server = server();
    let fixture = fixture();
    let cloud = run_with(
        MYSELF,
        &server.cloud(),
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert!(cloud.is_success(), "{}", cloud.message);
    assert_eq!(cloud.data["displayName"], "Me Myself");
    assert_eq!(cloud.data["account"], "5b10ac8d82e05b22cc7d4ef5");
    assert_eq!(cloud.data["timeZone"], "Europe/Warsaw");
    // The e-mail address is not an output.
    assert!(
        !serde_json::to_string(&cloud)
            .unwrap()
            .contains("me@example.com")
    );
    let request = server.last();
    assert!(
        request.starts_with("GET /jira/rest/api/3/myself "),
        "{request}"
    );
    assert!(
        request.contains(&format!("authorization: {}", basic(CLOUD_SECRET))),
        "{request}"
    );

    let dc = run_with(
        MYSELF,
        &server.data_center(),
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert!(dc.is_success(), "{}", dc.message);
    assert_eq!(dc.data["account"], "me");
    let request = server.last();
    assert!(
        request.starts_with("GET /jira/rest/api/2/myself "),
        "{request}"
    );
    assert!(request.contains(&format!("authorization: Bearer {DC_TOKEN}")));
    assert_no_secret(&cloud);
    assert_no_secret(&dc);
}

#[test]
fn private_sites_need_permission() {
    let server = server();
    let fixture = fixture();
    let strict = settings_for(&server.site(), "cloud", false, 20);
    let refused = run_with(
        MYSELF,
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
    assert!(refused.message.contains("Settings › Modules › Jira"));
    assert_eq!(server.seen().len(), 0, "nothing reached the server");

    let insecure = settings_for("http://jira.example.com", "cloud", true, 20);
    let result = run_with(
        MYSELF,
        &insecure,
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert_eq!(result.code.as_deref(), Some(codes::INVALID_SITE_URL));
}

#[test]
fn issue_information() {
    let server = server();
    let fixture = fixture();
    let result = run_with(ISSUE, &server.cloud(), &key("abc-1"), &fixture.context());
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["key"], "ABC-1");
    assert_eq!(result.data["summary"], "Login  fails");
    assert_eq!(result.data["status"], "In Review");
    assert_eq!(result.data["statusCategory"], "in-progress");
    assert_eq!(result.data["type"], "Bug");
    assert_eq!(result.data["priority"], "High");
    assert_eq!(result.data["assignee"], "Dev One");
    assert_eq!(
        result.data["url"],
        format!("http://127.0.0.1:{}/jira/browse/ABC-1", server.port)
    );
    // The summary stays out of the message, which is kept in history.
    assert!(!result.message.contains("Login"));
    assert!(
        server
            .last()
            .starts_with("GET /jira/rest/api/3/issue/ABC-1?fields=")
    );

    let invalid = run_with(
        ISSUE,
        &server.cloud(),
        &key("ABC-1/../x"),
        &fixture.context(),
    );
    assert_eq!(invalid.code.as_deref(), Some(codes::INVALID_ISSUE_KEY));
}

#[test]
fn jira_failures_have_codes() {
    let server = server();
    let fixture = fixture();
    let settings = server.cloud();
    let run = |name: &str| run_with(ISSUE, &settings, &key(name), &fixture.context());

    let missing = run("ABC-404");
    assert_eq!(missing.code.as_deref(), Some(codes::NOT_FOUND));
    assert!(missing.message.contains("ABC-404"));
    let forbidden = run("ABC-403");
    assert_eq!(forbidden.code.as_deref(), Some(codes::FORBIDDEN));
    assert!(forbidden.message.contains("No permission"));
    let limited = run("ABC-429");
    assert_eq!(limited.code.as_deref(), Some(codes::RATE_LIMITED));
    assert_eq!(limited.data["retryAfterSeconds"], "30");
    let broken = run("ABC-500");
    assert_eq!(broken.code.as_deref(), Some(codes::JIRA_ERROR));
    assert!(broken.message.contains("500") && broken.message.contains("boom"));
    let big = run("ABC-888");
    assert_eq!(big.code.as_deref(), Some(codes::INVALID_RESPONSE));
    let unknown_filter = run_with(ISSUES, &settings, &filter(404), &fixture.context());
    assert_eq!(unknown_filter.code.as_deref(), Some(codes::JIRA_ERROR));
    assert!(unknown_filter.message.contains("does not exist"));
    for result in [
        &missing,
        &forbidden,
        &limited,
        &broken,
        &big,
        &unknown_filter,
    ] {
        assert_no_secret(result);
    }
}

#[test]
fn credentials_missing_refused_or_malformed() {
    let server = server();
    let fixture = fixture();
    let settings = server.cloud();
    let with = |name: &str| ActionParameters::new().with("credential", text(name));
    let revoked = run_with(MYSELF, &settings, &with("jira.old"), &fixture.context());
    assert_eq!(revoked.code.as_deref(), Some(codes::UNAUTHORIZED));
    assert!(revoked.message.contains("jira.old"));
    assert_no_secret(&revoked);
    let before = server.seen().len();

    let missing = run_with(MYSELF, &settings, &with("jira.other"), &fixture.context());
    assert_eq!(missing.code.as_deref(), Some("credential-missing"));
    // Cloud needs email:api-token.
    let plain = run_with(MYSELF, &settings, &with("jira.plain"), &fixture.context());
    assert_eq!(plain.code.as_deref(), Some("credential-missing"));
    assert!(plain.message.contains("email:api-token"));
    assert!(!plain.message.contains("withoutEmail"));
    // Refused when saved (a credential parameter) and, should it get
    // through, when run.
    let handler = handler_with(MYSELF, &settings, &recording());
    assert!(handler.definition().normalize(&with("Not A Name")).is_err());
    let invalid = handler.execute(&with("Not A Name"), &fixture.context());
    assert_eq!(invalid.code.as_deref(), Some("invalid-parameters"));
    assert_eq!(
        server.seen().len(),
        before,
        "nothing sent without a credential"
    );
}

#[test]
fn a_secret_repeated_by_the_server_is_redacted() {
    let server = server();
    let fixture = fixture();
    let result = run_with(ISSUE, &server.cloud(), &key("ABC-777"), &fixture.context());
    assert_eq!(result.code.as_deref(), Some(codes::FORBIDDEN));
    assert!(result.message.contains("[redacted]"), "{}", result.message);
    assert_no_secret(&result);
    let dc = run_with(
        ISSUE,
        &server.data_center(),
        &key("ABC-777"),
        &fixture.context(),
    );
    assert!(dc.message.contains("[redacted]"), "{}", dc.message);
    assert_no_secret(&dc);
}

#[test]
fn issues_assigned_to_me_on_cloud() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        ISSUES,
        &server.cloud(),
        &ActionParameters::new()
            .with("project", text("web"))
            .with("limit", ParameterValue::Integer(2)),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    // No total on Cloud: counted from pages of 100 and 7.
    assert_eq!(result.data["count"], "107");
    assert_eq!(result.data["countExact"], "true");
    assert_eq!(result.data["listed"], "2");
    assert_eq!(result.data["truncated"], "true");
    assert_eq!(
        result.data["items"],
        "ABC-1 Fix thing 1 [In Progress] (@Dev One)\nABC-2 Fix thing 2 [In Progress] (unassigned)"
    );
    assert_eq!(result.data["firstKey"], "ABC-1");
    assert!(result.data["firstUrl"].ends_with("/jira/browse/ABC-1"));
    assert!(!result.message.contains("Fix"));
    let seen = server.seen();
    let listing = seen.iter().find(|r| r.contains("fields=summary")).unwrap();
    assert!(
        listing.starts_with("GET /jira/rest/api/3/search/jql?"),
        "{listing}"
    );
    assert_eq!(
        jql(listing),
        "assignee = currentUser() AND project = \"WEB\" AND statusCategory != Done ORDER BY \
         updated DESC"
    );
    assert!(server.last().contains("nextPageToken=p2"));

    let done = run_with(
        ISSUES,
        &server.cloud(),
        &filter(2).with("includeDone", ParameterValue::Boolean(true)),
        &fixture.context(),
    );
    assert!(done.is_success(), "{}", done.message);
    assert_eq!(done.data["count"], "2");
    assert_eq!(jql(&server.last()), "filter = 2 ORDER BY updated DESC");

    let injected = run_with(
        ISSUES,
        &server.cloud(),
        &ActionParameters::new().with("project", text("WEB\" OR project = SECRET")),
        &fixture.context(),
    );
    assert_eq!(injected.code.as_deref(), Some(codes::INVALID_PROJECT));
    let no_filter = run_with(
        ISSUES,
        &server.cloud(),
        &ActionParameters::new().with("source", ParameterValue::Choice("filter".into())),
        &fixture.context(),
    );
    assert_eq!(no_filter.code.as_deref(), Some("invalid-parameters"));
}

#[test]
fn filters_on_data_center_use_the_total() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        ISSUES,
        &server.data_center(),
        &filter(12345),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["count"], "42");
    assert_eq!(result.data["countExact"], "true");
    assert_eq!(result.data["listed"], "3");
    let request = server.last();
    assert!(
        request.starts_with("GET /jira/rest/api/2/search?"),
        "{request}"
    );
    assert_eq!(
        jql(&request),
        "filter = 12345 AND statusCategory != Done ORDER BY updated DESC"
    );
    assert_eq!(
        server
            .seen()
            .iter()
            .filter(|r| r.contains("/search"))
            .count(),
        1
    );
}

#[test]
fn counting_pages_is_bounded() {
    let server = server();
    let fixture = fixture();
    let result = run_with(ISSUES, &server.cloud(), &filter(1), &fixture.context());
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["countExact"], "false");
    assert_eq!(result.data["count"], MAX_PAGES.to_string());
    assert!(result.message.starts_with("at least"));
    let counting = server
        .seen()
        .iter()
        .filter(|r| r.contains("fields=key"))
        .count();
    assert_eq!(counting, MAX_PAGES);
}

#[test]
fn time_limits_and_cancellation() {
    let server = server();
    let fixture = fixture();
    let quick = settings_for(&server.site(), "cloud", true, 1);
    let begun = Instant::now();
    let slow = run_with(ISSUE, &quick, &key("ABC-999"), &fixture.context());
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
    let stopped = run_with(ISSUE, &server.cloud(), &key("ABC-999"), &context);
    canceller.join().unwrap();
    assert_eq!(
        stopped.code.as_deref(),
        Some("cancelled"),
        "{}",
        stopped.message
    );
    assert!(begun.elapsed() < Duration::from_secs(3));

    let before = server.seen().len();
    let again = run_with(ISSUE, &server.cloud(), &key("ABC-1"), &context);
    assert_eq!(again.code.as_deref(), Some("cancelled"));
    assert_eq!(server.seen().len(), before);
}

#[test]
fn capabilities_are_required() {
    let server = server();
    let fixture = fixture();
    let credentials_only = BTreeSet::from([Capability::Credentials]);
    let denied = run_with(
        MYSELF,
        &server.cloud(),
        &ActionParameters::new(),
        &fixture.context().restricted(&credentials_only),
    );
    assert_eq!(denied.code.as_deref(), Some("not-permitted"));
    let network_only = BTreeSet::from([Capability::Network]);
    let no_secret = run_with(
        MYSELF,
        &server.cloud(),
        &ActionParameters::new(),
        &fixture.context().restricted(&network_only),
    );
    assert_eq!(no_secret.code.as_deref(), Some("not-permitted"));
    assert_eq!(server.seen().len(), 0);

    let settings = settings_for("https://team.atlassian.net", "cloud", false, 20);
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = handler_with(OPEN, &settings, &(launcher.clone() as Arc<dyn Launcher>));
    let parameters = handler.definition().normalize(&key("ABC-1")).unwrap();
    let opened = handler.execute(&parameters, &fixture.context().restricted(&network_only));
    assert_eq!(opened.code.as_deref(), Some("not-permitted"));
    assert_eq!(launcher.plans().len(), 0);
}

#[test]
fn only_pages_on_the_jira_site_are_opened() {
    let fixture = fixture();
    let settings = settings_for("https://intra.example.com/jira", "data-center", false, 20);
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = handler_with(OPEN, &settings, &(launcher.clone() as Arc<dyn Launcher>));
    let run = |parameters: &ActionParameters| {
        let parameters = handler.definition().normalize(parameters).unwrap();
        handler.execute(&parameters, &fixture.context())
    };
    let issue = run(&key("abc-7"));
    assert!(issue.is_success(), "{}", issue.message);
    assert_eq!(
        issue.data["url"],
        "https://intra.example.com/jira/browse/ABC-7"
    );
    let page = run(&ActionParameters::new().with(
        "url",
        text("https://intra.example.com/jira/issues/?filter=12345"),
    ));
    assert!(page.is_success(), "{}", page.message);
    assert_eq!(launcher.plans().len(), 2);
    for refused in [
        "https://team.atlassian.net/browse/ABC-1",
        "http://intra.example.com/jira/browse/ABC-1",
        "https://intra.example.com/other",
        "file:///etc/passwd",
    ] {
        let result = run(&ActionParameters::new().with("url", text(refused)));
        assert_eq!(
            result.code.as_deref(),
            Some(codes::URL_NOT_ALLOWED),
            "{refused}"
        );
    }
    assert_eq!(
        run(&key("not a key")).code.as_deref(),
        Some(codes::INVALID_ISSUE_KEY)
    );
    assert_eq!(launcher.plans().len(), 2);
}

#[test]
fn problems_are_found_when_saving() {
    let fixture = fixture();
    let settings = settings_for("https://team.atlassian.net", "cloud", false, 20);
    let launcher = recording();
    let list = handler_with(ISSUES, &settings, &launcher);
    let bad = ActionParameters::new()
        .with("project", text("A B"))
        .with("credential", text("Bad Name"))
        .with("source", ParameterValue::Choice("filter".into()));
    assert_eq!(list.check(&bad, &fixture.profile).len(), 3);
    let templated = ActionParameters::new().with("project", text("{{ steps.a.output.key }}"));
    assert_eq!(
        list.check(&templated, &fixture.profile),
        Vec::<String>::new()
    );
    let open = handler_with(OPEN, &settings, &launcher);
    assert_eq!(open.check(&key("x"), &fixture.profile).len(), 1);
    assert_eq!(
        open.check(&ActionParameters::new(), &fixture.profile).len(),
        1
    );
}

#[test]
fn searches_and_categories() {
    assert_eq!(
        search_jql(None, None, false),
        "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC"
    );
    assert_eq!(
        search_jql(Some(7), Some("WEB"), true),
        "filter = 7 AND project = \"WEB\" ORDER BY updated DESC"
    );
    let category = |key: &str| {
        status_category(
            &serde_json::json!({"fields": {"status": {"statusCategory": {"key": key}}}}),
        )
    };
    assert_eq!(category("new"), "todo");
    assert_eq!(category("done"), "done");
    assert_eq!(category("other"), "unknown");
}
