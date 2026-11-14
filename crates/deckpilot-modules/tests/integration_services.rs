//! The GitHub, GitLab and Jira modules against stand-ins on this computer:
//! what happens when the service is unreachable, slow, refuses the token,
//! limits the rate, pages its lists, changes the shape of its answers or
//! repeats the token back, and when the module is turned off.
//!
//! Every server is a small HTTP (or HTTPS, with a self-signed certificate)
//! listener on 127.0.0.1 started by the test. No other host is contacted
//! and no real account or token is used; the actions run through the
//! module registry exactly as the agent runs them.

use std::io::{BufRead, BufReader, Read, Write};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use deckpilot_core::{
    ActionParameters, ActionRegistry, ActionResult, ActionTypeId, CancellationToken, CredentialId,
    ExecutionContext, ExecutionControl, MachineConfig, MemoryCredentials, ModuleId, ModuleRegistry,
    Os, ParameterValue, Profile, Progress, Secret,
};
use deckpilot_modules::builtin;

const GITHUB_TOKEN: &str = "ghp_INTEGRATIONsecretAAAA1111";
const GITLAB_TOKEN: &str = "glpat-INTEGRATIONsecretBBBB2222";
const JIRA_TOKEN: &str = "ATATT-INTEGRATIONsecretCCCC3333";
const JIRA_EMAIL: &str = "tester@example.com";

/// A self-signed certificate for 127.0.0.1 and localhost and its key,
/// made only for these tests and trusted by nothing.
const TEST_CERT: &[u8] = include_bytes!("fixtures/self-signed-cert.der");
const TEST_KEY: &[u8] = include_bytes!("fixtures/self-signed-key.der");

fn jira_credential() -> String {
    format!("{JIRA_EMAIL}:{JIRA_TOKEN}")
}

/// Every form in which a secret could leak.
fn secrets() -> Vec<String> {
    vec![
        GITHUB_TOKEN.to_owned(),
        GITLAB_TOKEN.to_owned(),
        JIRA_TOKEN.to_owned(),
        base64::engine::general_purpose::STANDARD.encode(jira_credential()),
    ]
}

fn assert_no_secret(result: &ActionResult) {
    let shown = serde_json::to_string(result).unwrap();
    for secret in secrets() {
        assert!(!shown.contains(&secret), "a secret leaked: {shown}");
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// --- The stand-in server -------------------------------------------------

/// A request as the server saw it.
#[derive(Debug, Clone)]
struct Seen {
    /// The server's own port, for links to further pages.
    port: u16,
    path: String,
    query: String,
    /// The request line and headers.
    head: String,
}

impl Seen {
    fn param(&self, name: &str) -> Option<String> {
        url::form_urlencoded::parse(self.query.as_bytes())
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.into_owned())
    }
}

/// What the server answers.
#[derive(Debug, Clone)]
struct Reply {
    status: u16,
    headers: Vec<String>,
    body: Vec<u8>,
    /// Wait before answering at all.
    delay: Duration,
    /// Send the head and the first byte of the body, then wait this long.
    stall_body: Duration,
}

impl Reply {
    fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            delay: Duration::ZERO,
            stall_body: Duration::ZERO,
        }
    }

    fn json(status: u16, body: &str) -> Self {
        Self::new(status, body.as_bytes().to_vec()).header("Content-Type", "application/json")
    }

    fn header(mut self, name: &str, value: impl std::fmt::Display) -> Self {
        self.headers.push(format!("{name}: {value}"));
        self
    }
}

type Handler = Arc<dyn Fn(&Seen) -> Reply + Send + Sync>;

struct Server {
    port: u16,
    tls: bool,
    seen: Arc<Mutex<Vec<Seen>>>,
    /// Stops the server when the test ends.
    _acceptor: local::Acceptor,
}

