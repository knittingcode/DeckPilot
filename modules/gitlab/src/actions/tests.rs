//! Requests go to a small GitLab stand-in on 127.0.0.1 started by the
//! test; no other host is contacted and no real account is used.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use deckpilot_core::FeatureModule;
use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{
    CancellationToken, ExecutionControl, MachineConfig, MemoryCredentials, Os, Progress, Secret,
};

use super::*;
use crate::api::MAX_PAGES;

const TOKEN: &str = "glpat-s3cretTOKENvalue";

/// Request heads the server received.
type Seen = Arc<Mutex<Vec<String>>>;

struct Server {
    port: u16,
    seen: Seen,
}

type Answer = (String, Vec<String>, Vec<u8>);

fn json(status: &str, body: &str) -> Answer {
    (
        status.to_owned(),
        vec!["Content-Type: application/json".into()],
        body.as_bytes().to_vec(),
    )
}

fn with_headers(answer: Answer, headers: &[String]) -> Answer {
    let (status, mut all, body) = answer;
    all.extend(headers.iter().cloned());
    (status, all, body)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn param(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|p| p.strip_prefix(&format!("{name}=")))
        .map(str::to_owned)
}

fn merge_request(n: usize) -> String {
    format!(
        r#"{{"iid":{n},"title":"Fix\nthing {n}","web_url":"https://gitlab.com/group/site/-/merge_requests/{n}","references":{{"full":"group/site!{n}"}},"author":{{"username":"dev{n}"}},"draft":{}}}"#,
        n == 2
    )
}

fn merge_requests(count: usize) -> String {
    let items: Vec<String> = (1..=count).map(merge_request).collect();
    format!("[{}]", items.join(","))
}

