//! Native unit tests: the definition agrees with manifest.json, the
//! manifest is valid against the schema, and every action behaves, with
//! the agent replaced by `MockHost`. The comparison with the built-in
//! module on the same answers is `apps/agent/tests/plugins_gitlab.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use deckpilot_first_party_support::MAX_OUTPUT_TEXT;
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
const API: &str = "https://gitlab.com/api/v4";
/// A self-managed GitLab under a path.
const OWN: &str = "https://code.example.com/gitlab/api/v4";

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).unwrap()
}

/// One canned answer.
#[derive(Debug, Clone)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    /// The agent refuses it as larger than an answer may be.
    too_large: bool,
}

fn answer(status: u16, body: &Value) -> Answer {
    Answer {
        status,
        headers: vec![("date".into(), DATE.into())],
        body: body.to_string(),
        too_large: false,
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
        settings: json!({"apiUrl": API, "timeoutSeconds": 20}),
        token: true,
        ..Agent::default()
    }))
}

fn own_agent() -> Shared {
    let agent = agent();
    agent.borrow_mut().settings = json!({ "apiUrl": OWN });
    agent
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
                assert_eq!(auth.scheme, AuthScheme::Header);
                assert!(f.headers.iter().any(|h| h.name == "Accept"));
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
                    Some(a) if a.too_large => CallResult::error(
                        CallErrorCode::Failed,
                        "the response is larger than 524288 bytes",
                    ),
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
    let slot = &manifest["contributes"]["credentials"][0];
    assert_eq!(slot["key"], api::TOKEN_SLOT);
    // The token goes to gitlab.com or the server of `apiUrl`, in GitLab's
    // header, and in no other way.
    assert_eq!(slot["hosts"], json!(["gitlab.com"]));
    assert_eq!(slot["userOrigin"], api::API_URL);
    assert_eq!(slot["schemes"], json!(["header"]));
    assert_eq!(slot["headerName"], "PRIVATE-TOKEN");
    let fields = manifest["contributes"]["settings"]["fields"]
        .as_array()
        .unwrap();
    let settings: Vec<&str> = fields.iter().map(|f| f["key"].as_str().unwrap()).collect();
    assert_eq!(settings, [api::API_URL, api::TIMEOUT]);
    assert_eq!(fields[0]["default"]["value"], api::DEFAULT_API_URL);
    // GET to gitlab.com or the entered server, the token there, pages
    // there: nothing else.
    assert_eq!(
        manifest["permissions"],
        json!([
            {"kind": "network", "hosts": ["gitlab.com"], "methods": ["GET"],
             "userOrigins": [{"setting": "apiUrl", "pathPrefix": true, "privateNetwork": true}],
             "justification": manifest["permissions"][0]["justification"]},
            {"kind": "credentials", "refs": ["token"],
             "justification": manifest["permissions"][1]["justification"]},
            {"kind": "openUrl", "hosts": ["gitlab.com"], "userOrigins": ["apiUrl"],
             "justification": manifest["permissions"][2]["justification"]},
        ])
    );
}

#[test]
fn the_current_user() {
    let agent = agent();
    route(
        &agent,
        &format!("{API}/user"),
        answer(
            200,
            &json!({"id": 7, "username": "dev", "name": "The\u{202e} Dev",
                    "web_url": "https://gitlab.com/dev"}),
        )
        .header("ratelimit-remaining", "1999"),
    );
    let out = run(&agent, "current-user", json!({}));
    let (message, data) = ok(&out);
    assert_eq!(message, "signed in to GitLab as dev");
    assert_eq!(data["username"], s("dev"));
    assert_eq!(data["name"], s("The  Dev"));
    assert_eq!(data["url"], s("https://gitlab.com/dev"));
    assert_eq!(data["rateLimitRemaining"], ParamValue::Integer(1999));
}