impl Server {
    fn start(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Self {
        Self::spawn(Arc::new(handler), false)
    }

    /// The same, over TLS with the self-signed test certificate.
    fn start_tls(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Self {
        Self::spawn(Arc::new(handler), true)
    }

    fn spawn(handler: Handler, tls: bool) -> Self {
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let shared = seen.clone();
        let config = tls.then(tls_config);
        let acceptor = local::Acceptor::start(move |stream, port| match config.clone() {
            Some(config) => {
                let Ok(connection) = rustls::ServerConnection::new(config) else {
                    return;
                };
                respond(
                    rustls::StreamOwned::new(connection, stream),
                    port,
                    &shared,
                    &handler,
                );
            }
            None => respond(stream, port, &shared, &handler),
        });
        Self {
            port: acceptor.port,
            tls,
            seen,
            _acceptor: acceptor,
        }
    }

    fn url(&self, path: &str) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        format!("{scheme}://127.0.0.1:{}{path}", self.port)
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
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

fn tls_config() -> Arc<rustls::ServerConfig> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(TEST_CERT.to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(TEST_KEY.to_vec())),
        )
        .unwrap();
    Arc::new(config)
}

fn respond<S: Read + Write>(stream: S, port: u16, seen: &Mutex<Vec<Seen>>, handler: &Handler) {
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        // A failed TLS handshake ends here, before any request is read.
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line == "\r\n" {
            break;
        }
        head.push_str(&line);
    }
    let target = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let request = Seen {
        port,
        path: path.to_owned(),
        query: query.to_owned(),
        head,
    };
    seen.lock().unwrap().push(request.clone());
    let reply = handler(&request);
    std::thread::sleep(reply.delay);
    let mut response = format!(
        "HTTP/1.1 {} Stand-in\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for header in &reply.headers {
        response.push_str(header);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    let stream = reader.get_mut();
    let _ = stream.write_all(response.as_bytes());
    if reply.stall_body.is_zero() {
        let _ = stream.write_all(&reply.body);
    } else {
        let _ = stream.write_all(&reply.body[..1.min(reply.body.len())]);
        let _ = stream.flush();
        std::thread::sleep(reply.stall_body);
    }
    let _ = stream.flush();
}

/// A port on this computer that nothing listens on.
fn closed_port() -> u16 {
    let listener = local::bind();
    listener.local_addr().unwrap().port()
}

// --- The services ----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Service {
    Github,
    Gitlab,
    Jira,
}

const SERVICES: [Service; 3] = [Service::Github, Service::Gitlab, Service::Jira];

impl Service {
    fn module(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Jira => "jira",
        }
    }

    /// The action that reads the signed-in user.
    fn probe(self) -> &'static str {
        match self {
            Self::Github => "github.current-user",
            Self::Gitlab => "gitlab.current-user",
            Self::Jira => "jira.myself",
        }
    }

    fn url_key(self) -> &'static str {
        match self {
            Self::Jira => "siteUrl",
            _ => "apiUrl",
        }
    }

    /// The code of an error status the module has no own code for.
    fn error_code(self) -> &'static str {
        match self {
            Self::Github => "github-error",
            Self::Gitlab => "gitlab-error",
            Self::Jira => "jira-error",
        }
    }

    fn invalid_url_code(self) -> &'static str {
        match self {
            Self::Jira => "invalid-site-url",
            _ => "invalid-api-url",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Gitlab => "GitLab",
            Self::Jira => "Jira",
        }
    }

    /// The module's settings for a server at `url`.
    fn config(self, url: &str, allow_private: bool, timeout: i64) -> ActionParameters {
        let config = ActionParameters::new()
            .with(self.url_key(), ParameterValue::Text(url.into()))
            .with("allowPrivateHost", ParameterValue::Boolean(allow_private))
            .with("timeoutSeconds", ParameterValue::Integer(timeout));
        match self {
            Self::Jira => config.with("deployment", ParameterValue::Choice("cloud".into())),
            _ => config,
        }
    }

    /// Every reading action with parameters that pass validation.
    fn readings(self) -> Vec<(&'static str, ActionParameters)> {
        let text = |v: &str| ParameterValue::Text(v.into());
        let int = ParameterValue::Integer;
        match self {
            Self::Github => vec![
                ("github.current-user", ActionParameters::new()),
                (
                    "github.repository",
                    ActionParameters::new().with("repository", text("octo/site")),
                ),
                (
                    "github.pull-requests",
                    ActionParameters::new().with("repository", text("octo/site")),
                ),
                ("github.review-requests", ActionParameters::new()),
                (
                    "github.checks",
                    ActionParameters::new()
                        .with("repository", text("octo/site"))
                        .with("ref", text("main")),
                ),
                (
                    "github.checks",
                    ActionParameters::new()
                        .with("repository", text("octo/site"))
                        .with("pullRequest", int(7)),
                ),
            ],
            Self::Gitlab => vec![
                ("gitlab.current-user", ActionParameters::new()),
                (
                    "gitlab.project",
                    ActionParameters::new().with("project", text("group/site")),
                ),
                (
                    "gitlab.merge-requests",
                    ActionParameters::new().with("project", text("group/site")),
                ),
                ("gitlab.review-requests", ActionParameters::new()),
                (
                    "gitlab.pipeline",
                    ActionParameters::new()
                        .with("project", text("group/site"))
                        .with("ref", text("main")),
                ),
                (
                    "gitlab.pipeline",
                    ActionParameters::new()
                        .with("project", text("group/site"))
                        .with("mergeRequest", int(3)),
                ),
            ],
            Self::Jira => vec![
                ("jira.myself", ActionParameters::new()),
                (
                    "jira.issue",
                    ActionParameters::new().with("issue", text("ABC-1")),
                ),
                ("jira.issues", ActionParameters::new()),
                (
                    "jira.issues",
                    ActionParameters::new()
                        .with("source", ParameterValue::Choice("filter".into()))
                        .with("filterId", int(10)),
                ),
            ],
        }
    }
}

