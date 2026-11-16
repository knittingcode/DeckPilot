//! Native unit tests: the definition agrees with manifest.json, the
//! manifest is valid against the schema, and every action behaves, with
//! the agent replaced by `MockHost`. The comparison with the built-in
//! module on the same answers is `apps/agent/tests/plugins_jira.rs`.

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
/// A Jira Cloud site.
const CLOUD: &str = "https://team.atlassian.net";
/// A Data Center under a path.
const DC: &str = "https://jira.example.com/jira";

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

/// A stand-in agent: settings, a stored credential (or not), canned
/// answers by URL prefix (the longest matching prefix wins), opened URLs.
#[derive(Debug, Default)]
struct Agent {
    settings: Value,
    credential: bool,
    /// The site is not allowed: the agent refuses the slot.
    not_allowed: bool,
    /// The scheme every request must name.
    scheme: Option<AuthScheme>,
    answers: BTreeMap<String, Answer>,
    /// URLs that get no answer, with the agent's network kind.
    unreachable: BTreeMap<String, NetworkErrorKind>,
    fetched: Vec<String>,
    opened: Vec<String>,
}

type Shared = Rc<RefCell<Agent>>;

fn cloud() -> Shared {
    Rc::new(RefCell::new(Agent {
        settings: json!({"siteUrl": CLOUD, "deployment": "cloud", "timeoutSeconds": 20}),
        credential: true,
        scheme: Some(AuthScheme::Basic),
        ..Agent::default()
    }))
}