#[test]
fn a_self_managed_gitlab_under_a_path() {
    let agent = own_agent();
    // No `web_url` in the answer: the page is under the web address, the
    // API URL without its /api/v4.
    route(
        &agent,
        &format!("{OWN}/user"),
        answer(200, &json!({"id": 7, "username": "dev"})),
    );
    let out = run(&agent, "current-user", json!({}));
    assert_eq!(ok(&out).1["url"], s("https://code.example.com/gitlab/dev"));
    // A project path is one encoded segment under the API.
    route(
        &agent,
        &format!("{OWN}/projects/group%2Fsub%2Fapp"),
        answer(
            200,
            &json!({"path_with_namespace": "group/sub/app", "visibility": "internal"}),
        ),
    );
    let out = run(&agent, "project", json!({"project": "group/sub/app"}));
    let (message, data) = ok(&out);
    assert_eq!(message, "read group/sub/app");
    assert_eq!(
        data["url"],
        s("https://code.example.com/gitlab/group/sub/app")
    );
    assert_eq!(
        agent.borrow().fetched,
        [
            format!("{OWN}/user"),
            format!("{OWN}/projects/group%2Fsub%2Fapp")
        ]
    );
}

#[test]
fn a_missing_token_or_an_address_that_is_no_api_is_refused_before_any_request() {
    let agent = agent();
    agent.borrow_mut().token = false;
    let out = run(&agent, "project", json!({"project": "a/b"}));
    let (code, message) = failed(&out);
    assert_eq!(code, "credential-missing");
    assert!(message.contains("Settings › Plugins › GitLab"), "{message}");
    for bad in [
        "http://gitlab.com/api/v4",
        "http://127.0.0.1:8080/api/v4",
        "https://u:p@gitlab.com/api/v4",
        "https://gitlab.com/api/v4?x=1",
        "not a url",
    ] {
        let agent = self::agent();
        agent.borrow_mut().settings = json!({ "apiUrl": bad });
        let out = run(&agent, "current-user", json!({}));
        assert_eq!(failed(&out).0, "invalid-api-url", "{bad}");
        assert!(agent.borrow().fetched.is_empty());
        // Open checks the setting too, and opens nothing.
        let out = run(&agent, "open", json!({"project": "a/b"}));
        assert_eq!(failed(&out).0, "invalid-api-url", "{bad}");
        assert!(agent.borrow().opened.is_empty());
    }
    // Nothing saved: GitLab.com.
    let agent = self::agent();
    agent.borrow_mut().settings = json!({});
    route(
        &agent,
        &format!("{API}/user"),
        answer(200, &json!({"id": 1, "username": "dev"})),
    );
    assert_eq!(
        ok(&run(&agent, "current-user", json!({}))).1["url"],
        s("https://gitlab.com/dev")
    );
}

#[test]
fn project_information() {
    let agent = agent();
    route(
        &agent,
        &format!("{API}/projects/group%2Fapp"),
        answer(
            200,
            &json!({
                "path_with_namespace": "group/app", "default_branch": "main",
                "visibility": "private", "archived": false, "open_issues_count": 7,
                "star_count": 42, "forks_count": 3, "web_url": "https://gitlab.com/group/app",
                "description": "", "last_activity_at": "2026-11-01T10:00:00Z"
            }),
        ),
    );
    let out = run(&agent, "project", json!({"project": " group/app.git "}));
    let (message, data) = ok(&out);
    assert_eq!(message, "read group/app");
    assert_eq!(data["visibility"], s("private"));
    assert_eq!(data["archived"], ParamValue::Bool(false));
    assert_eq!(data["openIssues"], ParamValue::Integer(7));
    assert_eq!(data["stars"], ParamValue::Integer(42));
    assert!(!data.contains_key("description"));
    assert_eq!(data["lastActivityAt"], s("2026-11-01T10:00:00Z"));
    // By numeric id; issues turned off.
    route(
        &agent,
        &format!("{API}/projects/42"),
        answer(200, &json!({"path_with_namespace": "group/other"})),
    );
    let out = run(&agent, "project", json!({"project": "42"}));
    let data = ok(&out).1;
    assert_eq!(data["visibility"], s("unknown"));
    assert!(!data.contains_key("openIssues"));
    assert!(!data.contains_key("defaultBranch"));
    for bad in ["not a project", "group", "a//b", "../etc"] {
        let out = run(&agent, "project", json!({ "project": bad }));
        assert_eq!(failed(&out).0, "invalid-project", "{bad}");
    }
}

fn merge_request_items(from: usize, count: usize) -> Vec<Value> {
    (from..from + count)
        .map(|n| {
            json!({
                "iid": n, "title": format!("Change {n}"), "draft": n % 2 == 0,
                "references": {"full": format!("group/app!{n}")},
                "author": {"username": "dev"},
                "web_url": format!("https://gitlab.com/group/app/-/merge_requests/{n}")
            })
        })
        .collect()
}