struct World {
    profile: Profile,
    machine: MachineConfig,
    credentials: MemoryCredentials,
}

fn world() -> World {
    World {
        profile: Profile::new("Work"),
        machine: MachineConfig::new(Os::current()),
        credentials: MemoryCredentials::new()
            .with(
                CredentialId::new("github.token").unwrap(),
                Secret::new(GITHUB_TOKEN),
            )
            .with(
                CredentialId::new("gitlab.token").unwrap(),
                Secret::new(GITLAB_TOKEN),
            )
            .with(
                CredentialId::new("jira.token").unwrap(),
                Secret::new(jira_credential()),
            ),
    }
}

impl World {
    fn context(&self) -> ExecutionContext<'_> {
        ExecutionContext::new(&self.profile, None, &self.machine)
            .with_credentials(&self.credentials)
    }
}

/// The built-in modules with `service` configured.
fn modules(service: Service, config: &ActionParameters) -> ModuleRegistry {
    let mut modules = builtin(Os::current());
    modules
        .set_config(&ModuleId::new(service.module()).unwrap(), config)
        .unwrap();
    modules
}

fn actions(service: Service, config: &ActionParameters) -> ActionRegistry {
    modules(service, config).action_registry()
}

/// Runs `action` as the agent does: normalized, through the registry
/// (which limits it to its module's capabilities), and without panicking.
fn run_in(
    registry: &ActionRegistry,
    action: &str,
    parameters: &ActionParameters,
    context: &ExecutionContext<'_>,
) -> ActionResult {
    let id = ActionTypeId::new(action).unwrap();
    let parameters = registry.normalize(&id, parameters).unwrap();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        registry.dispatch(&id, &parameters, context)
    }))
    .unwrap_or_else(|_| panic!("{action} panicked"));
    assert_no_secret(&result);
    result
}

fn probe(service: Service, config: &ActionParameters) -> ActionResult {
    let world = world();
    run_in(
        &actions(service, config),
        service.probe(),
        &ActionParameters::new(),
        &world.context(),
    )
}

fn code(result: &ActionResult) -> &str {
    result.code.as_deref().unwrap_or("<none>")
}

// --- Unavailable services ----------------------------------------------------

#[test]
fn a_refused_connection_is_reported() {
    let port = closed_port();
    for service in SERVICES {
        let url = format!("http://127.0.0.1:{port}/api");
        let result = probe(service, &service.config(&url, true, 10));
        assert_eq!(
            code(&result),
            "connection-failed",
            "{service:?}: {result:?}"
        );
        assert!(
            !result.message.contains("/api"),
            "no URL in the message: {}",
            result.message
        );
    }
}

#[test]
fn an_unknown_host_is_reported() {
    // `.invalid` never resolves (RFC 6761).
    for service in SERVICES {
        let result = probe(
            service,
            &service.config("https://deckpilot-test.invalid/api", false, 10),
        );
        assert_eq!(code(&result), "dns-failed", "{service:?}: {result:?}");
        assert!(result.message.contains("deckpilot-test.invalid"));
    }
}

