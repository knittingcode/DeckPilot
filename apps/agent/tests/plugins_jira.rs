//! `deckpilot.jira`, the first-party plugin that replaced the removed
//! built-in `jira` module (`docs/marketplace/module-migration.md` §9,
//! P11), as a regression test of every action on known Jira answers
//! (feature `plugins`):
//!
//! * the plugin (`plugins/first-party/jira`) is built to wasm with the
//!   `guest` profile and bundled as an installed application ships it;
//!   the real agent seeds it, the user consents to its permissions, enters
//!   the site, allows exactly that server and stores the credential, and
//!   every action runs in the real sandboxed host, invoked over IPC like
//!   the configuration app does;
//! * its HTTP goes to canned answers (`DECKPILOT_TEST_PLUGIN_HTTP`, feature
//!   `test-hooks`) built from one function ([`answer`]) for exactly the
//!   requests the scenarios make (the searches' templates and pages, as
//!   the built-in sent them), and its opened pages to a file
//!   (`DECKPILOT_TEST_PLUGIN_OPEN_URL`): never a real Jira, never a
//!   browser. A request beyond those has no answer, and fails the test;
//! * every scenario's result is checked: status, error code, and on
//!   success the message and the key outputs. The plugin was proven to
//!   answer exactly as the built-in did, with the same requests, before
//!   the built-in was removed (§11 D2); these expectations are those
//!   answers. Its action definitions are checked against the built-in's
//!   frozen 0.9.0 contract (`crates/deckpilot-modules/legacy/jira.json`),
//!   as the cut-over's aliases need;
//! * twice: Jira Cloud (REST API 3, Basic, token-paged search) and a Data
//!   Center under a path (REST API 2, Bearer, totals);
//! * Jira has no fixed host: nothing is sent, and no page opens, before
//!   the user allowed the server, after the site changed or after the
//!   consent was revoked;
//! * network failures (a name that does not resolve, a certificate that
//!   is not trusted) end in the transport's code and retryability.
//!
//! The plugin is pinned here as a `default`-class plugin and invoked by its
//! own action types: `migration`-class plugins are installed only by the
//! migration engine, and the legacy `jira.*` types reach the plugin only
//! after a computer's migration is cut over.
#![cfg(feature = "plugins")]

#[path = "support/parity.rs"]
mod parity;
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::time::Duration;

use deckpilot_ipc::plugins::{PluginActionOutcome, PluginSecret, PluginState, UserOriginStatus};
use deckpilot_ipc::{Request, Response};
use parity::{Answer, Expect, json_answer, query};
use serde_json::{Value, json};

const PLUGIN: parity::Plugin = parity::Plugin {
    id: "deckpilot.jira",
    dir: "jira",
    package: "deckpilot-jira",
};
const ID: &str = PLUGIN.id;
const GRANTS: &[&str] = &[
    "network:user-origin:siteUrl",
    "credentials:credential",
    "openUrl:user-origin:siteUrl",
];
/// What the user stores for Jira Cloud (`email:api-token`).
const CLOUD_SECRET: &str = "dev@example.com:ATATT-parity-5e1d-not-a-real-token";
/// A Data Center personal access token.
const DC_SECRET: &str = "dc-pat-parity-93ab-not-a-real-token";
/// A Jira Cloud site, and a Data Center under a path: what the user
/// enters.
const CLOUD: &str = "https://team.atlassian.net";
const DC: &str = "https://jira.example.com/jira";
/// What the plugin answers may use of the IPC message limit.
const ANSWER_BUDGET: usize = deckpilot_ipc::framing::MAX_MESSAGE_BYTES / 2;

/// The built-in's frozen 0.9.0 contract.
const CONTRACT: &str = include_str!("../../../crates/deckpilot-modules/legacy/jira.json");

// --------------------------------------------------------- Jira's answers

