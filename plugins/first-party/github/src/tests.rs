//! Native unit tests: the definition agrees with manifest.json, the
//! manifest is valid against the schema, and every action behaves, with
//! the agent replaced by `MockHost`. The comparison with the built-in
//! module on the same answers is `apps/agent/tests/plugins_github.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};
use deckpilot_plugin_sdk::{
    ActionOutcome, AuthScheme, CallErrorCode, CallResult, CapabilityRequest, HttpMethod,
    ParamValue, Params,
};
use serde_json::{Value, json};

use super::*;

const MANIFEST: &str = include_str!("../manifest.json");
const SCHEMA: &str = include_str!("../../../schemas/plugin-manifest.schema.json");
const DATE: &str = "Mon, 16 Nov 2026 10:00:00 GMT";

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).unwrap()
}

/// One canned answer.
#[derive(Debug, Clone)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

fn answer(status: u16, body: &Value) -> Answer {
    Answer {
        status,
        headers: vec![("date".into(), DATE.into())],
        body: body.to_string(),
    }
}

impl Answer {
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// A stand-in agent: settings, a stored token (or not), canned answers by
/// URL prefix (the longest matching prefix wins), opened URLs.
#[derive(Debug, Default)]
struct Agent {
    settings: Value,
    token: bool,
    answers: BTreeMap<String, Answer>,
    /// URLs that get no answer, with the agent's network kind.
    unreachable: BTreeMap<String, NetworkErrorKind>,
    fetched: Vec<String>,
    opened: Vec<String>,
}

type Shared = Rc<RefCell<Agent>>;

fn agent() -> Shared {
    Rc::new(RefCell::new(Agent {
        settings: json!({"apiUrl": "https://api.github.com", "timeoutSeconds": 20}),
        token: true,
        ..Agent::default()
    }))
}

fn host(agent: &Shared) -> MockHost {
    let agent = agent.clone();
    MockHost::new(move |request| {
        let mut agent = agent.borrow_mut();
        match request {
            CapabilityRequest::SettingsGet(_) => CallResult::Ok(agent.settings.clone()),
            CapabilityRequest::CredentialHas(r) => {
                assert_eq!(r.credential, api::TOKEN_SLOT);
                CallResult::Ok(json!(agent.token))
            }
            CapabilityRequest::HttpFetch(f) => {
                assert_eq!(f.method, HttpMethod::Get, "read-only");
                let auth = f.auth.as_ref().expect("every request names the token");
                assert_eq!(auth.credential, api::TOKEN_SLOT);
                assert_eq!(auth.scheme, AuthScheme::Bearer);
                for name in ["Accept", "X-GitHub-Api-Version", "User-Agent"] {
                    assert!(f.headers.iter().any(|h| h.name == name), "{name}");
                }
                agent.fetched.push(f.url.clone());
                if let Some(kind) = agent.unreachable.get(&f.url) {
                    return CallResult::error_with_kind(
                        CallErrorCode::Failed,
                        format!("{} (canned)", kind.code()),
                        Some(*kind),
                    );
                }
                let found = agent
                    .answers
                    .iter()
                    .filter(|(prefix, _)| f.url.starts_with(prefix.as_str()))
                    .max_by_key(|(prefix, _)| prefix.len())
                    .map(|(_, a)| a.clone());
                match found {
                    Some(a) => CallResult::Ok(json!({
                        "status": a.status,
                        "headers": a.headers.iter()
                            .map(|(n, v)| json!({"name": n, "value": v}))
                            .collect::<Vec<_>>(),
                        "body": a.body,
                    })),
                    None => CallResult::error(CallErrorCode::Failed, "no route"),
                }
            }
            CapabilityRequest::OpenUrl(o) => {
                agent.opened.push(o.url.clone());
                CallResult::Ok(Value::Null)
            }
            other => panic!("unexpected request {other:?}"),
        }
    })
}

fn route(agent: &Shared, url: &str, answer: Answer) {
    agent.borrow_mut().answers.insert(url.into(), answer);
}

fn run(agent: &Shared, action: &str, params: Value) -> ActionOutcome {
    host(agent).invoke(&plugin(), action, params)
}

fn ok(outcome: &ActionOutcome) -> (&str, &Params) {
    match outcome {
        ActionOutcome::Ok { message, data } => (message.as_deref().unwrap_or(""), data),
        other => panic!("expected ok, got {other:?}"),
    }
}

fn failed(outcome: &ActionOutcome) -> (&str, &str) {
    match outcome {
        ActionOutcome::Failed { code, message, .. } => (code, message.as_deref().unwrap_or("")),
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn s(value: &str) -> ParamValue {
    ParamValue::String(value.into())
}

#[test]
fn definition_is_valid_and_matches_the_manifest() {
    plugin().validate().unwrap();
    let manifest = manifest();
    assert_eq!(
        manifest_mismatches(&plugin(), &manifest),
        Vec::<String>::new()
    );
    assert_eq!(manifest["id"], ID);
    assert_eq!(manifest["version"], VERSION);
    // A bundled plugin is manifest.json, plugin.wasm, LICENSE and its
    // icons: no other assets.
    assert!(manifest.get("assets").is_none());
}

#[test]
fn manifest_is_valid_against_the_schema() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let manifest = manifest();
    let errors: Vec<String> = validator
        .iter_errors(&manifest)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn codes_slots_settings_and_permissions_are_declared() {
    let manifest = manifest();
    let declared = manifest["contributes"]["errorCodes"].as_object().unwrap();
    let mut codes_used: Vec<&str> = api::codes::ALL.to_vec();
    codes_used.sort_unstable();
    let mut in_manifest: Vec<&str> = declared.keys().map(String::as_str).collect();
    in_manifest.sort_unstable();
    assert_eq!(codes_used, in_manifest);
    assert_eq!(
        manifest["contributes"]["credentials"][0]["key"],
        api::TOKEN_SLOT
    );
    let settings: Vec<&str> = manifest["contributes"]["settings"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["key"].as_str().unwrap())
        .collect();
    assert_eq!(settings, [api::API_URL, api::TIMEOUT]);
    // GET to the API, the token to the API, pages on github.com: nothing
    // else.
    assert_eq!(
        manifest["permissions"],
        json!([
            {"kind": "network", "hosts": ["api.github.com"], "methods": ["GET"],
             "justification": manifest["permissions"][0]["justification"]},
            {"kind": "credentials", "refs": ["token"],
             "justification": manifest["permissions"][1]["justification"]},
            {"kind": "openUrl", "hosts": ["github.com"],
             "justification": manifest["permissions"][2]["justification"]},
        ])
    );
}

#[test]
fn the_current_user() {
    let agent = agent();
    route(
        &agent,
        "https://api.github.com/user",
        answer(
            200,
            &json!({"login": "octocat", "name": "The\u{202e} Octocat", "html_url": "https://github.com/octocat"}),
        )
        .header("x-ratelimit-remaining", "4999"),
    );
    let out = run(&agent, "current-user", json!({}));
    let (message, data) = ok(&out);
    assert_eq!(message, "signed in to GitHub as octocat");
    assert_eq!(data["login"], s("octocat"));
    assert_eq!(data["name"], s("The  Octocat"));
    assert_eq!(data["url"], s("https://github.com/octocat"));
    assert_eq!(data["rateLimitRemaining"], ParamValue::Integer(4999));
}

#[test]
fn a_missing_token_or_another_api_is_refused_before_any_request() {
    let agent = agent();
    agent.borrow_mut().token = false;
    let out = run(&agent, "repository", json!({"repository": "a/b"}));
    assert_eq!(failed(&out).0, "credential-missing");
    let agent = self::agent();
    agent.borrow_mut().settings = json!({"apiUrl": "https://ghe.example.com/api/v3"});
    let out = run(&agent, "current-user", json!({}));
    let (code, message) = failed(&out);
    assert_eq!(code, "invalid-api-url");
    assert!(message.contains("built-in GitHub module"), "{message}");
    assert!(agent.borrow().fetched.is_empty());
    // Open checks the setting too, and opens nothing.
    let out = run(&agent, "open", json!({"repository": "a/b"}));
    assert_eq!(failed(&out).0, "invalid-api-url");
    assert!(agent.borrow().opened.is_empty());
}

#[test]
fn repository_information() {
    let agent = agent();
    route(
        &agent,
        "https://api.github.com/repos/octo/site",
        answer(
            200,
            &json!({
                "full_name": "octo/site", "default_branch": "main", "private": true,
                "archived": false, "open_issues_count": 7, "stargazers_count": 42,
                "forks_count": 3, "html_url": "https://github.com/octo/site",
                "description": "", "pushed_at": "2026-11-01T10:00:00Z"
            }),
        ),
    );
    let out = run(
        &agent,
        "repository",
        json!({"repository": " octo/site.git "}),
    );
    let (message, data) = ok(&out);
    assert_eq!(message, "read octo/site");
    assert_eq!(data["private"], ParamValue::Bool(true));
    assert_eq!(data["openIssues"], ParamValue::Integer(7));
    assert!(!data.contains_key("description"));
    assert_eq!(data["pushedAt"], s("2026-11-01T10:00:00Z"));
    let out = run(&agent, "repository", json!({"repository": "not a repo"}));
    assert_eq!(failed(&out).0, "invalid-repository");
}

fn search_items(from: usize, count: usize) -> Vec<Value> {
    (from..from + count)
        .map(|n| {
            json!({
                "number": n, "title": format!("Change {n}"), "draft": n % 2 == 0,
                "repository_url": "https://api.github.com/repos/octo/site",
                "user": {"login": "dev"},
                "html_url": format!("https://github.com/octo/site/pull/{n}")
            })
        })
        .collect()
}

#[test]
fn pull_requests_up_to_fifty_are_one_request_and_more_follow_the_next_link() {
    let agent = agent();
    let first = "https://api.github.com/search/issues?q=is%3Apr+is%3Aopen+repo%3Aocto%2Fsite\
                 &sort=updated&order=desc&per_page=";
    route(
        &agent,
        &format!("{first}20"),
        answer(
            200,
            &json!({"total_count": 120, "items": search_items(1, 20)}),
        ),
    );
    let out = run(&agent, "pull-requests", json!({"repository": "octo/site"}));
    let (message, data) = ok(&out);
    assert_eq!(message, "120 open pull requests in octo/site (all)");
    assert_eq!(data["count"], ParamValue::Integer(120));
    assert_eq!(data["listed"], ParamValue::Integer(20));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
    assert_eq!(data["firstUrl"], s("https://github.com/octo/site/pull/1"));
    let ParamValue::String(items) = &data["items"] else {
        panic!()
    };
    assert_eq!(
        items.lines().take(2).collect::<Vec<_>>(),
        [
            "octo/site#1 Change 1 (@dev)",
            "octo/site#2 Change 2 (@dev) [draft]"
        ]
    );

    // 70: a page of 50, then GitHub's next page, of which 20 are listed.
    let next = "https://api.github.com/search/issues?page=2";
    route(
        &agent,
        &format!("{first}50"),
        answer(
            200,
            &json!({"total_count": 120, "items": search_items(1, 50)}),
        )
        .header("link", &format!("<{next}>; rel=\"next\"")),
    );
    route(
        &agent,
        next,
        answer(
            200,
            &json!({"total_count": 120, "items": search_items(51, 50)}),
        )
        .header(
            "link",
            "<https://api.github.com/search/issues?page=3>; rel=\"next\"",
        ),
    );
    agent.borrow_mut().fetched.clear();
    let out = run(
        &agent,
        "pull-requests",
        json!({"repository": "octo/site", "limit": 70}),
    );
    let (_, data) = ok(&out);
    assert_eq!(data["listed"], ParamValue::Integer(70));
    assert_eq!(agent.borrow().fetched.len(), 2, "never a third page");

    // Every repository needs a filter other than all.
    let out = run(&agent, "pull-requests", json!({}));
    assert_eq!(failed(&out).0, "invalid-repository");
}

#[test]
fn review_requests_search_across_repositories() {
    let agent = agent();
    route(
        &agent,
        "https://api.github.com/search/issues?q=is%3Apr+is%3Aopen+archived%3Afalse+review-requested%3A%40me",
        answer(200, &json!({"total_count": 0, "items": []})),
    );
    let out = run(&agent, "review-requests", json!({}));
    let (message, data) = ok(&out);
    assert_eq!(message, "0 pull requests wait for your review");
    assert_eq!(data["items"], s(""));
    assert!(!data.contains_key("firstUrl"));
}

#[test]
fn a_next_page_outside_the_api_is_refused() {
    let agent = agent();
    route(
        &agent,
        "https://api.github.com/repos/octo/site/commits/main/check-runs",
        answer(200, &json!({"total_count": 1, "check_runs": []}))
            .header("link", "<https://evil.example.com/x>; rel=\"next\""),
    );
    let out = run(
        &agent,
        "checks",
        json!({"repository": "octo/site", "ref": "main"}),
    );
    assert_eq!(failed(&out).0, "invalid-response");
}

#[test]
fn checks_of_a_pull_request_count_and_name_the_failing() {
    let agent = agent();
    route(
        &agent,
        "https://api.github.com/repos/octo/site/pulls/9",
        answer(
            200,
            &json!({"head": {"sha": "abc123"}, "html_url": "https://github.com/octo/site/pull/9"}),
        ),
    );
    route(
        &agent,
        "https://api.github.com/repos/octo/site/commits/abc123/check-runs?per_page=50&filter=latest",
        answer(
            200,
            &json!({"total_count": 3, "check_runs": [
                {"name": "build", "status": "completed", "conclusion": "success"},
                {"name": "lint", "status": "completed", "conclusion": "failure"},
                {"name": "e2e", "status": "in_progress"}
            ]}),
        ),
    );
    route(
        &agent,
        "https://api.github.com/repos/octo/site/commits/abc123/status?per_page=50",
        answer(
            200,
            &json!({"sha": "abc123", "statuses": [{"context": "ci/legacy", "state": "error"}]}),
        ),
    );
    let out = run(
        &agent,
        "checks",
        json!({"repository": "octo/site", "pullRequest": 9}),
    );
    let (message, data) = ok(&out);
    assert_eq!(
        message,
        "octo/site#9: failing (1 passed, 2 failed, 1 pending)"
    );
    assert_eq!(data["failing"], s("lint\nci/legacy"));
    assert_eq!(data["total"], ParamValue::Integer(4));
    assert_eq!(data["truncated"], ParamValue::Bool(false));
    assert_eq!(data["url"], s("https://github.com/octo/site/pull/9"));
    let out = run(&agent, "checks", json!({"repository": "octo/site"}));
    assert_eq!(failed(&out).0, "invalid-parameters");
    let out = run(
        &agent,
        "checks",
        json!({"repository": "octo/site", "ref": "a..b"}),
    );
    assert_eq!(failed(&out).0, "invalid-ref");
}

#[test]
fn lists_stop_after_ten_pages() {
    let agent = agent();
    let base = "https://api.github.com/repos/octo/site/commits/main/check-runs";
    route(
        &agent,
        base,
        answer(
            200,
            &json!({"total_count": 999, "check_runs": [
                {"name": "x", "status": "completed", "conclusion": "success"}
            ]}),
        )
        .header("link", &format!("<{base}?page=2>; rel=\"next\"")),
    );
    route(
        &agent,
        "https://api.github.com/repos/octo/site/commits/main/status",
        answer(200, &json!({"sha": "f00", "statuses": []})),
    );
    let out = run(
        &agent,
        "checks",
        json!({"repository": "octo/site", "ref": "main"}),
    );
    let (_, data) = ok(&out);
    assert_eq!(data["passed"], ParamValue::Integer(10));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
    assert_eq!(data["sha"], s("f00"));
    assert_eq!(data["url"], s("https://github.com/octo/site/commit/f00"));
    assert_eq!(agent.borrow().fetched.len(), 11);
}

#[test]
fn failures_have_the_builtins_codes() {
    let agent = agent();
    let cases = [
        (
            "a/unauthorized",
            answer(401, &json!({"message": "Bad credentials"})),
            "unauthorized",
        ),
        (
            "a/forbidden",
            answer(403, &json!({"message": "SSO required"})),
            "forbidden",
        ),
        (
            "a/missing",
            answer(404, &json!({"message": "Not Found"})),
            "not-found",
        ),
        ("a/broken", answer(502, &json!({})), "github-error"),
        (
            "a/limited",
            answer(403, &json!({"message": "API rate limit exceeded"}))
                .header("x-ratelimit-remaining", "0")
                .header("x-ratelimit-reset", "1794823230"),
            "rate-limited",
        ),
        (
            "a/secondary",
            answer(429, &json!({})).header("retry-after", "30"),
            "rate-limited",
        ),
        (
            "a/html",
            Answer {
                status: 200,
                headers: Vec::new(),
                body: "<html>".into(),
            },
            "invalid-response",
        ),
        (
            "a/shape",
            answer(200, &json!({"name": "x"})),
            "invalid-response",
        ),
    ];
    for (repo, reply, code) in cases {
        route(
            &agent,
            &format!("https://api.github.com/repos/{repo}"),
            reply,
        );
        let out = run(&agent, "repository", json!({"repository": repo}));
        assert_eq!(failed(&out).0, code, "{repo}");
    }
    // The reset is counted from GitHub's Date (10:00:00), 30 s later.
    let out = run(&agent, "repository", json!({"repository": "a/limited"}));
    assert_eq!(
        failed(&out).1,
        "GitHub's rate limit is reached; try again in 30 s"
    );
    let out = run(&agent, "repository", json!({"repository": "a/secondary"}));
    assert_eq!(
        failed(&out).1,
        "GitHub's rate limit is reached; try again in 30 s"
    );
    // No answer, and no network kind (an older agent, or no route): the
    // agent's reason, retryable.
    let out = run(&agent, "repository", json!({"repository": "a/offline"}));
    assert_eq!(failed(&out).0, "request-failed");
}

/// A request the network fails reports the built-in's transport code
/// (`deckpilot-net`'s), with its retryability, through the agent's
/// network kind (ABI 1 `dp_features`).
#[test]
fn network_failures_report_the_builtins_codes() {
    use deckpilot_plugin_sdk::NetworkErrorKind as K;
    let agent = agent();
    for (kind, code, retryable) in [
        (K::DnsFailed, "dns-failed", true),
        (K::ConnectionFailed, "connection-failed", true),
        (K::TimedOut, "timed-out", true),
        (K::TlsFailed, "tls-failed", false),
        (K::TooManyRedirects, "too-many-redirects", false),
        (K::InsecureRedirect, "insecure-redirect", false),
        (K::BlockedAddress, "blocked-address", false),
    ] {
        agent.borrow_mut().unreachable =
            BTreeMap::from([("https://api.github.com/repos/octo/site".to_owned(), kind)]);
        let out = run(&agent, "repository", json!({"repository": "octo/site"}));
        match &out {
            ActionOutcome::Failed {
                code: got,
                retryable: r,
                ..
            } => {
                assert_eq!(got, code);
                assert_eq!(*r, retryable, "{code}");
            }
            other => panic!("{code}: {other:?}"),
        }
    }
    // A kind added later is the agent's reason, as before.
    agent.borrow_mut().unreachable = BTreeMap::from([(
        "https://api.github.com/repos/octo/site".to_owned(),
        K::Other,
    )]);
    let out = run(&agent, "repository", json!({"repository": "octo/site"}));
    assert_eq!(failed(&out).0, "request-failed");
}

/// The module reads the network kind: `dp_features` reports it.
#[test]
fn the_module_opts_in_to_network_kinds() {
    assert_ne!(
        u64::try_from(dp_features()).unwrap()
            & deckpilot_plugin_abi::guest::features::NETWORK_ERROR_KIND,
        0
    );
}

#[test]
fn the_time_limit_is_counted_with_githubs_date() {
    let agent = agent();
    agent.borrow_mut().settings = json!({"timeoutSeconds": 5});
    route(
        &agent,
        "https://api.github.com/repos/octo/site/pulls/9",
        answer(200, &json!({"head": {"sha": "abc123"}})),
    );
    let mut late = answer(200, &json!({"total_count": 0, "check_runs": []}));
    late.headers = vec![("date".into(), "Mon, 16 Nov 2026 10:00:06 GMT".into())];
    route(
        &agent,
        "https://api.github.com/repos/octo/site/commits/abc123/check-runs",
        late,
    );
    let out = run(
        &agent,
        "checks",
        json!({"repository": "octo/site", "pullRequest": 9}),
    );
    assert_eq!(failed(&out).0, "timed-out");
    assert_eq!(agent.borrow().fetched.len(), 2, "no third request");
}

#[test]
fn only_github_pages_are_opened() {
    let agent = agent();
    let out = run(
        &agent,
        "open",
        json!({"repository": "octo/site", "pullRequest": 12}),
    );
    let (message, data) = ok(&out);
    assert_eq!(message, "opened https://github.com/octo/site/pull/12");
    assert_eq!(data["url"], s("https://github.com/octo/site/pull/12"));
    let out = run(
        &agent,
        "open",
        json!({"url": "https://GitHub.com/octo/site/pull/12/files?token=q#diff"}),
    );
    assert_eq!(
        ok(&out).1["url"],
        s("https://github.com/octo/site/pull/12/files")
    );
    assert_eq!(
        agent.borrow().opened,
        [
            "https://github.com/octo/site/pull/12",
            "https://github.com/octo/site/pull/12/files?token=q#diff"
        ]
    );
    for url in [
        "http://github.com/a",
        "https://gist.github.com/a",
        "https://user@github.com/a",
    ] {
        let out = run(&agent, "open", json!({"url": url}));
        assert_eq!(failed(&out).0, "url-not-allowed", "{url}");
    }
    let out = run(&agent, "open", json!({}));
    assert_eq!(failed(&out).0, "invalid-repository");
    assert_eq!(agent.borrow().opened.len(), 2);
    assert!(agent.borrow().fetched.is_empty(), "open sends no request");
}

#[test]
fn outputs_stay_within_one_value() {
    let lines: Vec<String> = (0..400).map(|n| format!("{n:0>60}")).collect();
    let (text, kept) = bounded_lines(&lines);
    assert!(text.len() <= MAX_OUTPUT_TEXT);
    assert_eq!(kept, MAX_OUTPUT_TEXT / 61);
    assert_eq!(text.lines().count(), kept);
}

#[test]
fn rate_limit_waits() {
    let response = |headers: &[(&str, &str)]| Response {
        status: 403,
        headers: headers
            .iter()
            .map(|(n, v)| deckpilot_plugin_sdk::Header {
                name: (*n).into(),
                value: (*v).into(),
            })
            .collect(),
        body: None,
    };
    let reset = response(&[
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-reset", "1100"),
    ]);
    assert_eq!(
        api::rate_limit_wait(403, &reset, None, Some(1000)),
        Some(100)
    );
    assert_eq!(api::rate_limit_wait(403, &reset, None, None), Some(60));
    let after = response(&[("retry-after", "30")]);
    assert_eq!(api::rate_limit_wait(429, &after, None, None), Some(30));
    assert_eq!(
        api::rate_limit_wait(429, &response(&[]), None, None),
        Some(60)
    );
    let plenty = response(&[("x-ratelimit-remaining", "12")]);
    assert_eq!(api::rate_limit_wait(403, &plenty, None, None), None);
    assert_eq!(
        api::rate_limit_wait(
            403,
            &plenty,
            Some("You have exceeded a secondary rate limit"),
            None
        ),
        Some(60)
    );
    assert_eq!(
        api::rate_limit_wait(500, &after, Some("rate limit"), None),
        None
    );
    assert_eq!(
        api::http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
        Some(784_111_777)
    );
}
