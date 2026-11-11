//! Requests go to a small server on 127.0.0.1 started by the test; no
//! other host is contacted.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionParameters, CancellationToken, CredentialId, ExecutionContext, ExecutionControl,
    MachineConfig, MemoryCredentials, Os, ParameterValue, Profile, Progress, Secret,
};

use super::*;

/// Request heads the server received.
type Seen = Arc<Mutex<Vec<String>>>;

struct Server {
    port: u16,
    seen: Seen,
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
    let length = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    head.push_str(&String::from_utf8_lossy(&body));
    seen.lock().unwrap().push(head.clone());
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let path = path.split('?').next().unwrap_or("/").to_owned();
    let (status, headers, body): (&str, Vec<String>, Vec<u8>) = match path.as_str() {
        "/json" => (
            "200 OK",
            vec![
                "Content-Type: application/json".into(),
                "Set-Cookie: session=abc123".into(),
            ],
            br#"{"data":[{"id":7,"name":"Shop"}]}"#.to_vec(),
        ),
        "/health" => (
            "200 OK",
            vec!["Content-Type: text/plain".into()],
            b"status: ok".to_vec(),
        ),
        "/fail" => ("500 Internal Server Error", Vec::new(), b"boom".to_vec()),
        "/same" => ("302 Found", vec!["Location: /json".into()], Vec::new()),
        "/other" => (
            "302 Found",
            vec![format!("Location: http://localhost:{port}/json")],
            Vec::new(),
        ),
        "/loop" => ("302 Found", vec!["Location: /loop".into()], Vec::new()),
        "/big" => (
            "200 OK",
            vec!["Content-Type: text/plain".into()],
            vec![b'x'; 2 * 1024 * 1024],
        ),
        "/slow" => {
            std::thread::sleep(Duration::from_secs(5));
            ("200 OK", Vec::new(), b"late".to_vec())
        }
        _ => ("404 Not Found", Vec::new(), b"missing".to_vec()),
    };
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

fn server() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let shared = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let seen = shared.clone();
            std::thread::spawn(move || respond(stream, &seen, port));
        }
    });
    Server { port, seen }
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
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
                CredentialId::new("api.token").unwrap(),
                Secret::new("s3cret-token"),
            )
            .with(
                CredentialId::new("api.basic").unwrap(),
                Secret::new("bob:pa55"),
            ),
    }
}

impl Fixture {
    fn context(&self) -> ExecutionContext<'_> {
        ExecutionContext::new(&self.profile, None, &self.machine)
            .with_credentials(&self.credentials)
    }
}

fn local_settings(hosts: &[&str]) -> Arc<HttpSettings> {
    Arc::new(crate::settings(&ActionParameters::new().with(
        crate::PRIVATE_HOSTS,
        ParameterValue::Text(hosts.join("\n")),
    )))
}

fn handler(kind: &str, settings: &Arc<HttpSettings>) -> Box<dyn ActionHandler> {
    handlers(settings)
        .into_iter()
        .find(|h| h.definition().id.as_str() == kind)
        .unwrap()
}

fn run_with(
    kind: &str,
    settings: &Arc<HttpSettings>,
    parameters: &ActionParameters,
    context: &ExecutionContext<'_>,
) -> ActionResult {
    let handler = handler(kind, settings);
    let normalized = handler.definition().normalize(parameters).unwrap();
    handler.execute(&normalized, context)
}

fn get(url: &str) -> ActionParameters {
    ActionParameters::new().with("url", ParameterValue::Text(url.into()))
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.into())
}

#[test]
fn the_contract_of_both_actions() {
    let settings = local_settings(&[]);
    let request = handler(REQUEST, &settings);
    let node = &request.definition().node;
    assert_eq!(node.effect, ActionEffect::Modifies);
    assert!(node.cancellable && !node.retry_safe);
    let sensitive: Vec<_> = node
        .outputs
        .iter()
        .filter(|o| o.sensitive)
        .map(|o| o.key.as_str())
        .collect();
    assert_eq!(sensitive, ["body", "value"]);
    let health = handler(HEALTH_CHECK, &settings);
    assert_eq!(health.definition().node.effect, ActionEffect::ReadOnly);
    assert!(health.definition().node.retry_safe);
}