#[test]
fn server_errors_are_reported_without_panicking() {
    let replies = [
        Reply::new(500, "<html><body>Internal error</body></html>")
            .header("Content-Type", "text/html"),
        Reply::new(502, ""),
        Reply::json(503, r#"{"message":"Service unavailable, maintenance"}"#),
        Reply::new(504, vec![0xff, 0xfe, 0x00]),
    ];
    for reply in replies {
        let status = reply.status;
        let server = Server::start(move |_| reply.clone());
        for service in SERVICES {
            let result = probe(service, &service.config(&server.url("/api"), true, 10));
            assert_eq!(code(&result), service.error_code(), "{service:?} {status}");
            assert!(
                result.message.contains(&status.to_string()),
                "{}",
                result.message
            );
            assert!(!result.message.contains("<html"), "{}", result.message);
        }
    }
}

// --- TLS --------------------------------------------------------------------------

#[test]
fn a_self_signed_certificate_is_refused_and_nothing_is_sent() {
    let server = Server::start_tls(|_| Reply::json(200, r#"{"login":"octocat"}"#));
    for service in SERVICES {
        let result = probe(service, &service.config(&server.url("/api"), true, 10));
        assert_eq!(code(&result), "tls-failed", "{service:?}: {result:?}");
    }
    assert_eq!(server.count(), 0, "no request got through the handshake");
}

#[test]
fn plain_text_on_an_https_address_is_refused() {
    // An https address whose server answers in plain HTTP at once,
    // without waiting for a request.
    let server = local::Acceptor::start(|mut stream, _| {
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
    });
    let port = server.port;
    for service in SERVICES {
        let url = format!("https://127.0.0.1:{port}/api");
        let result = probe(service, &service.config(&url, true, 10));
        assert!(!result.is_success(), "{service:?}: {result:?}");
        assert!(
            matches!(code(&result), "tls-failed" | "connection-failed"),
            "{service:?}: {result:?}"
        );
    }
}

// --- Addresses ---------------------------------------------------------------------

#[test]
fn invalid_and_insecure_addresses_are_refused_before_connecting() {
    for service in SERVICES {
        for url in [
            "http://api.example.com",
            "http://192.0.2.1/api",
            "ftp://api.example.com",
            "https://user:hunter2@api.example.com",
            "https://api.example.com/?token=x",
            "https://api.example.com/#x",
            "file:///etc/passwd",
            "api.example.com",
            "not a url",
        ] {
            let result = probe(service, &service.config(url, false, 10));
            assert_eq!(
                code(&result),
                service.invalid_url_code(),
                "{service:?} {url}: {result:?}"
            );
            assert!(!result.message.contains("hunter2"), "{}", result.message);
        }
    }
    // Jira has no default site.
    let world = world();
    let registry = builtin(Os::current()).action_registry();
    let unset = run_in(
        &registry,
        "jira.myself",
        &ActionParameters::new(),
        &world.context(),
    );
    assert_eq!(code(&unset), "invalid-site-url", "{unset:?}");
}

#[test]
fn hosts_on_this_computer_need_permission() {
    let server = Server::start(|_| Reply::json(200, "{}"));
    for service in SERVICES {
        for host in ["127.0.0.1", "localhost"] {
            let url = format!("http://{host}:{}/api", server.port);
            let result = probe(service, &service.config(&url, false, 10));
            assert_eq!(code(&result), "blocked-address", "{service:?}: {result:?}");
            assert!(
                result.message.contains(service.name()),
                "{}",
                result.message
            );
        }
    }
    assert_eq!(server.count(), 0);
}

#[test]
fn a_redirect_to_another_origin_does_not_take_the_token() {
    let target = Server::start(|_| {
        Reply::json(
            200,
            r#"{"login":"octocat","username":"octocat","id":1,"displayName":"Me","accountId":"a1"}"#,
        )
    });
    let elsewhere = target.url("/moved");
    let origin = Server::start(move |_| Reply::new(302, "").header("Location", &elsewhere));
    for service in SERVICES {
        // The other origin is reached without the token, so the result
        // may be anything; what matters is what that origin received.
        probe(service, &service.config(&origin.url("/api"), true, 10));
    }
    let received = target.seen();
    assert_eq!(received.len(), SERVICES.len());
    for request in received {
        assert!(
            !request.head.to_lowercase().contains("authorization")
                && !request.head.to_lowercase().contains("private-token"),
            "{}",
            request.head
        );
        for secret in secrets() {
            assert!(!request.head.contains(&secret), "{}", request.head);
        }
    }
}

// --- Authentication -------------------------------------------------------------------

#[test]
fn expired_or_revoked_tokens_are_unauthorized() {
    let server = Server::start(|seen| match seen.path.as_str() {
        p if p.contains("github") => Reply::json(401, r#"{"message":"Bad credentials"}"#),
        p if p.contains("gitlab") => Reply::json(
            401,
            r#"{"error":"invalid_token","error_description":"Token is expired. You can either do re-authorization or token refresh."}"#,
        ),
        _ => Reply::new(401, "").header("WWW-Authenticate", "Basic realm=\"jira\""),
    });
    for service in SERVICES {
        let url = server.url(&format!("/{}", service.module()));
        let result = probe(service, &service.config(&url, true, 10));
        assert_eq!(code(&result), "unauthorized", "{service:?}: {result:?}");
        assert!(
            result
                .message
                .contains(&format!("{}.token", service.module())),
            "names the credential: {}",
            result.message
        );
        assert!(result.message.contains("expired"), "{}", result.message);
    }
}

#[test]
fn forbidden_answers_keep_the_reason() {
    let server = Server::start(|_| {
        Reply::json(
            403,
            r#"{"message":"Resource protected by organization SAML enforcement","errorMessages":["Resource protected by organization SAML enforcement"]}"#,
        )
    });
    for service in SERVICES {
        let result = probe(service, &service.config(&server.url("/api"), true, 10));
        assert_eq!(code(&result), "forbidden", "{service:?}: {result:?}");
        assert!(result.message.contains("SAML"), "{}", result.message);
    }
}

// --- Rate limits ------------------------------------------------------------------

fn wait_of(result: &ActionResult) -> u64 {
    assert_eq!(code(result), "rate-limited", "{result:?}");
    assert!(
        result.message.contains("try again in"),
        "{}",
        result.message
    );
    result.data["retryAfterSeconds"].parse().unwrap()
}

#[test]
fn rate_limits_report_how_long_to_wait() {
    let server = Server::start(|seen| {
        let reset = now() + 90;
        match seen.path.as_str() {
            "/retry-after/api/user" | "/retry-after/api/rest/api/3/myself" => {
                Reply::json(429, "{}").header("Retry-After", 17)
            }
            "/github-primary/api/user" => {
                Reply::json(403, r#"{"message":"API rate limit exceeded"}"#)
                    .header("X-RateLimit-Limit", 5000)
                    .header("X-RateLimit-Remaining", 0)
                    .header("X-RateLimit-Reset", reset)
            }
            "/github-secondary/api/user" => Reply::json(
                403,
                r#"{"message":"You have exceeded a secondary rate limit. Please wait a few minutes before you try again."}"#,
            ),
            "/gitlab-reset/api/user" => Reply::json(429, r#"{"message":"Retry later"}"#)
                .header("RateLimit-Remaining", 0)
                .header("RateLimit-Reset", reset),
            _ => Reply::json(429, "{}"),
        }
    });
    for service in SERVICES {
        let url = server.url("/retry-after/api");
        assert_eq!(
            wait_of(&probe(service, &service.config(&url, true, 10))),
            17,
            "{service:?}"
        );
        // Without any hint: a minute.
        let url = server.url("/no-hint/api");
        assert_eq!(
            wait_of(&probe(service, &service.config(&url, true, 10))),
            60,
            "{service:?}"
        );
    }
    let primary = probe(
        Service::Github,
        &Service::Github.config(&server.url("/github-primary/api"), true, 10),
    );
    assert!((80..=90).contains(&wait_of(&primary)), "{primary:?}");
    let secondary = probe(
        Service::Github,
        &Service::Github.config(&server.url("/github-secondary/api"), true, 10),
    );
    assert_eq!(wait_of(&secondary), 60, "{secondary:?}");
    let reset = probe(
        Service::Gitlab,
        &Service::Gitlab.config(&server.url("/gitlab-reset/api"), true, 10),
    );
    assert!((80..=90).contains(&wait_of(&reset)), "{reset:?}");
}

// --- Answers of another shape ---------------------------------------------------------

/// Bodies a changed or broken API might answer with.
fn odd_bodies() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("empty", Vec::new()),
        ("null", b"null".to_vec()),
        ("a list", b"[]".to_vec()),
        ("a list of odd items", br#"[1,"x",null,{"id":"x"}]"#.to_vec()),
        ("an empty object", b"{}".to_vec()),
        ("text", br#""hello""#.to_vec()),
        ("a number", b"42".to_vec()),
        ("cut off", br#"{"login":"oc"#.to_vec()),
        ("html", b"<!doctype html><title>Sign in</title>".to_vec()),
        (
            "renamed fields",
            br#"{"user_login":"x","user_name":"x","display_name":"x","full":"x","issues_list":[],"results":[],"check_runs_list":[]}"#.to_vec(),
        ),
        (
            "wrong types",
            br#"{"login":42,"username":[],"displayName":{},"accountId":7,"id":"seven","name":1,"full_name":2,"path_with_namespace":3,"key":4,"total_count":"many","items":"none","check_runs":{},"statuses":"x","issues":{},"total":-1,"head":"x","head_pipeline":"x","status":5,"sha":6,"fields":[]}"#.to_vec(),
        ),
        (
            "nulls",
            br#"{"login":null,"username":null,"displayName":null,"accountId":null,"id":null,"full_name":null,"path_with_namespace":null,"key":null,"total_count":null,"items":null,"check_runs":null,"statuses":null,"issues":null,"total":null,"head":null,"head_pipeline":null,"fields":null}"#.to_vec(),
        ),
        ("deeply nested", {
            let mut deep = "[".repeat(100_000);
            deep.push_str(&"]".repeat(100_000));
            deep.into_bytes()
        }),
    ]
}

#[test]
fn answers_of_another_shape_fail_clearly_and_never_panic() {
    for (label, body) in odd_bodies() {
        let server = Server::start(move |_| {
            Reply::new(200, body.clone()).header("Content-Type", "application/json")
        });
        for service in SERVICES {
            let world = world();
            let registry = actions(service, &service.config(&server.url("/api"), true, 10));
            for (action, parameters) in service.readings() {
                let result = run_in(&registry, action, &parameters, &world.context());
                if !result.is_success() {
                    assert_eq!(
                        code(&result),
                        "invalid-response",
                        "{action} with {label}: {result:?}"
                    );
                    assert!(
                        result.message.contains(service.name()),
                        "{action} with {label}: {}",
                        result.message
                    );
                }
            }
            // Reading the user always needs the user's name.
            let user = run_in(
                &registry,
                service.probe(),
                &ActionParameters::new(),
                &world.context(),
            );
            assert_eq!(
                code(&user),
                "invalid-response",
                "{} with {label}: {user:?}",
                service.probe()
            );
        }
    }
}

#[test]
fn extra_fields_are_ignored() {
    let server = Server::start(|_| {
        Reply::json(
            200,
            r#"{"login":"octocat","username":"octocat","displayName":"Octo Cat","accountId":"a-1",
                "id":1,"name":"Octo Cat","added_in_a_later_version":{"nested":[1,2,3]},
                "plan":{"name":"pro","private_repos":9999},"another":null}"#,
        )
    });
    for service in SERVICES {
        let result = probe(service, &service.config(&server.url("/api"), true, 10));
        assert!(result.is_success(), "{service:?}: {result:?}");
        assert!(
            !serde_json::to_string(&result.data)
                .unwrap()
                .contains("later_version"),
            "{result:?}"
        );
    }
}

// --- Paging ---------------------------------------------------------------------------

#[test]
fn github_follows_link_headers_across_pages() {
    let server = Server::start(|seen| {
        let page = seen.param("page").unwrap_or_else(|| "1".into());
        let base = format!("http://127.0.0.1:{}", seen.port);
        match (seen.path.as_str(), page.as_str()) {
            ("/api/repos/octo/site/commits/main/check-runs", "1") => Reply::json(
                200,
                r#"{"total_count":3,"check_runs":[
                    {"name":"a","status":"completed","conclusion":"success"},
                    {"name":"b","status":"completed","conclusion":"success"}]}"#,
            )
            .header(
                "Link",
                format!(
                    "<{base}/api/repos/octo/site/commits/main/check-runs?per_page=100&filter=latest&page=2>; rel=\"next\", \
                     <{base}/api/repos/octo/site/commits/main/check-runs?per_page=100&filter=latest&page=2>; rel=\"last\""
                ),
            ),
            ("/api/repos/octo/site/commits/main/check-runs", "2") => Reply::json(
                200,
                r#"{"total_count":3,"check_runs":[
                    {"name":"c","status":"completed","conclusion":"failure"}]}"#,
            )
            .header(
                "Link",
                format!(
                    "<{base}/api/repos/octo/site/commits/main/check-runs?per_page=100&filter=latest&page=1>; rel=\"prev\""
                ),
            ),
            ("/api/repos/octo/site/commits/main/status", _) => Reply::json(
                200,
                r#"{"sha":"0123abc","total_count":1,"statuses":[{"context":"ci","state":"pending"}]}"#,
            ),
            _ => Reply::json(404, r#"{"message":"Not Found"}"#),
        }
    });
    let world = world();
    let registry = actions(
        Service::Github,
        &Service::Github.config(&server.url("/api"), true, 10),
    );
    let (_, parameters) = Service::Github.readings().swap_remove(4);
    let result = run_in(&registry, "github.checks", &parameters, &world.context());
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.data["total"], "4");
    assert_eq!(result.data["passed"], "2");
    assert_eq!(result.data["failed"], "1");
    assert_eq!(result.data["pending"], "1");
    assert_eq!(result.data["failing"], "c");
    assert_eq!(result.data["truncated"], "false");
    assert_eq!(result.data["sha"], "0123abc");
    let pages: Vec<_> = server
        .seen()
        .iter()
        .filter(|s| s.path.ends_with("/check-runs"))
        .map(|s| s.param("page"))
        .collect();
    assert_eq!(pages, [None, Some("2".into())]);
}

#[test]
fn gitlab_counts_pages_when_it_gives_no_total() {
    let server = Server::start(|seen| {
        let port = seen.port;
        let page: usize = seen.param("page").and_then(|p| p.parse().ok()).unwrap_or(1);
        let size: usize = seen
            .param("per_page")
            .and_then(|p| p.parse().ok())
            .unwrap_or(20);
        // 230 open merge requests, and no X-Total (as for large lists).
        let all = 230;
        let from = (page - 1) * size;
        let to = (from + size).min(all);
        let items: Vec<String> = (from..to)
            .map(|n| {
                format!(
                    r#"{{"iid":{n},"title":"MR {n}","references":{{"full":"group/site!{n}"}},"author":{{"username":"dev"}},"web_url":"https://gitlab.example/group/site/-/merge_requests/{n}"}}"#
                )
            })
            .collect();
        let mut reply = Reply::json(200, &format!("[{}]", items.join(",")));
        if to < all {
            reply = reply.header(
                "Link",
                format!(
                    "<http://127.0.0.1:{port}/api/projects/group%2Fsite/merge_requests?state=opened&per_page={size}&page={}>; rel=\"next\"",
                    page + 1
                ),
            );
        }
        reply
    });
    let world = world();
    let registry = actions(
        Service::Gitlab,
        &Service::Gitlab.config(&server.url("/api"), true, 10),
    );
    let result = run_in(
        &registry,
        "gitlab.merge-requests",
        &ActionParameters::new()
            .with("project", ParameterValue::Text("group/site".into()))
            .with("limit", ParameterValue::Integer(5)),
        &world.context(),
    );
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.data["listed"], "5");
    assert_eq!(result.data["count"], "230");
    assert_eq!(result.data["countExact"], "true");
    assert_eq!(result.data["truncated"], "true");
    // The first page of 5, then three pages of 100 to count.
    let pages: Vec<(Option<String>, Option<String>)> = server
        .seen()
        .iter()
        .map(|s| (s.param("per_page"), s.param("page")))
        .collect();
    assert_eq!(
        pages,
        [
            (Some("5".into()), None),
            (Some("100".into()), None),
            (Some("100".into()), Some("2".into())),
            (Some("100".into()), Some("3".into())),
        ]
    );
}

#[test]
fn jira_cloud_counts_pages_with_next_page_tokens() {
    let server = Server::start(|seen| {
        let size: usize = seen
            .param("maxResults")
            .and_then(|p| p.parse().ok())
            .unwrap_or(50);
        let token = seen.param("nextPageToken");
        // 150 issues: 100 on the first counting page, 50 on the second.
        let (from, to, next) = match (size, token.as_deref()) {
            (100, None) => (0, 100, Some("page-2")),
            (100, Some("page-2")) => (100, 150, None),
            (_, None) => (0, size, Some("page-2-small")),
            _ => return Reply::json(400, r#"{"errorMessages":["bad token"]}"#),
        };
        let issues: Vec<String> = (from..to)
            .map(|n| {
                format!(
                    r#"{{"key":"ABC-{n}","fields":{{"summary":"Issue {n}","status":{{"name":"Open"}}}}}}"#,
                    n = n + 1
                )
            })
            .collect();
        let next = next.map_or_else(
            || r#""isLast":true"#.to_owned(),
            |t| format!(r#""nextPageToken":"{t}","isLast":false"#),
        );
        Reply::json(
            200,
            &format!(r#"{{"issues":[{}],{next}}}"#, issues.join(",")),
        )
    });
    let world = world();
    let registry = actions(
        Service::Jira,
        &Service::Jira.config(&server.url("/jira"), true, 10),
    );
    let result = run_in(
        &registry,
        "jira.issues",
        &ActionParameters::new().with("limit", ParameterValue::Integer(3)),
        &world.context(),
    );
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.data["listed"], "3");
    assert_eq!(result.data["count"], "150");
    assert_eq!(result.data["countExact"], "true");
    assert_eq!(result.data["firstKey"], "ABC-1");
    let seen = server.seen();
    assert!(seen.iter().all(|s| s.path == "/jira/rest/api/3/search/jql"));
    assert_eq!(seen.len(), 3);
}

// --- Time limits and cancellation -----------------------------------------------------

#[test]
fn slow_servers_are_bounded_by_the_time_limit() {
    let slow_head = Server::start(|_| {
        let mut reply = Reply::json(200, "{}");
        reply.delay = Duration::from_secs(6);
        reply
    });
    let slow_body = Server::start(|_| {
        let mut reply = Reply::json(200, r#"{"login":"octocat","username":"octocat"}"#);
        reply.stall_body = Duration::from_secs(6);
        reply
    });
    for server in [&slow_head, &slow_body] {
        for service in SERVICES {
            let begun = Instant::now();
            let result = probe(service, &service.config(&server.url("/api"), true, 1));
            assert_eq!(code(&result), "timed-out", "{service:?}: {result:?}");
            assert!(
                begun.elapsed() < Duration::from_secs(4),
                "{service:?} took {:?}",
                begun.elapsed()
            );
        }
    }
}

#[test]
fn cancelling_stops_a_request_at_once() {
    let server = Server::start(|_| {
        let mut reply = Reply::json(200, "{}");
        reply.delay = Duration::from_secs(6);
        reply
    });
    for service in SERVICES {
        let world = world();
        let registry = actions(service, &service.config(&server.url("/api"), true, 30));
        let token = CancellationToken::new();
        let progress = |_: Progress| {};
        let control = ExecutionControl::new(&token, &progress);
        let context = world.context().with_control(&control);
        let canceller = {
            let token = token.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                token.cancel();
            })
        };
        let begun = Instant::now();
        let result = run_in(
            &registry,
            service.probe(),
            &ActionParameters::new(),
            &context,
        );
        canceller.join().unwrap();
        assert_eq!(code(&result), "cancelled", "{service:?}: {result:?}");
        assert!(begun.elapsed() < Duration::from_secs(3));
    }
}

// --- Secrets ---------------------------------------------------------------------------

#[test]
fn a_server_that_repeats_the_token_never_gets_it_into_a_result() {
    // Echoes the request head (with the token) into every field an action
    // reads, in successful answers and in errors.
    let server = Server::start(|seen| {
        let echo = serde_json::to_string(&seen.head).unwrap();
        let status = if seen.path.starts_with("/fail") {
            500
        } else {
            200
        };
        let body = format!(
            r#"{{"login":{echo},"username":{echo},"name":{echo},"displayName":{echo},"accountId":{echo},
                "timeZone":{echo},"web_url":{echo},"html_url":{echo},"full_name":{echo},
                "path_with_namespace":{echo},"description":{echo},"key":"ABC-1","id":1,
                "message":{echo},"errorMessages":[{echo}],"error_description":{echo},
                "fields":{{"summary":{echo},"status":{{"name":{echo}}},"assignee":{{"displayName":{echo}}}}},
                "total_count":1,"items":[{{"title":{echo},"html_url":{echo},"user":{{"login":{echo}}}}}],
                "issues":[{{"key":"ABC-1","fields":{{"summary":{echo}}}}}],"total":1,"isLast":true}}"#
        );
        Reply::json(status, &body)
    });
    for prefix in ["/ok", "/fail"] {
        for service in SERVICES {
            let world = world();
            let url = server.url(&format!("{prefix}/api"));
            let registry = actions(service, &service.config(&url, true, 10));
            for (action, parameters) in service.readings() {
                // `run_in` checks every result for every secret.
                let result = run_in(&registry, action, &parameters, &world.context());
                if prefix == "/fail" {
                    assert!(!result.is_success(), "{action}: {result:?}");
                }
            }
        }
    }
    // The token did reach the server: the check above is not vacuous.
    let heads: String = server.seen().iter().map(|s| s.head.clone()).collect();
    assert!(heads.contains(GITHUB_TOKEN));
    assert!(heads.contains(GITLAB_TOKEN));
    assert!(heads.contains(&base64::engine::general_purpose::STANDARD.encode(jira_credential())));
}