/// The issues a search finds, by what its JQL asks for.
fn found(jql: &str) -> Result<Vec<Value>, Answer> {
    let count = if jql.contains("filter = 999") {
        return Err(json_answer(
            400,
            &json!({"errorMessages": ["A value with ID '999' does not exist for the field 'filter'."],
                    "errors": {}}),
        ));
    } else if jql.contains("filter = 12345") {
        7
    } else if jql.contains("filter = 777") {
        620
    } else if jql.contains("project = \"WEB\"") {
        130
    } else if jql.contains("assignee = currentUser()") {
        3
    } else {
        0
    };
    Ok((1..=count)
        .map(|i| {
            let summary = if i == 1 {
                "Fix\tthe\u{202e} login\npage".to_owned()
            } else {
                format!("Task number {i}")
            };
            json!({
                "id": (10_000 + i).to_string(),
                "key": format!("WEB-{}", 1000 - i),
                "fields": {
                    "summary": summary,
                    "status": {"name": if i % 2 == 0 { "To Do" } else { "In Review" }},
                    "assignee": if i % 3 == 0 { Value::Null }
                                else { json!({"displayName": format!("Dev {}", i % 3)}) }
                }
            })
        })
        .collect())
}

/// One page of a search. Cloud (`search/jql`): `maxResults` issues from
/// the offset in `nextPageToken`, and the next token while there are
/// more. Data Center (`search`): the first `maxResults` and the total.
fn search(target: &str, cloud: bool) -> Answer {
    let query = query(target);
    let issues = match found(query.get("jql").map_or("", String::as_str)) {
        Ok(issues) => issues,
        Err(refused) => return refused,
    };
    let size: usize = query.get("maxResults").map_or(50, |v| v.parse().unwrap());
    let keys_only = query.get("fields").map(String::as_str) == Some("key");
    let offset: usize = query
        .get("nextPageToken")
        .map_or(0, |t| t.trim_start_matches('o').parse().unwrap());
    let end = (offset + size).min(issues.len());
    let page: Vec<Value> = issues[offset.min(end)..end]
        .iter()
        .map(|issue| {
            if keys_only {
                json!({"id": issue["id"], "key": issue["key"]})
            } else {
                issue.clone()
            }
        })
        .collect();
    if !cloud {
        return json_answer(
            200,
            &json!({"startAt": 0, "maxResults": size, "total": issues.len(), "issues": page}),
        );
    }
    let mut body = json!({"issues": page, "isLast": end == issues.len()});
    if end < issues.len() {
        body["nextPageToken"] = json!(format!("o{end}"));
    }
    json_answer(200, &body)
}

fn issue(key: &str) -> Answer {
    match key {
        "WEB-12" => json_answer(
            200,
            &json!({
                "key": "WEB-12",
                "fields": {
                    "summary": "Fix\tthe\u{202e} login\npage",
                    "status": {"name": "In Progress", "statusCategory": {"key": "indeterminate"}},
                    "issuetype": {"name": "Bug"},
                    "priority": {"name": "High"},
                    "assignee": {"displayName": "Dev\u{7} Eloper"},
                    "updated": "2026-11-15T08:00:00.000+0100"
                }
            }),
        ),
        // Hardly any fields: the fallbacks.
        "OPS-1" => json_answer(
            200,
            &json!({"key": "OPS-1", "fields": {"status": {"name": "Done",
                    "statusCategory": {"key": "done"}}, "assignee": null, "priority": null}}),
        ),
        "WEB-401" => json_answer(401, &json!({"message": "Client must be authenticated"})),
        "WEB-403" => json_answer(
            403,
            &json!({"errorMessages": ["You do not have permission to see this issue."]}),
        ),
        "WEB-500" => json_answer(500, &json!({"message": "Internal\nServer Error"})),
        "WEB-429" => json_answer(429, &json!({})).header("retry-after", "30"),
        "WEB-200" => parity::html_answer(),
        "WEB-201" => json_answer(200, &json!({"id": "10001"})),
        _ => json_answer(
            404,
            &json!({"errorMessages": ["Issue does not exist or you do not have permission to see it."]}),
        ),
    }
}