const LIST: &str = "https://gitlab.com/api/v4/projects/group%2Fapp/merge_requests\
                    ?state=opened&order_by=updated_at&sort=desc&per_page=";

#[test]
fn merge_requests_up_to_fifty_are_one_request_and_more_follow_the_next_link() {
    let agent = agent();
    route(
        &agent,
        &format!("{LIST}20"),
        answer(200, &json!(merge_request_items(1, 20))).header("x-total", "120"),
    );
    let out = run(&agent, "merge-requests", json!({"project": "group/app"}));
    let (message, data) = ok(&out);
    assert_eq!(message, "120 open merge requests in group/app (all)");
    assert_eq!(data["count"], ParamValue::Integer(120));
    assert_eq!(data["countExact"], ParamValue::Bool(true));
    assert_eq!(data["listed"], ParamValue::Integer(20));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
    assert_eq!(
        data["firstUrl"],
        s("https://gitlab.com/group/app/-/merge_requests/1")
    );
    let ParamValue::String(items) = &data["items"] else {
        panic!()
    };
    assert_eq!(
        items.lines().take(2).collect::<Vec<_>>(),
        [
            "group/app!1 Change 1 (@dev)",
            "group/app!2 Change 2 (@dev) [draft]"
        ]
    );
    assert_eq!(agent.borrow().fetched, [format!("{LIST}20")]);

    // 70: a page of 50, then GitLab's next page, of which 20 are listed.
    let next = "https://gitlab.com/api/v4/projects/1/merge_requests?page=2&per_page=50";
    route(
        &agent,
        &format!("{LIST}50"),
        answer(200, &json!(merge_request_items(1, 50)))
            .header("x-total", "120")
            .header("link", &format!("<{next}>; rel=\"next\"")),
    );
    route(
        &agent,
        next,
        answer(200, &json!(merge_request_items(51, 50)))
            .header("link", "<https://gitlab.com/api/v4/x?page=3>; rel=\"next\""),
    );
    let out = run(
        &agent,
        "merge-requests",
        json!({"project": "group/app", "limit": 70}),
    );
    let data = ok(&out).1;
    assert_eq!(data["listed"], ParamValue::Integer(70));
    assert_eq!(data["count"], ParamValue::Integer(120));
    let ParamValue::String(items) = &data["items"] else {
        panic!()
    };
    assert_eq!(
        items.lines().last(),
        Some("group/app!70 Change 70 (@dev) [draft]")
    );
    assert_eq!(
        agent.borrow().fetched[1..],
        [format!("{LIST}50"), next.to_owned()]
    );
}

#[test]
fn a_list_without_a_total_is_counted_in_bounded_pages() {
    let agent = agent();
    // No X-Total and no next page: the list is the count.
    route(
        &agent,
        &format!("{LIST}20"),
        answer(200, &json!(merge_request_items(1, 3))),
    );
    let out = run(&agent, "merge-requests", json!({"project": "group/app"}));
    let (message, data) = ok(&out);
    assert_eq!(message, "3 open merge requests in group/app (all)");
    assert_eq!(data["truncated"], ParamValue::Bool(false));

    // No X-Total but more pages: counted 50 at a time, here 2 pages.
    let agent = self::agent();
    let page = |n: usize| format!("https://gitlab.com/api/v4/count?page={n}");
    route(
        &agent,
        &format!("{LIST}20"),
        answer(200, &json!(merge_request_items(1, 20))).header(
            "link",
            "<https://gitlab.com/api/v4/listing?page=2>; rel=\"next\"",
        ),
    );
    route(
        &agent,
        &format!("{LIST}50"),
        answer(200, &json!(merge_request_items(1, 50)))
            .header("link", &format!("<{}>; rel=\"next\"", page(2))),
    );
    route(
        &agent,
        &page(2),
        answer(200, &json!(merge_request_items(51, 30))),
    );
    let out = run(&agent, "merge-requests", json!({"project": "group/app"}));
    let (message, data) = ok(&out);
    assert_eq!(message, "80 open merge requests in group/app (all)");
    assert_eq!(data["countExact"], ParamValue::Bool(true));
    assert_eq!(data["listed"], ParamValue::Integer(20));
    assert_eq!(data["truncated"], ParamValue::Bool(true));

    // More than 500: "at least", after ten pages and no more.
    let agent = self::agent();
    route(
        &agent,
        &format!("{LIST}20"),
        answer(200, &json!(merge_request_items(1, 20))).header(
            "link",
            "<https://gitlab.com/api/v4/listing?page=2>; rel=\"next\"",
        ),
    );
    route(
        &agent,
        &format!("{LIST}50"),
        answer(200, &json!(merge_request_items(1, 50)))
            .header("link", &format!("<{}>; rel=\"next\"", page(2))),
    );
    route(
        &agent,
        "https://gitlab.com/api/v4/count",
        answer(200, &json!(merge_request_items(1, 50)))
            .header("link", &format!("<{}>; rel=\"next\"", page(3))),
    );
    let out = run(&agent, "merge-requests", json!({"project": "group/app"}));
    let (message, data) = ok(&out);
    assert_eq!(
        message,
        "at least 500 open merge requests in group/app (all)"
    );
    assert_eq!(data["count"], ParamValue::Integer(500));
    assert_eq!(data["countExact"], ParamValue::Bool(false));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
    assert_eq!(agent.borrow().fetched.len(), 1 + 10);
}