fn data_center() -> Shared {
    Rc::new(RefCell::new(Agent {
        settings: json!({"siteUrl": DC, "deployment": "data-center"}),
        credential: true,
        scheme: Some(AuthScheme::Bearer),
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
                assert_eq!(r.credential, api::CREDENTIAL_SLOT);
                if agent.not_allowed {
                    return CallResult::error(
                        CallErrorCode::NotPermitted,
                        "credential slot not granted",
                    );
                }
                CallResult::Ok(json!(agent.credential))
            }
            CapabilityRequest::HttpFetch(f) => {
                assert_eq!(f.method, HttpMethod::Get, "read-only");
                let auth = f.auth.as_ref().expect("every request names the credential");
                assert_eq!(auth.credential, api::CREDENTIAL_SLOT);
                assert_eq!(Some(auth.scheme), agent.scheme);
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
    assert_eq!(slot["key"], api::CREDENTIAL_SLOT);
    // The credential goes to the site of `siteUrl` only: no fixed host.
    assert_eq!(slot["hosts"], json!([]));
    assert_eq!(slot["userOrigin"], api::SITE_URL);
    assert_eq!(slot["schemes"], json!(["basic", "bearer"]));
    let fields = manifest["contributes"]["settings"]["fields"]
        .as_array()
        .unwrap();
    let settings: Vec<&str> = fields.iter().map(|f| f["key"].as_str().unwrap()).collect();
    assert_eq!(settings, [api::SITE_URL, api::DEPLOYMENT, api::TIMEOUT]);
    assert!(fields[0].get("default").is_none(), "Jira has no fixed host");
    // GET to the entered site, the credential there, pages there: nothing
    // else, and no host of its own.
    assert_eq!(
        manifest["permissions"],
        json!([
            {"kind": "network", "hosts": [], "methods": ["GET"],
             "userOrigins": [{"setting": "siteUrl", "pathPrefix": true, "privateNetwork": true}],
             "justification": manifest["permissions"][0]["justification"]},
            {"kind": "credentials", "refs": ["credential"],
             "justification": manifest["permissions"][1]["justification"]},
            {"kind": "openUrl", "hosts": [], "userOrigins": ["siteUrl"],
             "justification": manifest["permissions"][2]["justification"]},
        ])
    );
}

#[test]
fn the_signed_in_user_on_cloud_and_on_data_center() {
    let agent = cloud();
    route(
        &agent,
        &format!("{CLOUD}/rest/api/3/myself"),
        answer(
            200,
            &json!({"displayName": "Dev\u{202e} Eloper", "accountId": "5b10ac8d",
                    "timeZone": "Europe/Warsaw"}),
        ),
    );
    let out = run(&agent, "myself", json!({}));
    let (message, data) = ok(&out);
    assert_eq!(message, "signed in to Jira as Dev  Eloper");
    assert_eq!(data["displayName"], s("Dev  Eloper"));
    assert_eq!(data["account"], s("5b10ac8d"));
    assert_eq!(data["timeZone"], s("Europe/Warsaw"));

    // Data Center: API 2 under the site's path, a bearer token, a user
    // name instead of an account id.
    let agent = data_center();
    route(
        &agent,
        &format!("{DC}/rest/api/2/myself"),
        answer(200, &json!({"displayName": "Dev", "name": "dev"})),
    );
    let out = run(&agent, "myself", json!({}));
    let data = ok(&out).1;
    assert_eq!(data["account"], s("dev"));
    assert!(!data.contains_key("timeZone"));
    assert_eq!(agent.borrow().fetched, [format!("{DC}/rest/api/2/myself")]);
}

#[test]
fn a_missing_site_or_credential_is_refused_before_any_request() {
    for (value, reason) in [
        (json!({}), "is not set"),
        (json!({"siteUrl": "  "}), "is not set"),
        (
            json!({"siteUrl": "http://jira.example.com"}),
            "must be an https address",
        ),
        (
            json!({"siteUrl": "https://u:p@jira.example.com"}),
            "must be an https address",
        ),
        (
            json!({"siteUrl": "https://jira.example.com/?x=1"}),
            "must be an https address",
        ),
    ] {
        let agent = cloud();
        agent.borrow_mut().settings = value.clone();
        let out = run(&agent, "myself", json!({}));
        let (code, message) = failed(&out);
        assert_eq!(code, "invalid-site-url", "{value}");
        assert!(
            message.starts_with(&format!("the Jira site {reason}")),
            "{message}"
        );
        assert!(
            message.ends_with("set it in Settings › Plugins › Jira"),
            "{message}"
        );
        // Open checks the setting too, and opens nothing.
        let out = run(&agent, "open", json!({"issue": "ABC-1"}));
        assert_eq!(failed(&out).0, "invalid-site-url", "{value}");
        assert!(agent.borrow().fetched.is_empty() && agent.borrow().opened.is_empty());
    }
    for (agent, wanted) in [
        (cloud(), "email:api-token"),
        (data_center(), "a personal access token"),
    ] {
        agent.borrow_mut().credential = false;
        let out = run(&agent, "issue", json!({"issue": "ABC-1"}));
        let (code, message) = failed(&out);
        assert_eq!(code, "credential-missing");
        assert!(message.contains(wanted), "{message}");
        assert!(agent.borrow().fetched.is_empty());
    }
}

#[test]
fn a_site_that_is_not_allowed_says_where_to_allow_it() {
    let agent = data_center();
    agent.borrow_mut().not_allowed = true;
    for action in ["myself", "issues"] {
        let out = run(&agent, action, json!({}));
        let (code, message) = failed(&out);
        assert_eq!(code, "not-permitted");
        assert_eq!(
            message,
            "the Jira site jira.example.com is not allowed for this plugin; allow it in \
             Settings › Plugins › Jira › Configure"
        );
    }
    assert!(agent.borrow().fetched.is_empty());
}

#[test]
fn an_issue() {
    let agent = cloud();
    let url = format!(
        "{CLOUD}/rest/api/3/issue/WEB-12?fields=summary%2Cstatus%2Cissuetype%2Cpriority%2Cassignee%2Cupdated"
    );
    route(
        &agent,
        &url,
        answer(
            200,
            &json!({
                "key": "WEB-12",
                "fields": {
                    "summary": "Fix\tthe\u{202e} login\npage",
                    "status": {"name": "In Progress", "statusCategory": {"key": "indeterminate"}},
                    "issuetype": {"name": "Bug"},
                    "priority": {"name": "High"},
                    "assignee": {"displayName": "Dev Eloper"},
                    "updated": "2026-11-15T08:00:00.000+0100"
                }
            }),
        ),
    );
    let out = run(&agent, "issue", json!({"issue": " web-12 "}));
    let (message, data) = ok(&out);
    assert_eq!(message, "WEB-12: In Progress");
    assert_eq!(data["key"], s("WEB-12"));
    assert_eq!(data["summary"], s("Fix the  login page"));
    assert_eq!(data["statusCategory"], s("in-progress"));
    assert_eq!(data["type"], s("Bug"));
    assert_eq!(data["priority"], s("High"));
    assert_eq!(data["assignee"], s("Dev Eloper"));
    assert_eq!(data["url"], s("https://team.atlassian.net/browse/WEB-12"));
    assert_eq!(agent.borrow().fetched, [url]);

    // Hardly any fields, on a Data Center under a path.
    let agent = data_center();
    route(
        &agent,
        &format!("{DC}/rest/api/2/issue/OPS-1"),
        answer(200, &json!({"key": "OPS-1", "fields": {"assignee": null}})),
    );
    let out = run(&agent, "issue", json!({"issue": "OPS-1"}));
    let (message, data) = ok(&out);
    assert_eq!(message, "OPS-1: ");
    assert_eq!(data["statusCategory"], s("unknown"));
    assert_eq!(data["url"], s("https://jira.example.com/jira/browse/OPS-1"));
    for key in ["type", "priority", "assignee", "updated"] {
        assert!(!data.contains_key(key), "{key}");
    }
    for bad in ["", "ABC", "ABC-", "1A-2", "ABC-1/../x"] {
        let out = run(&agent, "issue", json!({ "issue": bad }));
        assert_eq!(failed(&out).0, "invalid-issue-key", "{bad}");
    }
}

fn found(from: usize, count: usize) -> Vec<Value> {
    (from..from + count)
        .map(|n| {
            json!({
                "key": format!("WEB-{n}"),
                "fields": {
                    "summary": format!("Task {n}"),
                    "status": {"name": if n % 2 == 0 { "To Do" } else { "In Review" }},
                    "assignee": if n % 3 == 0 { Value::Null } else { json!({"displayName": "Dev"}) }
                }
            })
        })
        .collect()
}

/// The search of the issues assigned to you, as the plugin asks for it.
fn search(base: &str, jql: &str, size: usize, fields: &str) -> String {
    let query = form(&[
        ("jql", jql),
        ("maxResults", &size.to_string()),
        ("fields", fields),
    ]);
    format!("{base}?{query}")
}

fn form(pairs: &[(&str, &str)]) -> String {
    let encode = |text: &str| {
        text.bytes()
            .map(|b| match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                    char::from(b).to_string()
                }
                b' ' => "+".to_owned(),
                other => format!("%{other:02X}"),
            })
            .collect::<String>()
    };
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

const MINE: &str = "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC";

#[test]
fn issues_assigned_to_you_on_cloud_are_counted_in_pages_of_keys() {
    let agent = cloud();
    let base = format!("{CLOUD}/rest/api/3/search/jql");
    // One page, the last: the list is the count.
    route(
        &agent,
        &search(&base, MINE, 20, "summary,status,assignee"),
        answer(200, &json!({"issues": found(1, 3), "isLast": true})),
    );
    let out = run(&agent, "issues", json!({}));
    let (message, data) = ok(&out);
    assert_eq!(message, "3 issues assigned to you");
    assert_eq!(data["count"], ParamValue::Integer(3));
    assert_eq!(data["countExact"], ParamValue::Bool(true));
    assert_eq!(data["listed"], ParamValue::Integer(3));
    assert_eq!(data["truncated"], ParamValue::Bool(false));
    assert_eq!(data["firstKey"], s("WEB-1"));
    assert_eq!(
        data["firstUrl"],
        s("https://team.atlassian.net/browse/WEB-1")
    );
    assert_eq!(
        data["items"],
        s(
            "WEB-1 Task 1 [In Review] (@Dev)\nWEB-2 Task 2 [To Do] (@Dev)\nWEB-3 Task 3 [In Review] (unassigned)"
        )
    );
    assert_eq!(agent.borrow().fetched.len(), 1);

    // More than the page: counted by keys, 100 at a time.
    let agent = cloud();
    route(
        &agent,
        &search(&base, MINE, 5, "summary,status,assignee"),
        answer(200, &json!({"issues": found(1, 5), "nextPageToken": "t1"})),
    );
    let keys = search(&base, MINE, 100, "key");
    route(
        &agent,
        &keys,
        answer(
            200,
            &json!({"issues": found(1, 100), "nextPageToken": "k2"}),
        ),
    );
    route(
        &agent,
        &format!("{keys}&nextPageToken=k2"),
        answer(200, &json!({"issues": found(101, 30)})),
    );
    let out = run(&agent, "issues", json!({"limit": 5}));
    let (message, data) = ok(&out);
    assert_eq!(message, "130 issues assigned to you");
    assert_eq!(data["listed"], ParamValue::Integer(5));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
    assert_eq!(
        agent.borrow().fetched[1..],
        [keys.clone(), format!("{keys}&nextPageToken=k2")]
    );

    // More than five pages of keys: "at least".
    let agent = cloud();
    route(
        &agent,
        &search(&base, MINE, 20, "summary,status,assignee"),
        answer(200, &json!({"issues": found(1, 20), "nextPageToken": "t1"})),
    );
    route(
        &agent,
        &keys,
        answer(
            200,
            &json!({"issues": found(1, 100), "nextPageToken": "more"}),
        ),
    );
    let out = run(&agent, "issues", json!({}));
    let (message, data) = ok(&out);
    assert_eq!(message, "at least 500 issues assigned to you");
    assert_eq!(data["countExact"], ParamValue::Bool(false));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
    assert_eq!(agent.borrow().fetched.len(), 1 + 5);
}

#[test]
fn a_saved_filter_on_data_center_has_a_total() {
    let agent = data_center();
    let jql = "filter = 12345 AND project = \"WEB\" ORDER BY updated DESC";
    let url = search(
        &format!("{DC}/rest/api/2/search"),
        jql,
        2,
        "summary,status,assignee",
    );
    route(
        &agent,
        &url,
        answer(200, &json!({"issues": found(1, 2), "total": 41})),
    );
    let out = run(
        &agent,
        "issues",
        json!({"source": "filter", "filterId": 12345, "project": "web",
               "includeDone": true, "limit": 2}),
    );
    let (message, data) = ok(&out);
    assert_eq!(message, "41 issues in filter 12345 in WEB");
    assert_eq!(data["count"], ParamValue::Integer(41));
    assert_eq!(data["listed"], ParamValue::Integer(2));
    assert_eq!(data["truncated"], ParamValue::Bool(true));
    assert_eq!(
        data["firstUrl"],
        s("https://jira.example.com/jira/browse/WEB-1")
    );
    assert_eq!(agent.borrow().fetched, [url]);
    // A filter needs its number; a project key is a key, never JQL.
    let out = run(&agent, "issues", json!({"source": "filter"}));
    let (code, message) = failed(&out);
    assert_eq!(code, "invalid-parameters");
    assert_eq!(message, "choose the saved filter's number");
    for bad in ["A\" OR 1=1", "A B", "1A", "A-B"] {
        let out = run(&agent, "issues", json!({ "project": bad }));
        assert_eq!(failed(&out).0, "invalid-project", "{bad}");
    }
    assert_eq!(agent.borrow().fetched.len(), 1);
    // A Data Center answer without a total is not what was expected.
    route(
        &agent,
        &format!("{DC}/rest/api/2/search"),
        answer(200, &json!({"issues": []})),
    );
    let out = run(&agent, "issues", json!({}));
    assert_eq!(failed(&out).0, "invalid-response");
}

#[test]
fn failures_have_the_builtins_codes() {
    let agent = cloud();
    let cases = [
        (
            "A-401",
            answer(401, &json!({"message": "Client must be authenticated"})),
            "unauthorized",
        ),
        (
            "A-403",
            answer(
                403,
                &json!({"errorMessages": ["You do not have permission"]}),
            ),
            "forbidden",
        ),
        (
            "A-404",
            answer(404, &json!({"errorMessages": ["Issue does not exist"]})),
            "not-found",
        ),
        (
            "A-400",
            answer(
                400,
                &json!({"errorMessages": [], "errors": {"jql": "The filter is gone"}}),
            ),
            "jira-error",
        ),
        (
            "A-429",
            answer(429, &json!({})).header("retry-after", "30"),
            "rate-limited",
        ),
        (
            "A-430",
            answer(429, &json!({})).header("retry-after", "Mon, 16 Nov 2026 10:00:45 GMT"),
            "rate-limited",
        ),
        ("A-431", answer(429, &json!({})), "rate-limited"),
        (
            "A-200",
            Answer {
                status: 200,
                headers: Vec::new(),
                body: "<html>".into(),
                too_large: false,
            },
            "invalid-response",
        ),
        (
            "A-201",
            answer(200, &json!({"id": "1"})),
            "invalid-response",
        ),
        (
            "A-202",
            Answer {
                too_large: true,
                ..answer(200, &json!({}))
            },
            "invalid-response",
        ),
    ];
    for (issue, reply, code) in cases {
        route(&agent, &format!("{CLOUD}/rest/api/3/issue/{issue}?"), reply);
        let out = run(&agent, "issue", json!({ "issue": issue }));
        assert_eq!(failed(&out).0, code, "{issue}");
    }
    let message = |issue: &str| {
        let out = run(&agent, "issue", json!({ "issue": issue }));
        failed(&out).1.to_owned()
    };
    assert_eq!(
        message("A-403"),
        "the account may not read the issue A-403 (403): You do not have permission"
    );
    assert_eq!(
        message("A-404"),
        "the issue A-404 was not found, or the account cannot see it (404)"
    );
    assert_eq!(
        message("A-400"),
        "Jira answered 400 for the issue A-400: The filter is gone"
    );
    assert_eq!(
        message("A-429"),
        "Jira's rate limit is reached; try again in 30 s"
    );
    // An HTTP date is counted from Jira's Date (10:00:00).
    assert_eq!(
        message("A-430"),
        "Jira's rate limit is reached; try again in 45 s"
    );
    assert_eq!(
        message("A-431"),
        "Jira's rate limit is reached; try again in 60 s"
    );
    assert_eq!(
        message("A-202"),
        "Jira's answer for the issue A-202 is larger than 512 KiB"
    );
    let out = run(&agent, "issue", json!({"issue": "A-429"}));
    assert!(matches!(
        out,
        ActionOutcome::Failed {
            retryable: true,
            ..
        }
    ));
    // No answer, and no network kind (an older agent, or no route): the
    // agent's reason, retryable.
    let out = run(&agent, "issue", json!({"issue": "A-999"}));
    assert_eq!(failed(&out).0, "request-failed");
}

/// A request the network fails reports the built-in's transport code
/// (`deckpilot-net`'s), with its retryability, through the agent's
/// network kind (ABI 1 `dp_features`).
#[test]
fn network_failures_report_the_builtins_codes() {
    use deckpilot_plugin_sdk::NetworkErrorKind as K;
    let agent = cloud();
    let url = format!("{CLOUD}/rest/api/3/myself");
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
        let out = run(&agent, "myself", json!({}));
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
    agent.borrow_mut().unreachable = BTreeMap::from([(url, K::Other)]);
    let out = run(&agent, "myself", json!({}));
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
fn the_time_limit_is_counted_with_jiras_date() {
    let agent = cloud();
    agent.borrow_mut().settings = json!({"siteUrl": CLOUD, "timeoutSeconds": 5});
    let base = format!("{CLOUD}/rest/api/3/search/jql");
    route(
        &agent,
        &search(&base, MINE, 20, "summary,status,assignee"),
        answer(200, &json!({"issues": found(1, 20), "nextPageToken": "t1"})),
    );
    let mut late = answer(
        200,
        &json!({"issues": found(1, 100), "nextPageToken": "k2"}),
    );
    late.headers = vec![("date".into(), "Mon, 16 Nov 2026 10:00:06 GMT".into())];
    route(&agent, &search(&base, MINE, 100, "key"), late);
    let out = run(&agent, "issues", json!({}));
    let (code, message) = failed(&out);
    assert_eq!(code, "timed-out");
    assert_eq!(message, "Jira did not answer within the time limit");
    assert_eq!(agent.borrow().fetched.len(), 2, "no third request");
}

#[test]
fn only_pages_of_the_site_are_opened() {
    let agent = cloud();
    let out = run(&agent, "open", json!({"issue": " web-12 "}));
    let (message, data) = ok(&out);
    assert_eq!(message, "opened https://team.atlassian.net/browse/WEB-12");
    assert_eq!(data["url"], s("https://team.atlassian.net/browse/WEB-12"));
    let out = run(
        &agent,
        "open",
        json!({"url": "https://Team.Atlassian.net/issues/?filter=12345#top"}),
    );
    assert_eq!(ok(&out).1["url"], s("https://team.atlassian.net/issues/"));
    assert_eq!(
        agent.borrow().opened,
        [
            "https://team.atlassian.net/browse/WEB-12",
            "https://team.atlassian.net/issues/?filter=12345#top"
        ]
    );
    for url in [
        "http://team.atlassian.net/browse/WEB-1",
        "https://other.atlassian.net/browse/WEB-1",
        "https://user@team.atlassian.net/browse/WEB-1",
        "https://team.atlassian.net.evil.example.net/",
    ] {
        let out = run(&agent, "open", json!({ "url": url }));
        let (code, message) = failed(&out);
        assert_eq!(code, "url-not-allowed", "{url}");
        assert_eq!(
            message,
            "only https pages on team.atlassian.net can be opened"
        );
    }
    let out = run(&agent, "open", json!({}));
    let (code, message) = failed(&out);
    assert_eq!(code, "invalid-issue-key");
    assert_eq!(message, "choose a page or an issue to open");
    assert_eq!(
        failed(&run(&agent, "open", json!({"issue": "nope"}))).0,
        "invalid-issue-key"
    );
    assert_eq!(agent.borrow().opened.len(), 2);
    assert!(agent.borrow().fetched.is_empty(), "open sends no request");

    // A Data Center under a path: its pages, and only under it.
    let agent = data_center();
    let out = run(&agent, "open", json!({"issue": "OPS-7"}));
    assert_eq!(
        ok(&out).1["url"],
        s("https://jira.example.com/jira/browse/OPS-7")
    );
    for url in [
        "https://jira.example.com/browse/OPS-7",
        "https://jira.example.com/jira/../admin",
        "https://jira.example.com:8443/jira/browse/OPS-7",
    ] {
        let out = run(&agent, "open", json!({ "url": url }));
        assert_eq!(failed(&out).0, "url-not-allowed", "{url}");
    }
    assert_eq!(agent.borrow().opened.len(), 1);
}

#[test]
fn a_long_list_stays_within_one_output() {
    let agent = cloud();
    let long: Vec<Value> = (1..=100)
        .map(|n| json!({"key": format!("WEB-{n}"), "fields": {"summary": "x".repeat(300)}}))
        .collect();
    route(
        &agent,
        &format!("{CLOUD}/rest/api/3/search/jql"),
        answer(200, &json!({"issues": long, "isLast": true})),
    );
    let out = run(&agent, "issues", json!({"limit": 100}));
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
    let after = response(&[("Retry-After", "30")]);
    assert_eq!(api::rate_limit_wait(429, &after, None), Some(30));
    assert_eq!(api::rate_limit_wait(429, &response(&[]), None), Some(60));
    let date = response(&[("Retry-After", "Thu, 12 Nov 2026 07:00:45 GMT")]);
    assert_eq!(
        api::rate_limit_wait(429, &date, Some(1_794_466_800)),
        Some(45)
    );
    // A date without the answer's Date to count from, or a form not
    // understood: a minute.
    assert_eq!(api::rate_limit_wait(429, &date, None), Some(60));
    let later = response(&[("Retry-After", "later")]);
    assert_eq!(api::rate_limit_wait(429, &later, Some(0)), Some(60));
    assert_eq!(api::rate_limit_wait(403, &after, Some(0)), None);
}