#[allow(clippy::too_many_lines)] // One table of the stand-in's answers.
fn route(path: &str, query: &str, token: &str, port: u16) -> Answer {
    if path == "/api/v4/projects/group%2Fleak" {
        // A server that repeats what it was sent.
        return json(
            "403 Forbidden",
            &format!(r#"{{"message":"refused {token}"}}"#),
        );
    }
    if token != TOKEN {
        return json("401 Unauthorized", r#"{"message":"401 Unauthorized"}"#);
    }
    let page: u32 = param(query, "page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let per_page: usize = param(query, "per_page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let Some(path) = path.strip_prefix("/api/v4") else {
        return json("404 Not Found", "{}");
    };
    match path {
        "/user" => with_headers(
            json(
                "200 OK",
                r#"{"id":7,"username":"octo","name":"Octo Cat","web_url":"https://gitlab.com/octo"}"#,
            ),
            &["RateLimit-Remaining: 1999".into()],
        ),
        "/projects/group%2Fsite" | "/projects/42" => json(
            "200 OK",
            r#"{"id":42,"path_with_namespace":"group/site","description":"Our web\u202esite","default_branch":"main","visibility":"private","archived":false,"open_issues_count":12,"star_count":5,"forks_count":2,"web_url":"https://gitlab.com/group/site","last_activity_at":"2026-11-01T10:00:00Z"}"#,
        ),
        "/projects/group%2Fforbidden" => json(
            "403 Forbidden",
            r#"{"error":"insufficient_scope","error_description":"The request requires higher privileges"}"#,
        ),
        "/projects/group%2Flimited" => with_headers(
            json("429 Too Many Requests", r#"{"message":"Retry later"}"#),
            &[format!("RateLimit-Reset: {}", now() + 120)],
        ),
        "/projects/group%2Fsecondary" => with_headers(
            json("429 Too Many Requests", "{}"),
            &["Retry-After: 30".into()],
        ),
        "/projects/group%2Fbroken" => json("502 Bad Gateway", r#"{"message":"Bad Gateway"}"#),
        "/projects/group%2Fslow" => {
            std::thread::sleep(Duration::from_secs(5));
            json("200 OK", "{}")
        }
        "/projects/group%2Fbig" => (
            "200 OK".into(),
            vec!["Content-Type: application/json".into()],
            vec![b' '; crate::api::MAX_RESPONSE_BYTES + 10],
        ),
        "/projects/group%2Fsite/merge_requests" => with_headers(
            json("200 OK", &merge_requests(per_page.min(3))),
            &["X-Total: 42".into()],
        ),
        // Across projects: no X-Total, as for very large lists.
        "/merge_requests" => {
            if per_page == 100 {
                let count = if page == 1 { 100 } else { 7 };
                let mut headers = Vec::new();
                if page == 1 {
                    headers.push(format!(
                        "Link: <http://127.0.0.1:{port}/api/v4/merge_requests?{query}&page=2>; rel=\"next\""
                    ));
                }
                with_headers(json("200 OK", &merge_requests(count)), &headers)
            } else {
                with_headers(
                    json("200 OK", &merge_requests(per_page.min(3))),
                    &[format!(
                        "Link: <http://127.0.0.1:{port}/api/v4/merge_requests?page=2>; rel=\"next\""
                    )],
                )
            }
        }
        "/projects/group%2Fsite/merge_requests/5" => json(
            "200 OK",
            r#"{"iid":5,"web_url":"https://gitlab.com/group/site/-/merge_requests/5","head_pipeline":{"id":99,"status":"failed","sha":"abc123","web_url":"https://gitlab.com/group/site/-/pipelines/99"}}"#,
        ),
        "/projects/group%2Fsite/merge_requests/6" => json(
            "200 OK",
            r#"{"iid":6,"web_url":"https://gitlab.com/group/site/-/merge_requests/6","head_pipeline":null}"#,
        ),
        "/projects/group%2Fsite/pipelines" => json(
            "200 OK",
            r#"[{"id":99,"status":"running","sha":"abc123","web_url":"https://gitlab.com/group/site/-/pipelines/99"}]"#,
        ),
        "/projects/group%2Fsite/pipelines/99/jobs" => with_headers(
            json(
                "200 OK",
                r#"[{"name":"build","status":"success"},
                    {"name":"lint","status":"failed","allow_failure":false},
                    {"name":"flaky","status":"failed","allow_failure":true},
                    {"name":"e2e","status":"running"},
                    {"name":"deploy","status":"manual"}]"#,
            ),
            &["X-Total: 5".into()],
        ),
        "/projects/group%2Fendless/pipelines" => json("200 OK", r#"[{"id":1,"status":"success"}]"#),
        "/projects/group%2Fendless/pipelines/1/jobs" => with_headers(
            json("200 OK", r#"[{"name":"x","status":"success"}]"#),
            &[format!(
                "Link: <http://127.0.0.1:{port}/api/v4/projects/group%2Fendless/pipelines/1/jobs?page={}>; rel=\"next\"",
                page + 1
            )],
        ),
        "/projects/group%2Fescape/pipelines" => json("200 OK", r#"[{"id":2,"status":"success"}]"#),
        "/projects/group%2Fescape/pipelines/2/jobs" => with_headers(
            json("200 OK", "[]"),
            &["Link: <https://collector.example/steal>; rel=\"next\"".into()],
        ),
        p if p.ends_with("/pipelines") => json("200 OK", "[]"),
        _ => json("404 Not Found", r#"{"message":"404 Project Not Found"}"#),
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
    let token = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("private-token")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default();
    let (status, headers, body) = route(path, query, &token, port);
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
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    fn last(&self) -> String {
        self.seen().pop().expect("a request reached the server")
    }

    fn api(&self) -> String {
        format!("http://127.0.0.1:{}/api/v4", self.port)
    }

    fn settings(&self) -> Arc<GitlabSettings> {
        settings_for(&self.api(), true, 20)
    }
}

fn settings_for(api_url: &str, allow_private: bool, timeout: i64) -> Arc<GitlabSettings> {
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
                CredentialId::new("gitlab.token").unwrap(),
                Secret::new(TOKEN),
            )
            .with(
                CredentialId::new("gitlab.old").unwrap(),
                Secret::new("glpat-revokedTOKEN"),
            ),
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
    settings: &Arc<GitlabSettings>,
    launcher: &Arc<dyn Launcher>,
) -> Box<dyn ActionHandler> {
    handlers(settings, launcher)
        .into_iter()
        .find(|h| h.definition().id.as_str() == kind)
        .unwrap()
}

fn run_with(
    kind: &str,
    settings: &Arc<GitlabSettings>,
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

fn project(name: &str) -> ActionParameters {
    ActionParameters::new().with("project", text(name))
}

/// The token is nowhere in a result.
fn assert_no_token(result: &ActionResult) {
    let shown = serde_json::to_string(result).unwrap();
    assert!(
        !shown.contains(TOKEN) && !shown.contains("glpat-revoked"),
        "{shown}"
    );
}

#[test]
fn the_module_and_its_contract() {
    let module = crate::GitlabModule::new();
    let manifest = module.manifest();
    assert_eq!(manifest.id.as_str(), "gitlab");
    assert_eq!(
        manifest.capabilities,
        [
            Capability::Network,
            Capability::Credentials,
            Capability::OpenExternal
        ]
    );
    let defaults = crate::settings(&ActionParameters::new());
    assert_eq!(defaults.api_url, "https://gitlab.com/api/v4");
    assert_eq!(defaults.credential, "gitlab.token");
    assert!(!defaults.allow_private);
    for key in [crate::API_URL, crate::ALLOW_PRIVATE] {
        assert_eq!(
            manifest.portability.settings[key],
            deckpilot_core::SettingScope::MachineSpecific
        );
    }

    let settings = settings_for(crate::DEFAULT_API_URL, false, 20);
    let all = handlers(&settings, &recording());
    let ids: Vec<_> = all.iter().map(|h| h.definition().id.to_string()).collect();
    assert_eq!(
        ids,
        [
            CURRENT_USER,
            PROJECT,
            MERGE_REQUESTS,
            REVIEW_REQUESTS,
            PIPELINE,
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
    assert_eq!(sensitive(PROJECT), ["description"]);
    assert_eq!(sensitive(MERGE_REQUESTS), ["items"]);
    assert_eq!(sensitive(REVIEW_REQUESTS), ["items"]);
}

#[test]
fn the_current_user_with_the_token_header() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        CURRENT_USER,
        &server.settings(),
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["username"], "octo");
    assert_eq!(result.data["name"], "Octo Cat");
    assert_eq!(result.data["url"], "https://gitlab.com/octo");
    assert_eq!(result.data["rateLimitRemaining"], "1999");
    let request = server.last();
    assert!(request.starts_with("GET /api/v4/user "), "{request}");
    assert!(
        request
            .to_lowercase()
            .contains(&format!("private-token: {}", TOKEN.to_lowercase())),
        "{request}"
    );
    assert!(!request.to_lowercase().contains("authorization"));
    assert_no_token(&result);
}

#[test]
fn private_api_hosts_need_permission() {
    let server = server();
    let fixture = fixture();
    let strict = settings_for(&server.api(), false, 20);
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
    assert!(refused.message.contains("Settings › Modules › GitLab"));
    assert_eq!(server.seen().len(), 0, "nothing reached the server");

    let insecure = settings_for("http://gitlab.example.com/api/v4", true, 20);
    let result = run_with(
        CURRENT_USER,
        &insecure,
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert_eq!(result.code.as_deref(), Some(codes::INVALID_API_URL));
}

#[test]
fn project_information() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        PROJECT,
        &server.settings(),
        &project("group/site"),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["path"], "group/site");
    assert_eq!(result.data["defaultBranch"], "main");
    assert_eq!(result.data["visibility"], "private");
    assert_eq!(result.data["archived"], "false");
    assert_eq!(result.data["openIssues"], "12");
    assert_eq!(result.data["stars"], "5");
    assert_eq!(result.data["forks"], "2");
    assert_eq!(result.data["lastActivityAt"], "2026-11-01T10:00:00Z");
    assert_eq!(result.data["description"], "Our web site");
    // The path is one encoded segment.
    assert!(
        server
            .last()
            .starts_with("GET /api/v4/projects/group%2Fsite ")
    );

    let by_id = run_with(
        PROJECT,
        &server.settings(),
        &project("42"),
        &fixture.context(),
    );
    assert!(by_id.is_success(), "{}", by_id.message);

    let invalid = run_with(
        PROJECT,
        &server.settings(),
        &project("group/../../user"),
        &fixture.context(),
    );
    assert_eq!(invalid.code.as_deref(), Some(codes::INVALID_PROJECT));
}

#[test]
fn gitlab_failures_have_codes() {
    let server = server();
    let fixture = fixture();
    let settings = server.settings();
    let run = |name: &str| run_with(PROJECT, &settings, &project(name), &fixture.context());

    let missing = run("group/private");
    assert_eq!(missing.code.as_deref(), Some(codes::NOT_FOUND));
    assert!(missing.message.contains("group/private"));

    let forbidden = run("group/forbidden");
    assert_eq!(forbidden.code.as_deref(), Some(codes::FORBIDDEN));
    assert!(
        forbidden.message.contains("higher privileges"),
        "{}",
        forbidden.message
    );

    let limited = run("group/limited");
    assert_eq!(limited.code.as_deref(), Some(codes::RATE_LIMITED));
    let wait: u64 = limited.data["retryAfterSeconds"].parse().unwrap();
    assert!((100..=120).contains(&wait), "{wait}");

    let secondary = run("group/secondary");
    assert_eq!(secondary.code.as_deref(), Some(codes::RATE_LIMITED));
    assert_eq!(secondary.data["retryAfterSeconds"], "30");

    let broken = run("group/broken");
    assert_eq!(broken.code.as_deref(), Some(codes::GITLAB_ERROR));
    assert!(broken.message.contains("502"));

    let big = run("group/big");
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
        &ActionParameters::new().with("credential", text("gitlab.old")),
        &fixture.context(),
    );
    assert_eq!(revoked.code.as_deref(), Some(codes::UNAUTHORIZED));
    assert!(revoked.message.contains("gitlab.old"));
    assert_no_token(&revoked);

    let missing = run_with(
        CURRENT_USER,
        &settings,
        &ActionParameters::new().with("credential", text("gitlab.other")),
        &fixture.context(),
    );
    assert_eq!(missing.code.as_deref(), Some("credential-missing"));
    let before = server.seen().len();

    let empty = MemoryCredentials::new();
    let context =
        ExecutionContext::new(&fixture.profile, None, &fixture.machine).with_credentials(&empty);
    let none = run_with(CURRENT_USER, &settings, &ActionParameters::new(), &context);
    assert_eq!(none.code.as_deref(), Some("credential-missing"));
    assert!(none.message.contains("gitlab.token"));
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
        PROJECT,
        &server.settings(),
        &project("group/leak"),
        &fixture.context(),
    );
    assert_eq!(result.code.as_deref(), Some(codes::FORBIDDEN));
    assert!(result.message.contains("[redacted]"), "{}", result.message);
    assert_no_token(&result);
}

#[test]
fn open_merge_requests_with_filters() {
    let server = server();
    let fixture = fixture();
    let settings = server.settings();
    let all = run_with(
        MERGE_REQUESTS,
        &settings,
        &project("group/site").with("limit", ParameterValue::Integer(2)),
        &fixture.context(),
    );
    assert!(all.is_success(), "{}", all.message);
    assert_eq!(all.data["count"], "42");
    assert_eq!(all.data["countExact"], "true");
    assert_eq!(all.data["listed"], "2");
    assert_eq!(all.data["truncated"], "true");
    assert_eq!(
        all.data["items"],
        "group/site!1 Fix thing 1 (@dev1)\ngroup/site!2 Fix thing 2 (@dev2) [draft]"
    );
    assert_eq!(
        all.data["firstUrl"],
        "https://gitlab.com/group/site/-/merge_requests/1"
    );
    assert!(!all.message.contains("Fix"));
    let request = server.last();
    assert!(
        request.starts_with(
            "GET /api/v4/projects/group%2Fsite/merge_requests?state=opened&order_by=updated_at&sort=desc&per_page=2 "
        ),
        "{request}"
    );

    let mine = run_with(
        MERGE_REQUESTS,
        &settings,
        &project("group/site").with("filter", ParameterValue::Choice("author-me".into())),
        &fixture.context(),
    );
    assert!(mine.is_success(), "{}", mine.message);
    let seen = server.seen();
    assert!(seen[seen.len() - 2].starts_with("GET /api/v4/user "));
    assert!(server.last().contains("&author_id=7&"));

    let everything = run_with(
        MERGE_REQUESTS,
        &settings,
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert_eq!(everything.code.as_deref(), Some(codes::INVALID_PROJECT));
}

#[test]
fn review_requests_are_counted_without_a_total() {
    let server = server();
    let fixture = fixture();
    let result = run_with(
        REVIEW_REQUESTS,
        &server.settings(),
        &ActionParameters::new(),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    // No X-Total: counted from two pages of 100 and 7.
    assert_eq!(result.data["count"], "107");
    assert_eq!(result.data["countExact"], "true");
    assert_eq!(result.data["listed"], "3");
    assert!(result.message.contains("wait for your review"));
    let seen = server.seen();
    let listing = seen
        .iter()
        .find(|r| r.starts_with("GET /api/v4/merge_requests?"))
        .unwrap();
    assert!(
        listing.contains("reviewer_id=7&scope=all&per_page=20"),
        "{listing}"
    );
}

#[test]
fn pipelines_of_a_ref_and_of_a_merge_request() {
    let server = server();
    let fixture = fixture();
    let settings = server.settings();
    let by_ref = run_with(
        PIPELINE,
        &settings,
        &project("group/site").with("ref", text("feature/login")),
        &fixture.context(),
    );
    assert!(by_ref.is_success(), "{}", by_ref.message);
    assert_eq!(by_ref.data["state"], "pending");
    assert_eq!(by_ref.data["status"], "running");
    assert_eq!(by_ref.data["pipelineId"], "99");
    assert_eq!(by_ref.data["sha"], "abc123");
    assert_eq!(by_ref.data["total"], "5");
    assert_eq!(by_ref.data["passed"], "2");
    assert_eq!(by_ref.data["failed"], "1");
    assert_eq!(by_ref.data["pending"], "1");
    assert_eq!(by_ref.data["skipped"], "1");
    assert_eq!(by_ref.data["failing"], "lint");
    assert_eq!(by_ref.data["truncated"], "false");
    assert_eq!(
        by_ref.data["url"],
        "https://gitlab.com/group/site/-/pipelines/99"
    );
    assert!(server.seen().iter().any(|r| {
        r.starts_with("GET /api/v4/projects/group%2Fsite/pipelines?ref=feature%2Flogin&")
    }));

    let by_mr = run_with(
        PIPELINE,
        &settings,
        &project("group/site").with("mergeRequest", ParameterValue::Integer(5)),
        &fixture.context(),
    );
    assert!(by_mr.is_success(), "{}", by_mr.message);
    assert_eq!(by_mr.data["state"], "failing");
    assert_eq!(
        by_mr.data["url"],
        "https://gitlab.com/group/site/-/merge_requests/5"
    );

    let without = run_with(
        PIPELINE,
        &settings,
        &project("group/site").with("mergeRequest", ParameterValue::Integer(6)),
        &fixture.context(),
    );
    assert_eq!(without.data["state"], "none", "{}", without.message);
    let empty = run_with(
        PIPELINE,
        &settings,
        &project("group/empty").with("ref", text("main")),
        &fixture.context(),
    );
    assert_eq!(empty.data["state"], "none", "{}", empty.message);
    assert_eq!(empty.data["total"], "0");

    let sha = "0123456789abcdef0123456789abcdef01234567";
    let _ = run_with(
        PIPELINE,
        &settings,
        &project("group/empty").with("ref", text(sha)),
        &fixture.context(),
    );
    assert!(server.last().contains(&format!("pipelines?sha={sha}&")));

    let none = run_with(
        PIPELINE,
        &settings,
        &project("group/site"),
        &fixture.context(),
    );
    assert_eq!(none.code.as_deref(), Some("invalid-parameters"));
    let bad_ref = run_with(
        PIPELINE,
        &settings,
        &project("group/site").with("ref", text("a..b")),
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
        PIPELINE,
        &settings,
        &project("group/endless").with("ref", text("main")),
        &fixture.context(),
    );
    assert!(endless.is_success(), "{}", endless.message);
    assert_eq!(endless.data["truncated"], "true");
    assert_eq!(endless.data["total"], MAX_PAGES.to_string());
    let pages = server
        .seen()
        .iter()
        .filter(|r| r.contains("/pipelines/1/jobs"))
        .count();
    assert_eq!(pages, MAX_PAGES);

    let escape = run_with(
        PIPELINE,
        &settings,
        &project("group/escape").with("ref", text("main")),
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
    let quick = settings_for(&server.api(), true, 1);
    let begun = Instant::now();
    let slow = run_with(PROJECT, &quick, &project("group/slow"), &fixture.context());
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
    let stopped = run_with(
        PROJECT,
        &server.settings(),
        &project("group/slow"),
        &context,
    );
    canceller.join().unwrap();
    assert_eq!(
        stopped.code.as_deref(),
        Some("cancelled"),
        "{}",
        stopped.message
    );
    assert!(begun.elapsed() < Duration::from_secs(3));

    let before = server.seen().len();
    let again = run_with(
        PROJECT,
        &server.settings(),
        &project("group/site"),
        &context,
    );
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
    let parameters = handler
        .definition()
        .normalize(&project("group/site"))
        .unwrap();
    let opened = handler.execute(&parameters, &fixture.context().restricted(&network_only));
    assert_eq!(opened.code.as_deref(), Some("not-permitted"));
    assert_eq!(launcher.plans().len(), 0);
}

#[test]
fn only_pages_on_the_gitlab_host_are_opened() {
    let fixture = fixture();
    let settings = settings_for("https://code.example.com/gitlab/api/v4", false, 20);
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = handler_with(OPEN, &settings, &(launcher.clone() as Arc<dyn Launcher>));
    let run = |parameters: &ActionParameters| {
        let parameters = handler.definition().normalize(parameters).unwrap();
        handler.execute(&parameters, &fixture.context())
    };

    let merge = run(&project("group/sub/site").with("mergeRequest", ParameterValue::Integer(7)));
    assert!(merge.is_success(), "{}", merge.message);
    assert_eq!(
        merge.data["url"],
        "https://code.example.com/gitlab/group/sub/site/-/merge_requests/7"
    );
    let page = run(&ActionParameters::new().with(
        "url",
        text("https://code.example.com/gitlab/group/site/-/pipelines/9"),
    ));
    assert!(page.is_success(), "{}", page.message);
    assert_eq!(launcher.plans().len(), 2);

    for refused in [
        "https://gitlab.com/group/site",
        "http://code.example.com/gitlab/group/site",
        "https://code.example.com/other/group/site",
        "https://code.example.com.evil.test/gitlab/x",
        "file:///etc/passwd",
    ] {
        let result = run(&ActionParameters::new().with("url", text(refused)));
        assert_eq!(
            result.code.as_deref(),
            Some(codes::URL_NOT_ALLOWED),
            "{refused}"
        );
    }
    let by_id = run(&project("42"));
    assert_eq!(by_id.code.as_deref(), Some(codes::INVALID_PROJECT));
    let nothing = run(&ActionParameters::new());
    assert_eq!(nothing.code.as_deref(), Some(codes::INVALID_PROJECT));
    assert_eq!(launcher.plans().len(), 2);
}

#[test]
fn problems_are_found_when_saving() {
    let fixture = fixture();
    let settings = settings_for(crate::DEFAULT_API_URL, false, 20);
    let launcher = recording();
    let pipeline = handler_with(PIPELINE, &settings, &launcher);
    let bad = project("not a project")
        .with("ref", text("a..b"))
        .with("credential", text("Bad Name"));
    assert_eq!(pipeline.check(&bad, &fixture.profile).len(), 3);
    let templated =
        project("{{ steps.a.output.project }}").with("ref", text("{{ steps.a.output.ref }}"));
    assert_eq!(
        pipeline.check(&templated, &fixture.profile),
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
}

#[test]
fn job_outcomes() {
    let job = |status: &str, allowed: bool| {
        job_outcome(&serde_json::json!({"status": status, "allow_failure": allowed}))
    };
    assert_eq!(job("success", false), Outcome::Passed);
    assert_eq!(job("failed", true), Outcome::Passed);
    assert_eq!(job("failed", false), Outcome::Failed);
    assert_eq!(job("canceled", false), Outcome::Failed);
    assert_eq!(job("manual", false), Outcome::Skipped);
    assert_eq!(job("skipped", false), Outcome::Skipped);
    for pending in [
        "created",
        "pending",
        "running",
        "waiting_for_resource",
        "scheduled",
    ] {
        assert_eq!(job(pending, false), Outcome::Pending, "{pending}");
    }
    assert_eq!(overall("success"), "ok");
    assert_eq!(overall("canceled"), "failing");
    assert_eq!(overall("manual"), "pending");
    assert_eq!(overall("skipped"), "none");
}