/// Jira's answer to `GET <site><target>`, where `target` is the path below
/// the site with the query: the plugin's canned answers come from here.
/// REST API 3 answers as Jira Cloud, 2 as a Data Center.
fn answer(_site: &str, target: &str) -> Answer {
    let path = target.split('?').next().unwrap();
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segments.as_slice() {
        ["rest", "api", "3", "myself"] => json_answer(
            200,
            &json!({"displayName": "Dev\nEloper", "accountId": "5b10ac8d82e05b22cc7d4ef5",
                    "timeZone": "Europe/Warsaw"}),
        ),
        ["rest", "api", "2", "myself"] => {
            json_answer(200, &json!({"displayName": "Dev Eloper", "name": "dev"}))
        }
        ["rest", "api", "2" | "3", "issue", key] => issue(key),
        ["rest", "api", "3", "search", "jql"] => search(target, true),
        ["rest", "api", "2", "search"] => search(target, false),
        _ => json_answer(404, &json!({"message": "null for uri"})),
    }
}

// ------------------------------------------------------------ scenarios

/// An action, its parameters, a name for messages, and what it ends in.
struct Scenario {
    name: &'static str,
    action: &'static str,
    params: Value,
    expect: Expect,
}

fn scenario(name: &'static str, action: &'static str, params: Value, expect: Expect) -> Scenario {
    Scenario {
        name,
        action,
        params,
        expect,
    }
}

/// Every read action, on success and on each failure Jira can answer.
/// `{site}` in an expectation is the Jira site.
#[allow(clippy::too_many_lines)] // one list
fn readings() -> Vec<Scenario> {
    let issue = |name, key: &str, expect| scenario(name, "issue", json!({ "issue": key }), expect);
    let failed = Expect::Failed;
    vec![
        scenario(
            "myself",
            "myself",
            json!({}),
            Expect::Ok(
                "signed in to Jira as Dev Eloper",
                &[
                    ("displayName", "Dev Eloper"),
                    ("account", "5b10ac8d82e05b22cc7d4ef5"),
                    ("timeZone", "Europe/Warsaw"),
                ],
            ),
        ),
        issue(
            "issue WEB-12",
            "web-12",
            Expect::Ok(
                "WEB-12: In Progress",
                &[
                    ("key", "WEB-12"),
                    ("summary", "Fix the  login page"),
                    ("status", "In Progress"),
                    ("statusCategory", "in-progress"),
                    ("type", "Bug"),
                    ("priority", "High"),
                    ("assignee", "Dev  Eloper"),
                    ("updated", "2026-11-15T08:00:00.000+0100"),
                    ("url", "{site}/browse/WEB-12"),
                ],
            ),
        ),
        issue(
            "issue OPS-1 (fallbacks)",
            " OPS-1 ",
            Expect::Ok(
                "OPS-1: Done",
                &[
                    ("key", "OPS-1"),
                    ("summary", ""),
                    ("status", "Done"),
                    ("statusCategory", "done"),
                    ("url", "{site}/browse/OPS-1"),
                ],
            ),
        ),
        issue("issue not a key", "WEB", failed("invalid-issue-key")),
        issue("issue WEB-404", "WEB-404", failed("not-found")),
        issue("issue WEB-403", "WEB-403", failed("forbidden")),
        issue("issue WEB-500", "WEB-500", failed("jira-error")),
        issue("issue WEB-429", "WEB-429", failed("rate-limited")),
        issue("issue WEB-401", "WEB-401", failed("unauthorized")),
        issue(
            "issue WEB-200 (not JSON)",
            "WEB-200",
            failed("invalid-response"),
        ),
        issue(
            "issue WEB-201 (another shape)",
            "WEB-201",
            failed("invalid-response"),
        ),
        scenario(
            "issues assigned to you",
            "issues",
            json!({}),
            Expect::Ok(
                "3 issues assigned to you",
                &[
                    ("count", "3"),
                    ("countExact", "true"),
                    ("listed", "3"),
                    ("truncated", "false"),
                    ("firstKey", "WEB-999"),
                    ("firstUrl", "{site}/browse/WEB-999"),
                    (
                        "items",
                        "WEB-999 Fix the  login page [In Review] (@Dev 1)\n\
                         WEB-998 Task number 2 [To Do] (@Dev 2)\n\
                         WEB-997 Task number 3 [In Review] (unassigned)",
                    ),
                ],
            ),
        ),
        scenario(
            "issues of a project, 5 listed",
            "issues",
            json!({"project": "web", "limit": 5}),
            Expect::Ok(
                "130 issues assigned to you in WEB",
                &[
                    ("count", "130"),
                    ("countExact", "true"),
                    ("listed", "5"),
                    ("truncated", "true"),
                    ("firstKey", "WEB-999"),
                    ("firstUrl", "{site}/browse/WEB-999"),
                ],
            ),
        ),
        scenario(
            "issues of a project, 100 listed",
            "issues",
            json!({"project": "WEB", "limit": 100, "includeDone": true}),
            Expect::Ok(
                "130 issues assigned to you in WEB",
                &[
                    ("count", "130"),
                    ("countExact", "true"),
                    ("listed", "100"),
                    ("truncated", "true"),
                    ("firstKey", "WEB-999"),
                    ("firstUrl", "{site}/browse/WEB-999"),
                ],
            ),
        ),
        scenario(
            "issues of a saved filter",
            "issues",
            json!({"source": "filter", "filterId": 12345, "includeDone": true}),
            Expect::Ok(
                "7 issues in filter 12345",
                &[
                    ("count", "7"),
                    ("countExact", "true"),
                    ("listed", "7"),
                    ("truncated", "false"),
                    ("firstKey", "WEB-999"),
                    ("firstUrl", "{site}/browse/WEB-999"),
                ],
            ),
        ),
        scenario(
            "issues of a filter with more than are counted",
            "issues",
            json!({"source": "filter", "filterId": 777}),
            Expect::Ok(
                "at least 500 issues in filter 777",
                &[
                    ("count", "500"),
                    ("countExact", "false"),
                    ("listed", "20"),
                    ("truncated", "true"),
                    ("firstKey", "WEB-999"),
                    ("firstUrl", "{site}/browse/WEB-999"),
                ],
            ),
        ),
        scenario(
            "issues of a filter that does not exist",
            "issues",
            json!({"source": "filter", "filterId": 999}),
            failed("jira-error"),
        ),
        scenario(
            "issues of a filter without its number",
            "issues",
            json!({"source": "filter"}),
            failed("invalid-parameters"),
        ),
        scenario(
            "issues of a project that is no key",
            "issues",
            json!({"project": "A\" OR 1=1"}),
            failed("invalid-project"),
        ),
    ]
}