#[test]
fn private_addresses_are_refused_unless_allowed() {
    let server = server();
    let fixture = fixture();
    let strict = Arc::new(crate::settings(&ActionParameters::new()));
    let refused = run_with(
        REQUEST,
        &strict,
        &get(&server.url("/json")),
        &fixture.context(),
    );
    assert_eq!(
        refused.code.as_deref(),
        Some(codes::BLOCKED_ADDRESS),
        "{}",
        refused.message
    );
    for host in ["localhost", "169.254.169.254", "[::1]", "10.0.0.1"] {
        let url = format!("http://{host}:{}/json", server.port);
        let result = run_with(REQUEST, &strict, &get(&url), &fixture.context());
        assert_eq!(
            result.code.as_deref(),
            Some(codes::BLOCKED_ADDRESS),
            "{host}: {}",
            result.message
        );
    }
    assert_eq!(
        server.seen(),
        Vec::<String>::new(),
        "nothing reached the server"
    );

    let allowed = run_with(
        REQUEST,
        &local_settings(&["127.0.0.1"]),
        &get(&server.url("/json")),
        &fixture.context(),
    );
    assert!(allowed.is_success(), "{}", allowed.message);
}

#[test]
fn json_responses_with_typed_outputs() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        REQUEST,
        &local_settings(&["127.0.0.1"]),
        &get(&server.url("/json"))
            .with("query", text("page=2\nq=a b&c"))
            .with("headers", text("Accept: application/json"))
            .with("extract", text("/data/0/name")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["status"], "200");
    assert_eq!(result.data["ok"], "true");
    assert_eq!(result.data["contentType"], "application/json");
    assert_eq!(result.data["value"], "Shop");
    assert!(result.data["body"].contains("\"id\":7"));
    // Cookies are never shown.
    assert!(
        result.data["headers"].contains("set-cookie: [redacted]"),
        "{}",
        result.data["headers"]
    );
    assert!(!result.data["headers"].contains("abc123"));
    let request = server.seen().pop().unwrap();
    assert!(
        request.starts_with("GET /json?page=2&q=a+b%26c "),
        "{request}"
    );
    assert!(request.to_lowercase().contains("accept: application/json"));
    // The query is not part of what is shown.
    assert_eq!(result.data["url"], server.url("/json"));
    assert!(!result.message.contains("page=2"));
}