#[test]
fn pages_too_large_for_one_answer_are_read_ten_at_a_time() {
    let agent = agent();
    let mut large = answer(200, &json!([]));
    large.too_large = true;
    route(&agent, &format!("{LIST}20"), large.clone());
    route(
        &agent,
        &format!("{LIST}10"),
        answer(200, &json!(merge_request_items(1, 10)))
            .header("x-total", "30")
            .header(
                "link",
                "<https://gitlab.com/api/v4/small?page=2>; rel=\"next\"",
            ),
    );
    route(
        &agent,
        "https://gitlab.com/api/v4/small?page=2",
        answer(200, &json!(merge_request_items(11, 10))).header(
            "link",
            "<https://gitlab.com/api/v4/small?page=3>; rel=\"next\"",
        ),
    );
    let out = run(&agent, "merge-requests", json!({"project": "group/app"}));
    let (message, data) = ok(&out);
    assert_eq!(message, "30 open merge requests in group/app (all)");
    assert_eq!(data["listed"], ParamValue::Integer(20));
    assert_eq!(agent.borrow().fetched.len(), 3);
    // Ten are too large as well: the built-in's code.
    route(&agent, &format!("{LIST}10"), large);
    let out = run(&agent, "merge-requests", json!({"project": "group/app"}));
    let (code, message) = failed(&out);
    assert_eq!(code, "invalid-response");
    assert_eq!(
        message,
        "GitLab's answer for the merge requests of group/app is larger than 512 KiB"
    );
}

#[test]
fn filters_about_you_ask_who_you_are() {
    let agent = agent();
    route(
        &agent,
        &format!("{API}/user"),
        answer(200, &json!({"id": 7, "username": "dev"})),
    );
    let mine = "https://gitlab.com/api/v4/merge_requests?state=opened&order_by=updated_at\
                &sort=desc&author_id=7&scope=all&per_page=5";
    route(
        &agent,
        mine,
        answer(200, &json!(merge_request_items(1, 2))).header("x-total", "2"),
    );
    let out = run(
        &agent,
        "merge-requests",
        json!({"filter": "author-me", "limit": 5}),
    );
    assert_eq!(ok(&out).0, "2 open merge requests (author-me)");
    let reviews = "https://gitlab.com/api/v4/projects/group%2Fapp/merge_requests?state=opened\
                   &order_by=updated_at&sort=desc&reviewer_id=7&per_page=20";
    route(
        &agent,
        reviews,
        answer(200, &json!(merge_request_items(1, 1))).header("x-total", "1"),
    );
    let out = run(&agent, "review-requests", json!({"project": "group/app"}));
    assert_eq!(
        ok(&out).0,
        "1 merge requests wait for your review in group/app"
    );
    assert_eq!(
        agent.borrow().fetched,
        [
            format!("{API}/user"),
            mine.to_owned(),
            format!("{API}/user"),
            reviews.to_owned()
        ]
    );
    // All merge requests need a project.
    let out = run(&agent, "merge-requests", json!({}));
    let (code, message) = failed(&out);
    assert_eq!(code, "invalid-project");
    assert_eq!(
        message,
        "choose a project to list all its open merge requests"
    );
}