/// Issues whose request gets no answer, and the transport's code (a name
/// that never resolves, a certificate that is not trusted), canned.
const NETWORK_FAILURES: &[(&str, &str)] = &[("NET-1", "dns-failed"), ("NET-2", "tls-failed")];

/// The issues the scenarios read.
const ISSUES: &[&str] = &[
    "WEB-12", "OPS-1", "WEB-404", "WEB-403", "WEB-500", "WEB-429", "WEB-401", "WEB-200", "WEB-201",
];

/// The searches the scenarios make: saved filter, project, done issues
/// included, issues listed.
const SEARCHES: &[(Option<i64>, Option<&str>, bool, usize)] = &[
    (None, None, false, 20),
    (None, Some("WEB"), false, 5),
    (None, Some("WEB"), true, 100),
    (Some(12345), None, true, 20),
    (Some(777), None, false, 20),
    (Some(999), None, false, 20),
];

/// The fields read for a list.
const LIST_FIELDS: &str = "summary,status,assignee";
/// Cloud counts by pages of keys: how many, and how large.
const COUNT_PAGES: usize = 5;
const COUNT_PAGE_SIZE: usize = 100;

/// The search's JQL, as Jira's search templates make it.
fn jql(filter: Option<i64>, project: Option<&str>, include_done: bool) -> String {
    let mut jql = filter.map_or_else(
        || "assignee = currentUser()".to_owned(),
        |id| format!("filter = {id}"),
    );
    if let Some(project) = project {
        let _ = write!(jql, " AND project = \"{project}\"");
    }
    if !include_done {
        jql.push_str(" AND statusCategory != Done");
    }
    jql.push_str(" ORDER BY updated DESC");
    jql
}