#[test]
fn bodies_methods_and_status_codes() {
    let server = server();
    let fixture = fixture();
    let settings = local_settings(&["127.0.0.1"]);
    let posted = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/json"))
            .with("method", ParameterValue::Choice("POST".into()))
            .with("body", text(r#"{"name":"x"}"#)),
        &fixture.context(),
    );
    assert!(posted.is_success(), "{}", posted.message);
    let request = server.seen().pop().unwrap();
    assert!(request.starts_with("POST /json"));
    assert!(
        request
            .to_lowercase()
            .contains("content-type: application/json")
    );
    assert!(request.ends_with(r#"{"name":"x"}"#));

    let invalid = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/json")).with("body", text("{nope")),
        &fixture.context(),
    );
    assert_eq!(invalid.code.as_deref(), Some(codes::INVALID_BODY));
    for method in ["PUT", "PATCH", "DELETE"] {
        let result = run_with(
            REQUEST,
            &settings,
            &get(&server.url("/json")).with("method", ParameterValue::Choice(method.into())),
            &fixture.context(),
        );
        assert!(result.is_success(), "{method}");
        assert!(server.seen().pop().unwrap().starts_with(method));
    }

    let failed = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/fail")),
        &fixture.context(),
    );
    assert_eq!(failed.code.as_deref(), Some(codes::HTTP_STATUS));
    let tolerated = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/fail")).with("failOnHttpError", ParameterValue::Boolean(false)),
        &fixture.context(),
    );
    assert!(tolerated.is_success());
    assert_eq!(tolerated.data["ok"], "false");
}

#[test]
fn credentials_are_sent_but_never_shown() {
    let server = server();
    let fixture = fixture();
    let settings = local_settings(&["127.0.0.1"]);
    let bearer = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/json")).with("credential", text("api.token")),
        &fixture.context(),
    );
    assert!(bearer.is_success(), "{}", bearer.message);
    assert!(
        server
            .seen()
            .pop()
            .unwrap()
            .contains("authorization: Bearer s3cret-token")
    );
    let header = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/json"))
            .with("credential", text("api.token"))
            .with("authScheme", ParameterValue::Choice("header".into())),
        &fixture.context(),
    );
    assert!(header.is_success());
    assert!(
        server
            .seen()
            .pop()
            .unwrap()
            .to_lowercase()
            .contains("x-api-key: s3cret-token")
    );
    let basic = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/json"))
            .with("credential", text("api.basic"))
            .with("authScheme", ParameterValue::Choice("basic".into())),
        &fixture.context(),
    );
    assert!(basic.is_success());
    // base64("bob:pa55")
    assert!(server.seen().pop().unwrap().contains("Basic Ym9iOnBhNTU="));
    let query = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/json"))
            .with("credential", text("api.token"))
            .with("authScheme", ParameterValue::Choice("query".into())),
        &fixture.context(),
    );
    assert!(
        server
            .seen()
            .pop()
            .unwrap()
            .contains("api_key=s3cret-token")
    );
    for result in [&bearer, &header, &basic, &query] {
        let shown = serde_json::to_string(result).unwrap();
        assert!(
            !shown.contains("s3cret-token") && !shown.contains("pa55"),
            "{shown}"
        );
    }

    let missing = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/json")).with("credential", text("unknown")),
        &fixture.context(),
    );
    assert_eq!(missing.code.as_deref(), Some("credential-missing"));

    // Secrets cannot be written into the workflow itself.
    let handler = handler(REQUEST, &settings);
    let written = get(&server.url("/json")).with("headers", text("Authorization: Bearer abc"));
    assert_eq!(handler.check(&written, &fixture.profile).len(), 1);
    let result = run_with(REQUEST, &settings, &written, &fixture.context());
    assert_eq!(result.code.as_deref(), Some(codes::SENSITIVE_HEADER));
    let in_url = get(&format!("http://bob:pw@127.0.0.1:{}/json", server.port));
    assert_eq!(
        run_with(REQUEST, &settings, &in_url, &fixture.context())
            .code
            .as_deref(),
        Some(codes::CREDENTIALS_IN_URL)
    );
}

#[test]
fn redirects_are_checked_at_every_hop() {
    let server = server();
    let fixture = fixture();
    let settings = local_settings(&["127.0.0.1"]);
    let same = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/same")).with("credential", text("api.token")),
        &fixture.context(),
    );
    assert!(same.is_success(), "{}", same.message);
    assert_eq!(same.data["redirects"], "1");
    assert!(
        server.seen().pop().unwrap().contains("Bearer s3cret-token"),
        "same origin keeps it"
    );

    // Another host: not on the allowed list, so it is refused.
    let other = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/other")),
        &fixture.context(),
    );
    assert_eq!(
        other.code.as_deref(),
        Some(codes::BLOCKED_ADDRESS),
        "{}",
        other.message
    );
    // Allowed, but the credential stays with its origin.
    let both = local_settings(&["127.0.0.1", "localhost"]);
    let other = run_with(
        REQUEST,
        &both,
        &get(&server.url("/other")).with("credential", text("api.token")),
        &fixture.context(),
    );
    assert!(other.is_success(), "{}", other.message);
    let followed = server.seen().pop().unwrap();
    assert!(
        followed.to_lowercase().contains("host: localhost"),
        "{followed}"
    );
    assert!(!followed.contains("s3cret-token"), "{followed}");

    let looping = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/loop")),
        &fixture.context(),
    );
    assert_eq!(looping.code.as_deref(), Some(codes::TOO_MANY_REDIRECTS));
    let unfollowed = run_with(
        REQUEST,
        &settings,
        &get(&server.url("/same")).with("followRedirects", ParameterValue::Boolean(false)),
        &fixture.context(),
    );
    assert_eq!(unfollowed.data["status"], "302");
}