#[test]
fn a_next_page_outside_the_api_is_refused() {
    let agent = own_agent();
    let list = format!(
        "{OWN}/projects/group%2Fapp/merge_requests?state=opened&order_by=updated_at&sort=desc\
         &per_page=50"
    );
    for outside in [
        "https://evil.example.net/gitlab/api/v4/x?page=2",
        "https://code.example.com/gitlab/admin?page=2",
        "https://code.example.com/other/api/v4/x?page=2",
        "http://code.example.com/gitlab/api/v4/x?page=2",
    ] {
        route(
            &agent,
            &list,
            answer(200, &json!(merge_request_items(1, 50)))
                .header("link", &format!("<{outside}>; rel=\"next\"")),
        );
        let out = run(
            &agent,
            "merge-requests",
            json!({"project": "group/app", "limit": 100}),
        );
        let (code, message) = failed(&out);
        assert_eq!(code, "invalid-response", "{outside}");
        assert_eq!(message, "GitLab pointed to a next page outside its API");
    }
    assert!(agent.borrow().fetched.iter().all(|url| url == &list));
}

fn jobs(statuses: &[(&str, &str, bool)]) -> Vec<Value> {
    statuses
        .iter()
        .map(|(name, status, allow_failure)| {
            json!({"name": name, "status": status, "allow_failure": allow_failure})
        })
        .collect()
}

#[test]
fn the_pipeline_of_a_merge_request_counts_and_names_the_failing_jobs() {
    let agent = agent();
    route(
        &agent,
        &format!("{API}/projects/group%2Fapp/merge_requests/9"),
        answer(
            200,
            &json!({
                "web_url": "https://gitlab.com/group/app/-/merge_requests/9",
                "head_pipeline": {"id": 501, "status": "failed", "sha": "abc123",
                                  "web_url": "https://gitlab.com/group/app/-/pipelines/501"}
            }),
        ),
    );
    let jobs_url = format!("{API}/projects/group%2Fapp/pipelines/501/jobs?per_page=50");
    route(
        &agent,
        &jobs_url,
        answer(
            200,
            &json!(jobs(&[
                ("build", "success", false),
                ("lint", "failed", true),
                ("test", "failed", false),
                ("deploy\nprod", "canceled", false),
                ("docs", "skipped", false),
                ("release", "manual", false),
                ("e2e", "running", false),
                ("scan", "created", false),
            ])),
        )
        .header("x-total", "8"),
    );
    let out = run(
        &agent,
        "pipeline",
        json!({"project": "group/app", "mergeRequest": 9}),
    );
    let (message, data) = ok(&out);
    assert_eq!(
        message,
        "group/app!9: failing (2 passed, 2 failed, 2 pending)"
    );
    assert_eq!(data["state"], s("failing"));
    assert_eq!(data["status"], s("failed"));
    assert_eq!(data["pipelineId"], ParamValue::Integer(501));
    assert_eq!(data["sha"], s("abc123"));
    assert_eq!(data["total"], ParamValue::Integer(8));
    assert_eq!(data["skipped"], ParamValue::Integer(2));
    assert_eq!(data["failing"], s("test\ndeploy prod"));
    assert_eq!(data["truncated"], ParamValue::Bool(false));
    // The merge request's page, not the pipeline's.
    assert_eq!(
        data["url"],
        s("https://gitlab.com/group/app/-/merge_requests/9")
    );
    assert_eq!(agent.borrow().fetched[1], jobs_url);
}