fn search_target(cloud: bool, jql: &str, size: usize, fields: &str, token: Option<&str>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("jql", jql),
        ("maxResults", &size.to_string()),
        ("fields", fields),
    ]);
    if let Some(token) = token {
        query.append_pair("nextPageToken", token);
    }
    let path = if cloud {
        "/rest/api/3/search/jql"
    } else {
        "/rest/api/2/search"
    };
    format!("{path}?{}", query.finish())
}

fn next_token(answer: &Answer) -> Option<String> {
    let body: Value = serde_json::from_str(&answer.body).ok()?;
    body["nextPageToken"].as_str().map(str::to_owned)
}

/// The canned answers of the plugin's HTTP on `site`: [`answer`] for every
/// request of the scenarios (on Cloud, a search with more issues than
/// listed is counted by pages of keys). Longest first: the canned
/// transport takes the first route whose URL starts the request's.
fn routes(site: &str, cloud: bool) -> Vec<Value> {
    let api = if cloud { "/rest/api/3" } else { "/rest/api/2" };
    let mut targets = vec![format!("{api}/myself")];
    // An issue's answer does not depend on its query.
    targets.extend(ISSUES.iter().map(|key| format!("{api}/issue/{key}?")));
    for (filter, project, include_done, size) in SEARCHES {
        let jql = jql(*filter, *project, *include_done);
        let first = search_target(cloud, &jql, *size, LIST_FIELDS, None);
        let more = cloud && next_token(&answer(site, &first)).is_some();
        targets.push(first);
        if more {
            let mut token = None;
            for _ in 0..COUNT_PAGES {
                let target = search_target(cloud, &jql, COUNT_PAGE_SIZE, "key", token.as_deref());
                token = next_token(&answer(site, &target));
                targets.push(target);
                if token.is_none() {
                    break;
                }
            }
        }
    }
    targets.sort_by_key(|t| std::cmp::Reverse(t.len()));
    targets.dedup();
    let mut routes: Vec<Value> = Vec::new();
    if cloud {
        routes.extend(NETWORK_FAILURES.iter().map(|(issue, code)| {
            json!({"url": format!("{site}{api}/issue/{issue}?"), "status": 0, "body": "",
                   "fail": code})
        }));
    }
    routes.extend(targets.iter().map(|target| {
        let a = answer(site, target);
        json!({"url": format!("{site}{target}"), "status": a.status, "headers": a.headers,
               "body": a.body})
    }));
    routes
}

/// The scenarios repeated on the Data Center: what depends on the
/// deployment (REST API 2 under the site's path, a bearer token, a user
/// name, a search with a total).
const ON_DATA_CENTER: &[&str] = &[
    "myself",
    "issue WEB-12",
    "issue OPS-1 (fallbacks)",
    "issue WEB-404",
    "issues assigned to you",
    "issues of a project, 5 listed",
    "issues of a saved filter",
    "issues of a filter with more than are counted",
];

/// What one run covered: scenario, whether it succeeded, its code.
type Covered = Vec<(&'static str, bool, Option<&'static str>)>;

/// The plugin under test.
struct Side<'a> {
    env: &'a parity::Env,
    agent: &'a parity::Agent,
    pace: &'a parity::Pace,
    routes: &'a [Value],
}