#[test]
fn responses_are_bounded_and_requests_timed() {
    let server = server();
    let fixture = fixture();
    let small = Arc::new(crate::settings(
        &ActionParameters::new()
            .with(crate::PRIVATE_HOSTS, text("127.0.0.1"))
            .with(crate::MAX_RESPONSE_KB, ParameterValue::Integer(1)),
    ));
    let big = run_with(
        REQUEST,
        &small,
        &get(&server.url("/big")),
        &fixture.context(),
    );
    assert!(big.is_success(), "{}", big.message);
    assert_eq!(big.data["size"], "1024");
    assert_eq!(big.data["truncated"], "true");

    let begun = Instant::now();
    let slow = run_with(
        REQUEST,
        &small,
        &get(&server.url("/slow")).with("timeoutSeconds", ParameterValue::Integer(1)),
        &fixture.context(),
    );
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
    let stopped = run_with(REQUEST, &small, &get(&server.url("/slow")), &context);
    canceller.join().unwrap();
    assert_eq!(
        stopped.code.as_deref(),
        Some("cancelled"),
        "{}",
        stopped.message
    );
    assert!(begun.elapsed() < Duration::from_secs(3));
}

#[test]
fn health_checks() {
    let server = server();
    let fixture = fixture();
    let settings = local_settings(&["127.0.0.1"]);
    let healthy = run_with(
        HEALTH_CHECK,
        &settings,
        &get(&server.url("/health")).with("contains", text("ok")),
        &fixture.context(),
    );
    assert!(healthy.is_success(), "{}", healthy.message);
    assert_eq!(healthy.data["healthy"], "true");
    assert_eq!(healthy.data["status"], "200");

    let wrong = run_with(
        HEALTH_CHECK,
        &settings,
        &get(&server.url("/fail")),
        &fixture.context(),
    );
    assert_eq!(wrong.code.as_deref(), Some(codes::UNHEALTHY));
    let reported = run_with(
        HEALTH_CHECK,
        &settings,
        &get(&server.url("/health"))
            .with("contains", text("degraded"))
            .with("failWhenUnhealthy", ParameterValue::Boolean(false)),
        &fixture.context(),
    );
    assert!(reported.is_success());
    assert_eq!(reported.data["healthy"], "false");
    assert!(reported.data["reason"].contains("degraded"));

    // Nothing listens on port 9 of this computer.
    let down = run_with(
        HEALTH_CHECK,
        &settings,
        &get("http://127.0.0.1:9/").with("failWhenUnhealthy", ParameterValue::Boolean(false)),
        &fixture.context(),
    );
    assert_eq!(down.data["healthy"], "false", "{}", down.message);

    assert_eq!(status_matches("2xx", 204), Some(true));
    assert_eq!(status_matches("200,204", 204), Some(true));
    assert_eq!(status_matches("300-399", 301), Some(true));
    assert_eq!(status_matches("200", 500), Some(false));
    assert_eq!(status_matches("ok", 200), None);
}

#[test]
fn the_network_capability_is_required() {
    let server = server();
    let fixture = fixture();
    let granted = std::collections::BTreeSet::from([Capability::Credentials]);
    let context = fixture.context().restricted(&granted);
    let denied = run_with(
        REQUEST,
        &local_settings(&["127.0.0.1"]),
        &get(&server.url("/json")),
        &context,
    );
    assert_eq!(denied.code.as_deref(), Some("not-permitted"));
    let none = std::collections::BTreeSet::from([Capability::Network]);
    let context = fixture.context().restricted(&none);
    let no_credentials = run_with(
        REQUEST,
        &local_settings(&["127.0.0.1"]),
        &get(&server.url("/json")).with("credential", text("api.token")),
        &context,
    );
    assert_eq!(no_credentials.code.as_deref(), Some("not-permitted"));
}