#[test]
fn the_latest_pipeline_of_a_ref_or_a_commit() {
    let agent = agent();
    let by_ref = format!(
        "{API}/projects/group%2Fapp/pipelines?ref=feature%2Flogin&order_by=id&sort=desc&per_page=1"
    );
    route(
        &agent,
        &by_ref,
        answer(
            200,
            &json!([{"id": 77, "status": "success", "sha": "def456",
                     "web_url": "https://gitlab.com/group/app/-/pipelines/77"}]),
        ),
    );
    route(
        &agent,
        &format!("{API}/projects/group%2Fapp/pipelines/77/jobs"),
        answer(200, &json!(jobs(&[("build", "success", false)]))),
    );
    let out = run(
        &agent,
        "pipeline",
        json!({"project": "group/app", "ref": "feature/login"}),
    );
    let (message, data) = ok(&out);
    assert_eq!(
        message,
        "group/app@feature/login: ok (1 passed, 0 failed, 0 pending)"
    );
    assert_eq!(
        data["url"],
        s("https://gitlab.com/group/app/-/pipelines/77")
    );
    // A full commit id is asked for as `sha`; no pipeline is not a failure.
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let by_sha =
        format!("{API}/projects/group%2Fapp/pipelines?sha={sha}&order_by=id&sort=desc&per_page=1");
    route(&agent, &by_sha, answer(200, &json!([])));
    let out = run(
        &agent,
        "pipeline",
        json!({"project": "group/app", "ref": sha}),
    );
    let (message, data) = ok(&out);
    assert_eq!(message, format!("group/app@{sha}: no pipeline"));
    assert_eq!(data["state"], s("none"));
    assert_eq!(data["total"], ParamValue::Integer(0));
    assert_eq!(data["failing"], s(""));
    assert!(!data.contains_key("url"));
    assert!(agent.borrow().fetched.contains(&by_sha));
    // Neither a ref nor a merge request; a ref that is none.
    let out = run(&agent, "pipeline", json!({"project": "group/app"}));
    assert_eq!(failed(&out).0, "invalid-parameters");
    let out = run(
        &agent,
        "pipeline",
        json!({"project": "group/app", "ref": "a..b"}),
    );
    assert_eq!(failed(&out).0, "invalid-ref");
}

#[test]
fn jobs_stop_after_five_hundred() {
    let agent = agent();
    route(
        &agent,
        &format!("{API}/projects/1/merge_requests/2"),
        answer(
            200,
            &json!({"head_pipeline": {"id": 3, "status": "running"}}),
        ),
    );
    let many: Vec<(&str, &str, bool)> = (0..50).map(|_| ("job", "success", false)).collect();
    route(
        &agent,
        &format!("{API}/projects/1/pipelines/3/jobs"),
        answer(200, &json!(jobs(&many))).header(
            "link",
            &format!("<{API}/projects/1/pipelines/3/jobs?page=next>; rel=\"next\""),
        ),
    );
    let out = run(
        &agent,
        "pipeline",
        json!({"project": "1", "mergeRequest": 2}),
    );
    let (message, data) = ok(&out);
    assert_eq!(message, "1!2: pending (500 passed, 0 failed, 0 pending)");
    assert_eq!(data["total"], ParamValue::Integer(500));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
    assert_eq!(agent.borrow().fetched.len(), 1 + 10);
}

#[test]
fn failures_have_the_builtins_codes() {
    let agent = agent();
    let cases = [
        (
            "a/unauthorized",
            answer(401, &json!({"message": "401 Unauthorized"})),
            "unauthorized",
        ),
        (
            "a/forbidden",
            answer(403, &json!({"error_description": "insufficient_scope"})),
            "forbidden",
        ),
        (
            "a/missing",
            answer(404, &json!({"message": "404 Project Not Found"})),
            "not-found",
        ),
        (
            "a/broken",
            answer(502, &json!({"error": "bad gateway"})),
            "gitlab-error",
        ),
        (
            "a/limited",
            answer(429, &json!({})).header("ratelimit-reset", "1794823230"),
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
                too_large: false,
            },
            "invalid-response",
        ),
        (
            "a/shape",
            answer(200, &json!({"name": "x"})),
            "invalid-response",
        ),
    ];
    for (project, reply, code) in cases {
        route(
            &agent,
            &format!("{API}/projects/{}", project.replace('/', "%2F")),
            reply,
        );
        let out = run(&agent, "project", json!({ "project": project }));
        assert_eq!(failed(&out).0, code, "{project}");
    }
    let message = |project: &str| {
        let out = run(&agent, "project", json!({ "project": project }));
        failed(&out).1.to_owned()
    };
    assert_eq!(
        message("a/forbidden"),
        "the token may not read the project a/forbidden (403): insufficient_scope"
    );
    assert_eq!(
        message("a/missing"),
        "the project a/missing was not found, or the token cannot see it (404)"
    );
    assert_eq!(
        message("a/broken"),
        "GitLab answered 502 for the project a/broken: bad gateway"
    );
    // The reset is counted from GitLab's Date (10:00:00), 30 s later.
    assert_eq!(
        message("a/limited"),
        "GitLab's rate limit is reached; try again in 30 s"
    );
    assert_eq!(
        message("a/secondary"),
        "GitLab's rate limit is reached; try again in 30 s"
    );
    // A rate-limit failure can be tried again.
    let out = run(&agent, "project", json!({"project": "a/limited"}));
    assert!(matches!(
        out,
        ActionOutcome::Failed {
            retryable: true,
            ..
        }
    ));
    // No answer, and no network kind (an older agent, or no route): the
    // agent's reason, retryable.
    let out = run(&agent, "project", json!({"project": "a/offline"}));
    assert_eq!(failed(&out).0, "request-failed");
}