impl Side<'_> {
    /// The scenarios (all, or those named in `only`) on the Jira at
    /// `site`; `expect` overrides a scenario's expectation by name.
    async fn check(
        &self,
        site: &str,
        only: Option<&[&str]>,
        expect: &[(&str, Expect)],
        problems: &mut Vec<String>,
    ) -> Covered {
        let vars = [("{site}", site)];
        let mut covered = Vec::new();
        for s in readings() {
            if only.is_some_and(|only| !only.contains(&s.name)) {
                continue;
            }
            let wanted = expect
                .iter()
                .find(|(name, _)| *name == s.name)
                .map_or(s.expect, |(_, e)| *e);
            self.pace.room_for(self.env, 8).await;
            let outcome = self.agent.invoke(s.action, &s.params).await;
            parity::check(s.name, &outcome, wanted, &vars, problems);
            covered.push((s.name, wanted.ok(), wanted.code()));
        }
        covered
    }

    /// Opens pages, one per 2 s (a plugin's limit).
    async fn check_opens(
        &self,
        pages: &[(&'static str, Value, Expect)],
        problems: &mut Vec<String>,
    ) -> Covered {
        let mut covered = Vec::new();
        for (name, params, expect) in pages {
            tokio::time::sleep(Duration::from_millis(2_100)).await;
            let outcome = self.agent.invoke("open", params).await;
            parity::check(name, &outcome, *expect, &[], problems);
            covered.push((*name, expect.ok(), expect.code()));
        }
        covered
    }

    /// What the plugin sent: only to `site`, each with the credential in
    /// the deployment's `scheme`, and only what the scenarios ask for.
    fn sent_only_to(&self, sent: &[String], site: &str, scheme: &str, secret: &str) {
        assert_ne!(sent.len(), 0);
        for line in sent {
            let (url, credential) = line.split_once('\t').unwrap_or((line, ""));
            assert!(url.starts_with(&format!("{site}/rest/api/")), "{line}");
            assert_eq!(
                credential,
                format!("authorization: {scheme} {secret}"),
                "{url}"
            );
        }
        assert_eq!(parity::unrouted(sent, self.routes), Vec::<String>::new());
    }

    /// Enters `site` (and the deployment) in the plugin's settings: saved
    /// normalized, waiting for the user's consent to exactly it.
    async fn enter(&self, site: &str, entered: &str, deployment: &str) {
        let settings = self
            .agent
            .set_settings(BTreeMap::from([
                ("siteUrl".to_owned(), json!(entered)),
                ("deployment".to_owned(), json!(deployment)),
            ]))
            .await;
        let origin = &settings.origins[0];
        assert_eq!(origin.origin.as_deref(), Some(site), "{origin:?}");
        assert_eq!(origin.status, UserOriginStatus::NeedsConsent);
        assert_eq!(origin.credentials, ["credential"]);
        assert!(origin.opens_pages && !origin.private_host, "{origin:?}");
    }

    /// Nothing is sent and no page opens: the server is not allowed.
    async fn refused(&self, when: &str) {
        let sent = self.env.sent();
        let opened = self.env.opened();
        let refused = self.agent.invoke("myself", &json!({})).await;
        assert_eq!(
            refused.code.as_deref(),
            Some("not-permitted"),
            "{when}: {refused:?}"
        );
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        let refused = self.agent.invoke("open", &json!({"issue": "WEB-1"})).await;
        assert_eq!(
            refused.code.as_deref(),
            Some("not-permitted"),
            "{when}: {refused:?}"
        );
        assert_eq!(self.env.sent(), sent, "a request {when}");
        assert_eq!(self.env.opened(), opened, "a page {when}");
    }

    async fn allow(&self, site: &str) {
        self.agent
            .ok(Request::GrantPluginUserOrigin {
                plugin: ID.into(),
                setting: "siteUrl".into(),
                origin: site.into(),
                private_network: false,
            })
            .await;
    }

    async fn store(&self, secret: &str) {
        self.agent
            .ok(Request::SetPluginCredential {
                plugin: ID.into(),
                key: "credential".into(),
                secret: PluginSecret::new(secret),
            })
            .await;
    }
}

// ---------------------------------------------------------------- test

/// Every action of `deckpilot.jira` on known answers, with the results
/// the built-in gave; on Jira Cloud and on a Data Center.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // the steps of one run, in order
async fn the_plugin_answers_as_the_builtin_did() {
    let mut canned = routes(CLOUD, true);
    canned.extend(routes(DC, false));
    let env = parity::Env::new(PLUGIN, &canned);
    let agent = env.start().await;
    let pace = parity::Pace::start();
    let under_test = Side {
        env: &env,
        agent: &agent,
        pace: &pace,
        routes: &canned,
    };

    // Seeded, waiting for the user's consent to its permissions.
    let summary = match agent.request(Request::ListPlugins).await.unwrap() {
        Response::Plugins { plugins } => plugins
            .into_iter()
            .find(|p| p.id == ID)
            .unwrap_or_else(|| panic!("not seeded\n{}", agent.log())),
        other => panic!("unexpected {other:?}"),
    };
    assert_ne!(summary.state, PluginState::Enabled, "{}", agent.log());

    // The definitions as the agent's engine registers them hold §5.3's
    // rules against the built-in's frozen ones.
    assert_eq!(summary.actions.len(), 4);
    let problems = parity::contract_problems("jira", ID, CONTRACT, &summary.actions);
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    agent
        .ok(Request::EnablePlugin {
            plugin: ID.into(),
            consent: GRANTS.iter().map(|g| (*g).to_owned()).collect(),
        })
        .await;
    match agent
        .request(Request::GetPlugin { plugin: ID.into() })
        .await
        .unwrap()
    {
        response @ Response::Plugin { .. } => {
            let size = serde_json::to_vec(&response).unwrap().len();
            assert!(size < ANSWER_BUDGET, "getPlugin answers {size} bytes");
        }
        other => panic!("unexpected {other:?}"),
    }

    // No site yet: nothing is sent; the plugin says where to set it.
    let mut problems = Vec::new();
    let unset = agent.invoke("myself", &json!({})).await;
    parity::check(
        "myself without a site",
        &unset,
        Expect::Failed("invalid-site-url"),
        &[],
        &mut problems,
    );
    assert!(
        unset.message.contains("Settings › Plugins › Jira"),
        "{unset:?}"
    );

    // ---- Jira Cloud. The site entered, not allowed: nothing is sent, no
    // page opens; the plugin says where to allow it.
    under_test
        .enter(CLOUD, "https://Team.Atlassian.net/", "cloud")
        .await;
    under_test.refused("before the consent").await;
    let refused = agent.invoke("myself", &json!({})).await;
    assert_eq!(
        refused.message,
        "the Jira site team.atlassian.net is not allowed for this plugin; allow it in Settings \
         › Plugins › Jira › Configure"
    );
    // Allowed, no credential yet: the plugin names its slot.
    under_test.allow(CLOUD).await;
    let without = agent.invoke("myself", &json!({})).await;
    parity::check(
        "myself without a credential",
        &without,
        Expect::Failed("credential-missing"),
        &[],
        &mut problems,
    );
    assert!(
        without.message.contains("Settings › Plugins › Jira"),
        "{without:?}"
    );
    under_test.store(CLOUD_SECRET).await;
    assert_eq!(env.sent().len(), 0);

    let mut covered = under_test.check(CLOUD, None, &[], &mut problems).await;
    let pages = [
        (
            "open an issue",
            json!({"issue": " web-12 "}),
            Expect::Ok(
                "opened https://team.atlassian.net/browse/WEB-12",
                &[("url", "https://team.atlassian.net/browse/WEB-12")],
            ),
        ),
        (
            "open a page",
            json!({"url": " https://Team.Atlassian.net/issues/?filter=12345#top "}),
            // Shown without its query.
            Expect::Ok(
                "opened https://team.atlassian.net/issues/",
                &[("url", "https://team.atlassian.net/issues/")],
            ),
        ),
        (
            "open another site",
            json!({"url": "https://other.atlassian.net/browse/WEB-12"}),
            Expect::Failed("url-not-allowed"),
        ),
        (
            "open what is no issue",
            json!({"issue": "nope"}),
            Expect::Failed("invalid-issue-key"),
        ),
        (
            "open nothing",
            json!({}),
            Expect::Failed("invalid-issue-key"),
        ),
    ];
    covered.extend(under_test.check_opens(&pages, &mut problems).await);
    let on_cloud = [
        "https://team.atlassian.net/browse/WEB-12",
        "https://team.atlassian.net/issues/?filter=12345#top",
    ];
    assert_eq!(env.opened(), on_cloud);
    assert!(
        problems.is_empty(),
        "{}\n{}",
        problems.join("\n"),
        agent.log()
    );
    // The scenarios cover success and every failure code of the module.
    let codes: BTreeSet<_> = covered.iter().filter_map(|(_, _, c)| *c).collect();
    for code in [
        "invalid-issue-key",
        "invalid-project",
        "unauthorized",
        "forbidden",
        "not-found",
        "rate-limited",
        "jira-error",
        "invalid-response",
        "url-not-allowed",
        "invalid-parameters",
    ] {
        assert!(codes.contains(code), "no scenario ends in {code}");
    }
    assert!(covered.iter().filter(|(_, ok, _)| *ok).count() >= 9);
    under_test.sent_only_to(&env.sent(), CLOUD, "Basic", CLOUD_SECRET);

    pace.room_for(&env, 3).await;
    network_failures(&agent).await;
    let sent_to_cloud = env.sent();

    // ---- a Data Center under a path: another site, another credential.
    // The changed site ends the consent at once.
    under_test.enter(DC, DC, "data-center").await;
    under_test.store(DC_SECRET).await;
    under_test.refused("after the site changed").await;
    under_test.allow(DC).await;
    // A Data Center names the user by their user name, and gives the
    // search's total: no counting by pages.
    let on_data_center = [
        (
            "myself",
            Expect::Ok(
                "signed in to Jira as Dev Eloper",
                &[("displayName", "Dev Eloper"), ("account", "dev")],
            ),
        ),
        (
            "issues of a filter with more than are counted",
            Expect::Ok(
                "620 issues in filter 777",
                &[
                    ("count", "620"),
                    ("countExact", "true"),
                    ("listed", "20"),
                    ("truncated", "true"),
                    ("firstKey", "WEB-999"),
                    ("firstUrl", "{site}/browse/WEB-999"),
                ],
            ),
        ),
    ];
    let on_dc = under_test
        .check(DC, Some(ON_DATA_CENTER), &on_data_center, &mut problems)
        .await;
    assert_eq!(on_dc.len(), ON_DATA_CENTER.len());
    let pages = [
        (
            "open an issue of the Data Center",
            json!({"issue": "OPS-7"}),
            Expect::Ok(
                "opened https://jira.example.com/jira/browse/OPS-7",
                &[("url", "https://jira.example.com/jira/browse/OPS-7")],
            ),
        ),
        (
            "open a page beside the Data Center",
            json!({"url": "https://jira.example.com/confluence/Home"}),
            Expect::Failed("url-not-allowed"),
        ),
        (
            "open the Cloud site from the Data Center",
            json!({"url": "https://team.atlassian.net/browse/WEB-12"}),
            Expect::Failed("url-not-allowed"),
        ),
    ];
    under_test.check_opens(&pages, &mut problems).await;
    assert!(
        problems.is_empty(),
        "{}\n{}",
        problems.join("\n"),
        agent.log()
    );
    assert_eq!(
        env.opened()[on_cloud.len()..],
        ["https://jira.example.com/jira/browse/OPS-7"]
    );
    let sent = env.sent();
    under_test.sent_only_to(&sent[sent_to_cloud.len()..], DC, "Bearer", DC_SECRET);

    // The consent revoked: refused on the very next call.
    agent
        .ok(Request::RevokePluginUserOrigin {
            plugin: ID.into(),
            setting: "siteUrl".into(),
        })
        .await;
    under_test.refused("after the consent was revoked").await;

    // Neither secret is in a log line.
    let log = agent.log();
    for secret in [CLOUD_SECRET, DC_SECRET] {
        assert!(!log.contains(secret), "a secret is in the agent's log");
        let token = secret.rsplit(':').next().unwrap();
        assert!(!log.contains(token), "a token is in the agent's log");
    }
}

/// No answer at all: a name that does not resolve and a certificate that
/// is not trusted end in the transport's codes, with its retryability
/// (`dns-failed` retryable, `tls-failed` not), and a message that does
/// not name the secret.
async fn network_failures(agent: &parity::Agent) {
    for (issue, code) in NETWORK_FAILURES {
        let params = json!({ "issue": issue });
        let outcome = agent.invoke_raw("issue", &params).await;
        eprintln!("network {issue}: {outcome:?}");
        let PluginActionOutcome::Failed {
            code: plugin_code,
            message,
            retryable,
        } = &outcome
        else {
            panic!("{issue}: {outcome:?}\n{}", agent.log());
        };
        assert_eq!(plugin_code, code, "{outcome:?}");
        assert_eq!(*retryable, *code == "dns-failed", "{outcome:?}");
        let message = message.as_deref().unwrap_or_default();
        assert!(!message.contains("ATATT"), "{message}");
        assert_ne!(message, "");
    }
}