// --- Turning modules off ----------------------------------------------------------------

#[test]
fn a_disabled_module_sends_nothing_and_says_why() {
    let server = Server::start(|_| {
        Reply::json(
            200,
            r#"{"login":"octocat","username":"octocat","displayName":"Me","accountId":"a1"}"#,
        )
    });
    for service in SERVICES {
        let world = world();
        let id = ModuleId::new(service.module()).unwrap();
        let mut registry = modules(service, &service.config(&server.url("/api"), true, 10));
        assert!(registry.is_enabled(&id), "{service:?} is on by default");
        registry.set_enabled(&id, false).unwrap();
        let before = server.count();
        let actions = registry.action_registry();
        let action = ActionTypeId::new(service.probe()).unwrap();
        assert!(
            actions
                .normalize(&action, &ActionParameters::new())
                .is_err()
        );
        let result = actions.dispatch(&action, &ActionParameters::new(), &world.context());
        assert!(!result.is_success());
        assert!(
            result.message.contains("disabled") && result.message.contains(service.name()),
            "{}",
            result.message
        );
        assert_eq!(server.count(), before, "{service:?} sent a request");

        // Turned on again, with its settings kept.
        registry.set_enabled(&id, true).unwrap();
        let result = run_in(
            &registry.action_registry(),
            service.probe(),
            &ActionParameters::new(),
            &world.context(),
        );
        assert!(result.is_success(), "{service:?}: {result:?}");
        assert_eq!(server.count(), before + 1);
    }
}