/// A request the network fails reports the built-in's transport code
/// (`deckpilot-net`'s), with its retryability, through the agent's
/// network kind (ABI 1 `dp_features`).
#[test]
fn network_failures_report_the_builtins_codes() {
    use deckpilot_plugin_sdk::NetworkErrorKind as K;
    let agent = agent();
    let url = format!("{API}/projects/group%2Fapp");
    for (kind, code, retryable) in [
        (K::DnsFailed, "dns-failed", true),
        (K::ConnectionFailed, "connection-failed", true),
        (K::TimedOut, "timed-out", true),
        (K::TlsFailed, "tls-failed", false),
        (K::TooManyRedirects, "too-many-redirects", false),
        (K::InsecureRedirect, "insecure-redirect", false),
        (K::BlockedAddress, "blocked-address", false),
    ] {
        agent.borrow_mut().unreachable = BTreeMap::from([(url.clone(), kind)]);
        let out = run(&agent, "project", json!({"project": "group/app"}));
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
    agent.borrow_mut().unreachable = BTreeMap::from([(url, K::Other)]);
    let out = run(&agent, "project", json!({"project": "group/app"}));
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
fn the_time_limit_is_counted_with_gitlabs_date() {
    let agent = agent();
    agent.borrow_mut().settings = json!({"timeoutSeconds": 5});
    route(
        &agent,
        &format!("{API}/projects/group%2Fapp/merge_requests/9"),
        answer(
            200,
            &json!({"head_pipeline": {"id": 501, "status": "running"}}),
        ),
    );
    let mut late = answer(200, &json!(jobs(&[("build", "success", false)])));
    late.headers = vec![
        ("date".into(), "Mon, 16 Nov 2026 10:00:06 GMT".into()),
        (
            "link".into(),
            format!("<{API}/projects/group%2Fapp/pipelines/501/jobs?page=2>; rel=\"next\""),
        ),
    ];
    route(
        &agent,
        &format!("{API}/projects/group%2Fapp/pipelines/501/jobs"),
        late,
    );
    let out = run(
        &agent,
        "pipeline",
        json!({"project": "group/app", "mergeRequest": 9}),
    );
    let (code, message) = failed(&out);
    assert_eq!(code, "timed-out");
    assert_eq!(message, "GitLab did not answer within the time limit");
    assert_eq!(agent.borrow().fetched.len(), 2, "no third request");
}

#[test]
fn only_pages_of_the_configured_gitlab_are_opened() {
    let agent = agent();
    let out = run(
        &agent,
        "open",
        json!({"project": "group/app", "mergeRequest": 12}),
    );
    let (message, data) = ok(&out);
    assert_eq!(
        message,
        "opened https://gitlab.com/group/app/-/merge_requests/12"
    );
    assert_eq!(
        data["url"],
        s("https://gitlab.com/group/app/-/merge_requests/12")
    );
    let out = run(
        &agent,
        "open",
        json!({"url": "https://GitLab.com/group/app/-/pipelines/7?token=q#note"}),
    );
    assert_eq!(
        ok(&out).1["url"],
        s("https://gitlab.com/group/app/-/pipelines/7")
    );
    assert_eq!(
        agent.borrow().opened,
        [
            "https://gitlab.com/group/app/-/merge_requests/12",
            "https://gitlab.com/group/app/-/pipelines/7?token=q#note"
        ]
    );
    for url in [
        "http://gitlab.com/a",
        "https://about.gitlab.com/a",
        "https://user@gitlab.com/a",
        "https://code.example.com/gitlab/group/app",
    ] {
        let out = run(&agent, "open", json!({ "url": url }));
        let (code, message) = failed(&out);
        assert_eq!(code, "url-not-allowed", "{url}");
        assert_eq!(message, "only https pages on gitlab.com can be opened");
    }
    let out = run(&agent, "open", json!({}));
    assert_eq!(failed(&out).0, "invalid-project");
    let out = run(&agent, "open", json!({"project": "42"}));
    let (code, message) = failed(&out);
    assert_eq!(code, "invalid-project");
    assert_eq!(
        message,
        "a project is opened by its path (group/project), not its id"
    );
    assert_eq!(agent.borrow().opened.len(), 2);
    assert!(agent.borrow().fetched.is_empty(), "open sends no request");

    // A self-managed GitLab under a path: its pages, and only under it.
    let agent = own_agent();
    let out = run(&agent, "open", json!({"project": "group/app"}));
    assert_eq!(
        ok(&out).1["url"],
        s("https://code.example.com/gitlab/group/app")
    );
    for url in [
        "https://gitlab.com/group/app",
        "https://code.example.com/other/group/app",
        "https://code.example.com/gitlab/../admin",
        "https://code.example.com:8443/gitlab/group/app",
    ] {
        let out = run(&agent, "open", json!({ "url": url }));
        let (code, message) = failed(&out);
        assert_eq!(code, "url-not-allowed", "{url}");
        assert_eq!(
            message,
            "only https pages on code.example.com can be opened"
        );
    }
    assert_eq!(agent.borrow().opened.len(), 1);
}

#[test]
fn a_long_list_stays_within_one_output() {
    let agent = agent();
    let long: Vec<Value> = (1..=100)
        .map(|n| {
            json!({"iid": n, "title": "x".repeat(300), "author": {"username": "dev"},
                   "references": {"full": format!("group/app!{n}")}})
        })
        .collect();
    route(
        &agent,
        &format!("{LIST}50"),
        answer(200, &json!(long[..50]))
            .header("x-total", "100")
            .header(
                "link",
                "<https://gitlab.com/api/v4/long?page=2>; rel=\"next\"",
            ),
    );
    route(
        &agent,
        "https://gitlab.com/api/v4/long?page=2",
        answer(200, &json!(long[50..])),
    );
    let out = run(
        &agent,
        "merge-requests",
        json!({"project": "group/app", "limit": 100}),
    );
    let data = ok(&out).1;
    let ParamValue::String(items) = &data["items"] else {
        panic!()
    };
    assert!(items.len() <= MAX_OUTPUT_TEXT);
    let ParamValue::Integer(listed) = data["listed"] else {
        panic!()
    };
    assert_eq!(items.lines().count(), usize::try_from(listed).unwrap());
    assert!(listed < 100);
    assert_eq!(data["count"], ParamValue::Integer(100));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
}

#[test]
fn rate_limit_waits() {
    let response = |headers: &[(&str, &str)]| Response {
        status: 429,
        headers: headers
            .iter()
            .map(|(n, v)| deckpilot_plugin_sdk::Header {
                name: (*n).into(),
                value: (*v).into(),
            })
            .collect(),
        body: None,
    };
    let reset = response(&[("RateLimit-Reset", "1100")]);
    assert_eq!(api::rate_limit_wait(429, &reset, Some(1000)), Some(100));
    assert_eq!(api::rate_limit_wait(429, &reset, None), Some(60));
    let after = response(&[("Retry-After", "30")]);
    assert_eq!(api::rate_limit_wait(429, &after, None), Some(30));
    assert_eq!(api::rate_limit_wait(429, &response(&[]), None), Some(60));
    // An HTTP date, counted from the answer's Date (1794466800 is 07:00:00).
    let date = response(&[("Retry-After", "Thu, 12 Nov 2026 07:00:45 GMT")]);
    assert_eq!(
        api::rate_limit_wait(429, &date, Some(1_794_466_800)),
        Some(45)
    );
    // A form not understood: the reset time, then a minute.
    let later = response(&[("Retry-After", "later")]);
    assert_eq!(api::rate_limit_wait(429, &later, Some(0)), Some(60));
    assert_eq!(api::rate_limit_wait(403, &after, Some(0)), None);
}
