//! `deckpilot.gitlab`, the first-party plugin that replaced the removed
//! built-in `gitlab` module (`docs/marketplace/module-migration.md` §9,
//! P11), as a regression test of every action on known GitLab answers
//! (feature `plugins`):
//!
//! * the plugin (`plugins/first-party/gitlab`) is built to wasm with the
//!   `guest` profile and bundled as an installed application ships it;
//!   the real agent seeds it, the user consents to its permissions and
//!   stores its token, and every action runs in the real sandboxed host,
//!   invoked over IPC like the configuration app does;
//! * its HTTP goes to canned answers (`DECKPILOT_TEST_PLUGIN_HTTP`, feature
//!   `test-hooks`) built from one function ([`answer`]) for the requests
//!   the scenarios make, and its opened pages to a file
//!   (`DECKPILOT_TEST_PLUGIN_OPEN_URL`): never a real GitLab, never a
//!   browser;
//! * every scenario's result is checked: status, error code, and on
//!   success the message and the key outputs. The plugin was proven to
//!   answer exactly as the built-in did before the built-in was removed
//!   (§11 D2); these expectations are those answers. Its action
//!   definitions are checked against the built-in's frozen 0.9.0 contract
//!   (`crates/deckpilot-modules/legacy/gitlab.json`), as the cut-over's
//!   aliases need;
//! * twice: on GitLab.com (the plugin's declared host), and (the scenarios
//!   of [`ON_THE_ENTERED_GITLAB`]) on a self-managed GitLab under a path
//!   that the user enters in the plugin's `apiUrl` setting, which is
//!   reached, sent the token and opened only after the user allowed
//!   exactly that server, and no more once the consent is revoked;
//! * network failures (a name that does not resolve, a certificate that
//!   is not trusted) end in the transport's code and retryability.
//!
//! The plugin is pinned here as a `default`-class plugin and invoked by its
//! own action types: `migration`-class plugins are installed only by the
//! migration engine, and the legacy `gitlab.*` types reach the plugin only
//! after a computer's migration is cut over.
#![cfg(feature = "plugins")]

#[path = "support/parity.rs"]
mod parity;
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use deckpilot_ipc::plugins::{PluginActionOutcome, PluginSecret, PluginState, UserOriginStatus};
use deckpilot_ipc::{Request, Response};
use parity::{Answer, Expect, json_answer, query};
use serde_json::{Value, json};

const PLUGIN: parity::Plugin = parity::Plugin {
    id: "deckpilot.gitlab",
    dir: "gitlab",
    package: "deckpilot-gitlab",
};
const ID: &str = PLUGIN.id;
const GRANTS: &[&str] = &[
    "network:gitlab.com",
    "network:user-origin:apiUrl",
    "credentials:token",
    "openUrl:gitlab.com",
    "openUrl:user-origin:apiUrl",
];
/// The token the user stores.
const TOKEN: &str = "glpat-parity-7c2e9b-not-a-real-token";
/// GitLab.com's API: the plugin's default.
const API: &str = "https://gitlab.com/api/v4";
/// A self-managed GitLab under a path: the server the user enters.
const OWN_API: &str = "https://code.example.com/gitlab/api/v4";
const API_SUFFIX: &str = "/api/v4";
/// What the plugin answers may use of the IPC message limit.
const ANSWER_BUDGET: usize = deckpilot_ipc::framing::MAX_MESSAGE_BYTES / 2;

/// The built-in's frozen 0.9.0 contract.
const CONTRACT: &str = include_str!("../../../crates/deckpilot-modules/legacy/gitlab.json");

// ------------------------------------------------------- GitLab's answers

/// The web address of the GitLab whose API is at `base`.
fn web(base: &str) -> &str {
    base.strip_suffix(API_SUFFIX).unwrap_or(base)
}

/// One page of `items`, GitLab style: `per_page` and `page` from the
/// query, `x-total` (unless the list is one GitLab does not count) and a
/// `link` to the next page (on `base`) while there is one.
fn page(base: &str, target: &str, items: &[Value], counted: bool) -> Answer {
    let query = query(target);
    let per_page: usize = query.get("per_page").map_or(20, |v| v.parse().unwrap());
    let page: usize = query.get("page").map_or(1, |v| v.parse().unwrap());
    let start = ((page - 1) * per_page).min(items.len());
    let end = (page * per_page).min(items.len());
    let mut answer = json_answer(200, &Value::Array(items[start..end].to_vec()));
    if counted {
        answer = answer.header("x-total", &items.len().to_string());
    }
    if end < items.len() {
        let path = target.split('?').next().unwrap();
        let mut next = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in query.iter().filter(|(k, _)| *k != "page") {
            next.append_pair(k, v);
        }
        next.append_pair("page", &(page + 1).to_string());
        let link = format!(
            "<{base}{path}?{}>; rel=\"next\", <{base}{path}?page=99>; rel=\"last\"",
            next.finish()
        );
        answer = answer.header("link", &link);
    }
    answer
}

/// Open merge requests of `project` (or of every project) for a query,
/// and whether GitLab counts the list (`x-total`).
fn merge_requests(
    base: &str,
    project: Option<&str>,
    query: &BTreeMap<String, String>,
) -> (Vec<Value>, bool) {
    let mine = query.get("author_id").map(String::as_str) == Some("7");
    let reviewing = query.get("reviewer_id").map(String::as_str) == Some("7");
    let (count, counted) = match (project, mine, reviewing) {
        (Some("group/app"), false, false) => (73, true),
        (Some("group/app"), false, true) => (2, true),
        (Some("group/mid"), false, false) => (130, false),
        (Some("group/huge"), false, false) => (620, false),
        (None, true, false) => (3, true),
        (None, false, true) => (5, true),
        _ => (0, true),
    };
    let items = (1..=count)
        .map(|i| {
            let project = match project {
                Some(project) => project,
                None if i % 2 == 1 => "group/app",
                None => "group/api",
            };
            let iid = 1000 - i;
            let title = if i == 1 {
                "Fix\tthe\u{202e} build\nnow".to_owned()
            } else {
                format!("Change number {i}")
            };
            json!({
                "iid": iid,
                "title": title,
                "draft": i % 5 == 0,
                "references": {"full": format!("{project}!{iid}")},
                "author": {"username": format!("dev{}", i % 3)},
                "web_url": format!("{}/{project}/-/merge_requests/{iid}", web(base))
            })
        })
        .collect();
    (items, counted)
}

fn jobs(pipeline: &str) -> Vec<Value> {
    let count = match pipeline {
        "77" => 7,
        "501" => 130,
        "900" => 620,
        _ => 0,
    };
    (0..count)
        .map(|i| {
            let (status, allow_failure) = match i % 7 {
                0 => ("running", false),
                1 => ("failed", false),
                2 => ("canceled", false),
                3 => ("failed", true),
                4 => ("skipped", false),
                5 => ("manual", false),
                _ => ("success", false),
            };
            let name = if i == 1 {
                "unit\u{7} tests".to_owned()
            } else {
                format!("job-{i}")
            };
            json!({"name": name, "status": status, "allow_failure": allow_failure})
        })
        .collect()
}

fn project(base: &str, name: &str) -> Answer {
    match name {
        "group/app" | "42" => json_answer(
            200,
            &json!({
                "path_with_namespace": "group/app", "default_branch": "main",
                "visibility": "private", "archived": false, "open_issues_count": 12,
                "star_count": 345, "forks_count": 6,
                "web_url": format!("{}/group/app", web(base)),
                "description": "The\u{202e} web app\nof the group",
                "last_activity_at": "2026-11-15T08:00:00Z"
            }),
        ),
        // No page, no branch, issues turned off: the fallbacks.
        "group/bare" => json_answer(
            200,
            &json!({"path_with_namespace": "group/bare", "archived": true, "description": null}),
        ),
        "group/forbidden" => json_answer(403, &json!({"message": "403 Forbidden"})),
        "group/broken" => json_answer(502, &json!({"error": "Bad Gateway"})),
        "group/limited" => json_answer(429, &json!({"message": "Too Many Requests"}))
            .header("ratelimit-reset", "1000000000"),
        "group/secondary" => json_answer(429, &json!({})).header("retry-after", "30"),
        "group/revoked" => json_answer(401, &json!({"message": "401 Unauthorized"})),
        "group/html" => parity::html_answer(),
        "group/shape" => json_answer(200, &json!({"name": "shape"})),
        _ => json_answer(404, &json!({"message": "404 Project Not Found"})),
    }
}

/// GitLab's answer to `GET <base><target>`, where `base` is the API's
/// address and `target` the path below it with the query: the plugin's
/// canned answers come from here.
fn answer(base: &str, target: &str) -> Answer {
    let path = target.split('?').next().unwrap();
    let query = query(target);
    let segments: Vec<String> = path
        .trim_start_matches('/')
        .split('/')
        .map(parity::decode)
        .collect();
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    let not_found = || json_answer(404, &json!({"message": "404 Not Found"}));
    match segments.as_slice() {
        ["user"] => json_answer(
            200,
            &json!({"id": 7, "username": "dev", "name": "Dev\nEloper",
                    "web_url": format!("{}/dev", web(base))}),
        )
        .header("ratelimit-remaining", "1987"),
        ["projects", name] => project(base, name),
        ["merge_requests"] => {
            let (items, counted) = merge_requests(base, None, &query);
            page(base, target, &items, counted)
        }
        ["projects", name, "merge_requests"] => {
            let (items, counted) = merge_requests(base, Some(name), &query);
            page(base, target, &items, counted)
        }
        ["projects", "group/app", "merge_requests", "9"] => json_answer(
            200,
            &json!({
                "web_url": format!("{}/group/app/-/merge_requests/9", web(base)),
                "head_pipeline": {"id": 501, "status": "failed", "sha": "sha9",
                                  "web_url": format!("{}/group/app/-/pipelines/501", web(base))}
            }),
        ),
        // A merge request without a pipeline.
        ["projects", "group/app", "merge_requests", "10"] => json_answer(
            200,
            &json!({
                "web_url": format!("{}/group/app/-/merge_requests/10", web(base)),
                "head_pipeline": null
            }),
        ),
        ["projects", "group/app", "pipelines"] => {
            let found = match (query.get("ref").map(String::as_str), query.get("sha")) {
                (Some("main"), _) => json!([{
                    "id": 77, "status": "success", "sha": "0123abc",
                    "web_url": format!("{}/group/app/-/pipelines/77", web(base))
                }]),
                (Some("huge"), _) => json!([{"id": 900, "status": "running", "sha": "f00d"}]),
                _ => json!([]),
            };
            json_answer(200, &found)
        }
        ["projects", "group/app", "pipelines", id, "jobs"] => page(base, target, &jobs(id), true),
        _ => not_found(),
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

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// Every read action, on success and on each failure GitLab can answer.
/// `{web}` in an expectation is the GitLab's web address.
#[allow(clippy::too_many_lines)] // one list
fn readings() -> Vec<Scenario> {
    let project = |name, project: &str, expect| {
        scenario(name, "project", json!({ "project": project }), expect)
    };
    let failed = Expect::Failed;
    vec![
        scenario(
            "current-user",
            "current-user",
            json!({}),
            Expect::Ok(
                "signed in to GitLab as dev",
                &[
                    ("username", "dev"),
                    ("name", "Dev Eloper"),
                    ("url", "{web}/dev"),
                    ("rateLimitRemaining", "1987"),
                ],
            ),
        ),
        project(
            "project group/app",
            "group/app",
            Expect::Ok(
                "read group/app",
                &[
                    ("path", "group/app"),
                    ("defaultBranch", "main"),
                    ("visibility", "private"),
                    ("archived", "false"),
                    ("openIssues", "12"),
                    ("stars", "345"),
                    ("forks", "6"),
                    ("url", "{web}/group/app"),
                    ("description", "The  web app of the group"),
                    ("lastActivityAt", "2026-11-15T08:00:00Z"),
                ],
            ),
        ),
        project(
            "project group/bare (fallbacks)",
            "group/bare.git",
            Expect::Ok(
                "read group/bare",
                &[
                    ("path", "group/bare"),
                    ("archived", "true"),
                    ("visibility", "unknown"),
                    ("stars", "0"),
                    ("forks", "0"),
                    ("url", "{web}/group/bare"),
                ],
            ),
        ),
        project(
            "project by its id",
            "42",
            Expect::Ok(
                "read group/app",
                &[
                    ("path", "group/app"),
                    ("defaultBranch", "main"),
                    ("visibility", "private"),
                    ("archived", "false"),
                    ("openIssues", "12"),
                    ("stars", "345"),
                    ("forks", "6"),
                    ("url", "{web}/group/app"),
                    ("description", "The  web app of the group"),
                    ("lastActivityAt", "2026-11-15T08:00:00Z"),
                ],
            ),
        ),
        project(
            "project not a project",
            "not a project",
            failed("invalid-project"),
        ),
        project(
            "project group/missing",
            "group/missing",
            failed("not-found"),
        ),
        project(
            "project group/forbidden",
            "group/forbidden",
            failed("forbidden"),
        ),
        project(
            "project group/broken",
            "group/broken",
            failed("gitlab-error"),
        ),
        project(
            "project group/limited",
            "group/limited",
            failed("rate-limited"),
        ),
        project(
            "project group/secondary",
            "group/secondary",
            failed("rate-limited"),
        ),
        project(
            "project group/revoked",
            "group/revoked",
            failed("unauthorized"),
        ),
        project(
            "project group/html",
            "group/html",
            failed("invalid-response"),
        ),
        project(
            "project group/shape",
            "group/shape",
            failed("invalid-response"),
        ),
        scenario(
            "merge-requests of a project",
            "merge-requests",
            json!({"project": "group/app"}),
            Expect::Ok(
                "73 open merge requests in group/app (all)",
                &[
                    ("count", "73"),
                    ("countExact", "true"),
                    ("listed", "20"),
                    ("truncated", "true"),
                    ("firstUrl", "{web}/group/app/-/merge_requests/999"),
                ],
            ),
        ),
        scenario(
            "merge-requests, 70 listed",
            "merge-requests",
            json!({"project": "group/app", "limit": 70}),
            Expect::Ok(
                "73 open merge requests in group/app (all)",
                &[
                    ("count", "73"),
                    ("countExact", "true"),
                    ("listed", "70"),
                    ("truncated", "true"),
                    ("firstUrl", "{web}/group/app/-/merge_requests/999"),
                ],
            ),
        ),
        scenario(
            "merge-requests, 100 listed",
            "merge-requests",
            json!({"project": "group/app", "limit": 100}),
            Expect::Ok(
                "73 open merge requests in group/app (all)",
                &[
                    ("count", "73"),
                    ("countExact", "true"),
                    ("listed", "73"),
                    ("truncated", "false"),
                    ("firstUrl", "{web}/group/app/-/merge_requests/999"),
                ],
            ),
        ),
        scenario(
            "merge-requests, mine",
            "merge-requests",
            json!({"filter": "author-me"}),
            Expect::Ok(
                "3 open merge requests (author-me)",
                &[
                    ("count", "3"),
                    ("countExact", "true"),
                    ("listed", "3"),
                    ("truncated", "false"),
                    ("firstUrl", "{web}/group/app/-/merge_requests/999"),
                    (
                        "items",
                        "group/app!999 Fix the  build now (@dev1)\n\
                         group/api!998 Change number 2 (@dev2)\n\
                         group/app!997 Change number 3 (@dev0)",
                    ),
                ],
            ),
        ),
        scenario(
            "merge-requests, review requested in a project",
            "merge-requests",
            json!({"project": "group/app", "filter": "review-requested", "limit": 1}),
            Expect::Ok(
                "2 open merge requests in group/app (review-requested)",
                &[
                    ("count", "2"),
                    ("countExact", "true"),
                    ("listed", "1"),
                    ("truncated", "true"),
                    ("firstUrl", "{web}/group/app/-/merge_requests/999"),
                    ("items", "group/app!999 Fix the  build now (@dev1)"),
                ],
            ),
        ),
        scenario(
            "merge-requests, all without a project",
            "merge-requests",
            json!({"filter": "all"}),
            failed("invalid-project"),
        ),
        scenario(
            "merge-requests, not counted by GitLab",
            "merge-requests",
            json!({"project": "group/mid"}),
            Expect::Ok(
                "130 open merge requests in group/mid (all)",
                &[
                    ("count", "130"),
                    ("countExact", "true"),
                    ("listed", "20"),
                    ("truncated", "true"),
                    ("firstUrl", "{web}/group/mid/-/merge_requests/999"),
                ],
            ),
        ),
        scenario(
            "merge-requests, more than are counted",
            "merge-requests",
            json!({"project": "group/huge"}),
            Expect::Ok(
                "at least 500 open merge requests in group/huge (all)",
                &[
                    ("count", "500"),
                    ("countExact", "false"),
                    ("listed", "20"),
                    ("truncated", "true"),
                    ("firstUrl", "{web}/group/huge/-/merge_requests/999"),
                ],
            ),
        ),
        scenario(
            "review-requests",
            "review-requests",
            json!({}),
            Expect::Ok(
                "5 merge requests wait for your review",
                &[
                    ("count", "5"),
                    ("countExact", "true"),
                    ("listed", "5"),
                    ("truncated", "false"),
                    ("firstUrl", "{web}/group/app/-/merge_requests/999"),
                ],
            ),
        ),
        scenario(
            "review-requests in a project",
            "review-requests",
            json!({"project": "group/app", "limit": 1}),
            Expect::Ok(
                "2 merge requests wait for your review in group/app",
                &[
                    ("count", "2"),
                    ("countExact", "true"),
                    ("listed", "1"),
                    ("truncated", "true"),
                    ("firstUrl", "{web}/group/app/-/merge_requests/999"),
                ],
            ),
        ),
        scenario(
            "pipeline of a branch",
            "pipeline",
            json!({"project": "group/app", "ref": "main"}),
            Expect::Ok(
                "group/app@main: ok (2 passed, 2 failed, 1 pending)",
                &[
                    ("state", "ok"),
                    ("status", "success"),
                    ("pipelineId", "77"),
                    ("sha", "0123abc"),
                    ("total", "7"),
                    ("passed", "2"),
                    ("failed", "2"),
                    ("pending", "1"),
                    ("skipped", "2"),
                    ("failing", "unit  tests\njob-2"),
                    ("truncated", "false"),
                    ("url", "{web}/group/app/-/pipelines/77"),
                ],
            ),
        ),
        scenario(
            "pipeline of a merge request (paged)",
            "pipeline",
            json!({"project": "group/app", "mergeRequest": 9, "ref": "ignored"}),
            Expect::Ok(
                "group/app!9: failing (37 passed, 38 failed, 19 pending)",
                &[
                    ("state", "failing"),
                    ("status", "failed"),
                    ("pipelineId", "501"),
                    ("sha", "sha9"),
                    ("total", "130"),
                    ("skipped", "36"),
                    ("truncated", "false"),
                    ("url", "{web}/group/app/-/merge_requests/9"),
                ],
            ),
        ),
        scenario(
            "pipeline of a merge request without one",
            "pipeline",
            json!({"project": "group/app", "mergeRequest": 10}),
            Expect::Ok(
                "group/app!10: no pipeline",
                &[
                    ("state", "none"),
                    ("total", "0"),
                    ("url", "{web}/group/app/-/merge_requests/10"),
                ],
            ),
        ),
        scenario(
            "pipeline with more than 500 jobs (truncated)",
            "pipeline",
            json!({"project": "group/app", "ref": "huge"}),
            Expect::Ok(
                "group/app@huge: pending (142 passed, 144 failed, 72 pending)",
                &[
                    ("pipelineId", "900"),
                    ("sha", "f00d"),
                    ("skipped", "142"),
                    ("truncated", "true"),
                ],
            ),
        ),
        scenario(
            "pipeline of a commit without one",
            "pipeline",
            json!({"project": "group/app", "ref": SHA}),
            Expect::Ok(
                "group/app@0123456789abcdef0123456789abcdef01234567: no pipeline",
                &[("state", "none"), ("total", "0")],
            ),
        ),
        scenario(
            "pipeline of a missing merge request",
            "pipeline",
            json!({"project": "group/app", "mergeRequest": 404}),
            failed("not-found"),
        ),
        scenario(
            "pipeline of an invalid ref",
            "pipeline",
            json!({"project": "group/app", "ref": "a..b"}),
            failed("invalid-ref"),
        ),
        scenario(
            "pipeline without a ref",
            "pipeline",
            json!({"project": "group/app"}),
            failed("invalid-parameters"),
        ),
    ]
}

/// The readings repeated on the GitLab the user enters: what depends on
/// where the API is (the web address under a path, the links to further
/// pages, a project path as one segment below the path prefix). The
/// agent's 60 requests per minute are the reason it is not all of them.
const ON_THE_ENTERED_GITLAB: &[&str] = &[
    "current-user",
    "project group/app",
    "project group/bare (fallbacks)",
    "project group/missing",
    "merge-requests of a project",
    "merge-requests, 70 listed",
    "merge-requests, mine",
    "review-requests in a project",
    "pipeline of a branch",
    "pipeline of a merge request (paged)",
    "pipeline of an invalid ref",
];

/// Most requests one scenario sends (a list of more than 500).
const MOST_REQUESTS: usize = 12;

/// Projects whose request gets no answer, and the transport's code (a
/// name that never resolves, a certificate that is not trusted), canned.
const NETWORK_FAILURES: &[(&str, &str)] = &[
    ("group/unresolved", "dns-failed"),
    ("group/untrusted", "tls-failed"),
];

/// The canned answers of the plugin's HTTP for the GitLab at `base`:
/// [`answer`] for every URL the plugin asks for in [`readings`].
fn plugin_routes(base: &str) -> Vec<Value> {
    let mut urls: Vec<String> = [
        "/user",
        "/projects/group%2Fapp",
        "/projects/group%2Fbare",
        "/projects/42",
        "/projects/group%2Fmissing",
        "/projects/group%2Fforbidden",
        "/projects/group%2Fbroken",
        "/projects/group%2Flimited",
        "/projects/group%2Fsecondary",
        "/projects/group%2Frevoked",
        "/projects/group%2Fhtml",
        "/projects/group%2Fshape",
        "/projects/group%2Fapp/merge_requests/9",
        "/projects/group%2Fapp/merge_requests/10",
        "/projects/group%2Fapp/merge_requests/404",
    ]
    .iter()
    .map(|p| format!("{base}{p}"))
    .collect();
    let list = |path: &str, me: Option<&str>, per_page: usize| {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.extend_pairs([
            ("state", "opened"),
            ("order_by", "updated_at"),
            ("sort", "desc"),
        ]);
        if let Some(me) = me {
            query.append_pair(me, "7");
        }
        if path == "/merge_requests" {
            query.append_pair("scope", "all");
        }
        query.append_pair("per_page", &per_page.to_string());
        format!("{base}{path}?{}", query.finish())
    };
    let of = |project: &str| format!("/projects/group%2F{project}/merge_requests");
    urls.extend([
        list(&of("app"), None, 20),
        list(&of("app"), None, 50),
        list("/merge_requests", Some("author_id"), 20),
        list(&of("app"), Some("reviewer_id"), 1),
        list("/merge_requests", Some("reviewer_id"), 20),
        list(&of("mid"), None, 20),
        list(&of("mid"), None, 50),
        list(&of("huge"), None, 20),
        list(&of("huge"), None, 50),
    ]);
    let pipelines = format!("{base}/projects/group%2Fapp/pipelines");
    for by in ["ref=main", "ref=huge", &format!("sha={SHA}")] {
        urls.push(format!("{pipelines}?{by}&order_by=id&sort=desc&per_page=1"));
    }
    for id in ["77", "501", "900"] {
        urls.push(format!("{pipelines}/{id}/jobs?per_page=50"));
    }
    let failures: Vec<(String, &str)> = NETWORK_FAILURES
        .iter()
        .map(|(project, code)| {
            (
                format!("{base}/projects/{}", project.replace('/', "%2F")),
                *code,
            )
        })
        .collect();
    parity::routes(base, answer, &urls, &failures)
}

/// What one run covered: scenario, whether it succeeded, its code.
type Covered = Vec<(&'static str, bool, Option<&'static str>)>;

/// The plugin under test.
struct Side<'a> {
    env: &'a parity::Env,
    agent: &'a parity::Agent,
    pace: &'a parity::Pace,
}

/// The readings (all, or those named in `only`) on the GitLab at `api`.
async fn check_readings(
    under_test: &Side<'_>,
    api: &str,
    only: Option<&[&str]>,
    problems: &mut Vec<String>,
) -> Covered {
    let vars = [("{web}", web(api))];
    let mut covered = Vec::new();
    for s in readings() {
        if only.is_some_and(|only| !only.contains(&s.name)) {
            continue;
        }
        under_test
            .pace
            .room_for(under_test.env, MOST_REQUESTS)
            .await;
        let outcome = under_test.agent.invoke(s.action, &s.params).await;
        parity::check(s.name, &outcome, s.expect, &vars, problems);
        covered.push((s.name, s.expect.ok(), s.expect.code()));
    }
    covered
}

/// Opens pages, one per 2 s (a plugin's limit).
async fn check_opens(
    agent: &parity::Agent,
    pages: &[(&'static str, Value, Expect)],
    problems: &mut Vec<String>,
) -> Covered {
    let mut covered = Vec::new();
    for (name, params, expect) in pages {
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        let outcome = agent.invoke("open", params).await;
        parity::check(name, &outcome, *expect, &[], problems);
        covered.push((*name, expect.ok(), expect.code()));
    }
    covered
}

/// What the plugin sent: only GET to the API at `api`, each with the
/// token in GitLab's header, and only what the scenarios ask for.
fn sent_only_to(sent: &[String], api: &str, routes: &[Value]) {
    assert_ne!(sent.len(), 0);
    for line in sent {
        let (url, credential) = line.split_once('\t').unwrap_or((line, ""));
        assert!(url.starts_with(&format!("{api}/")), "{line}");
        assert_eq!(credential, format!("PRIVATE-TOKEN: {TOKEN}"), "{url}");
    }
    assert_eq!(parity::unrouted(sent, routes), Vec::<String>::new());
}

// ---------------------------------------------------------------- test

/// Every action of `deckpilot.gitlab` on known answers, with the results
/// the built-in gave; on GitLab.com and on a GitLab the user enters.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // the steps of one run, in order
async fn the_plugin_answers_as_the_builtin_did() {
    let mut routes = plugin_routes(API);
    routes.extend(plugin_routes(OWN_API));
    let env = parity::Env::new(PLUGIN, &routes);
    let agent = env.start().await;
    let pace = parity::Pace::start();
    let under_test = Side {
        env: &env,
        agent: &agent,
        pace: &pace,
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
    assert_eq!(summary.actions.len(), 6);
    let problems = parity::contract_problems("gitlab", ID, CONTRACT, &summary.actions);
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

    // Without a token nothing is sent; the plugin names its slot.
    let mut problems = Vec::new();
    let without = agent.invoke("current-user", &json!({})).await;
    parity::check(
        "current-user without a token",
        &without,
        Expect::Failed("credential-missing"),
        &[],
        &mut problems,
    );
    assert!(
        without.message.contains("Settings › Plugins › GitLab"),
        "{without:?}"
    );
    assert_eq!(env.sent().len(), 0);

    agent
        .ok(Request::SetPluginCredential {
            plugin: ID.into(),
            key: "token".into(),
            secret: PluginSecret::new(TOKEN),
        })
        .await;

    // ---- GitLab.com: the plugin's declared host, no consent of its own.
    let mut covered = check_readings(&under_test, API, None, &mut problems).await;
    let pages = [
        (
            "open a merge request",
            json!({"project": "group/app", "mergeRequest": 12}),
            Expect::Ok(
                "opened https://gitlab.com/group/app/-/merge_requests/12",
                &[("url", "https://gitlab.com/group/app/-/merge_requests/12")],
            ),
        ),
        (
            "open a page",
            json!({"url": " https://GitLab.com/group/app/-/pipelines/77?scope=failed#jobs "}),
            // Shown without its query.
            Expect::Ok(
                "opened https://gitlab.com/group/app/-/pipelines/77",
                &[("url", "https://gitlab.com/group/app/-/pipelines/77")],
            ),
        ),
        (
            "open another host",
            json!({"url": "https://about.gitlab.com/pricing"}),
            Expect::Failed("url-not-allowed"),
        ),
        (
            "open a project by its id",
            json!({"project": "42"}),
            Expect::Failed("invalid-project"),
        ),
        ("open nothing", json!({}), Expect::Failed("invalid-project")),
    ];
    covered.extend(check_opens(&agent, &pages, &mut problems).await);
    let on_gitlab_com = [
        "https://gitlab.com/group/app/-/merge_requests/12",
        "https://gitlab.com/group/app/-/pipelines/77?scope=failed#jobs",
    ];
    assert_eq!(env.opened(), on_gitlab_com);
    assert!(
        problems.is_empty(),
        "{}\n{}",
        problems.join("\n"),
        agent.log()
    );
    // The scenarios cover success and every failure code of the module.
    let codes: BTreeSet<_> = covered.iter().filter_map(|(_, _, c)| *c).collect();
    for code in [
        "invalid-project",
        "invalid-ref",
        "unauthorized",
        "forbidden",
        "not-found",
        "rate-limited",
        "gitlab-error",
        "invalid-response",
        "url-not-allowed",
        "invalid-parameters",
    ] {
        assert!(codes.contains(code), "no scenario ends in {code}");
    }
    assert!(covered.iter().filter(|(_, ok, _)| *ok).count() >= 15);
    let sent_to_gitlab_com = env.sent();
    sent_only_to(&sent_to_gitlab_com, API, &routes);

    // ---- a self-managed GitLab under a path: the user enters it.
    let settings = agent
        .set_settings(BTreeMap::from([(
            "apiUrl".to_owned(),
            json!("https://Code.Example.com:443/gitlab/api/v4/"),
        )]))
        .await;
    let entered = &settings.origins[0];
    assert_eq!(entered.origin.as_deref(), Some(OWN_API), "{entered:?}");
    assert_eq!(entered.status, UserOriginStatus::NeedsConsent);
    assert_eq!(entered.credentials, ["token"]);
    assert!(entered.opens_pages && !entered.private_host, "{entered:?}");
    // Entered, not allowed: nothing is sent there, no token, no page.
    let refused = agent.invoke("current-user", &json!({})).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    let refused = agent.invoke("open", &json!({"project": "group/app"})).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    assert_eq!(
        env.sent(),
        sent_to_gitlab_com,
        "a request before the consent"
    );
    assert_eq!(env.opened(), on_gitlab_com, "a page before the consent");

    // The user allows exactly that server.
    agent
        .ok(Request::GrantPluginUserOrigin {
            plugin: ID.into(),
            setting: "apiUrl".into(),
            origin: OWN_API.into(),
            private_network: false,
        })
        .await;
    let on_own = check_readings(
        &under_test,
        OWN_API,
        Some(ON_THE_ENTERED_GITLAB),
        &mut problems,
    )
    .await;
    assert_eq!(on_own.len(), ON_THE_ENTERED_GITLAB.len());
    let pages = [
        (
            "open a merge request of the entered GitLab",
            json!({"project": "group/sub/app", "mergeRequest": 12}),
            Expect::Ok(
                "opened https://code.example.com/gitlab/group/sub/app/-/merge_requests/12",
                &[(
                    "url",
                    "https://code.example.com/gitlab/group/sub/app/-/merge_requests/12",
                )],
            ),
        ),
        (
            "open a page of the entered GitLab",
            json!({"url": "https://code.example.com/gitlab/group/app/-/pipelines/77?scope=failed"}),
            Expect::Ok(
                "opened https://code.example.com/gitlab/group/app/-/pipelines/77",
                &[(
                    "url",
                    "https://code.example.com/gitlab/group/app/-/pipelines/77",
                )],
            ),
        ),
        (
            "open a page beside the entered GitLab",
            json!({"url": "https://code.example.com/wiki/Home"}),
            Expect::Failed("url-not-allowed"),
        ),
        (
            "open GitLab.com from the entered GitLab",
            json!({"url": "https://gitlab.com/group/app"}),
            Expect::Failed("url-not-allowed"),
        ),
    ];
    check_opens(&agent, &pages, &mut problems).await;
    assert!(
        problems.is_empty(),
        "{}\n{}",
        problems.join("\n"),
        agent.log()
    );
    assert_eq!(
        env.opened()[on_gitlab_com.len()..],
        [
            "https://code.example.com/gitlab/group/sub/app/-/merge_requests/12",
            "https://code.example.com/gitlab/group/app/-/pipelines/77?scope=failed"
        ]
    );
    let sent = env.sent();
    sent_only_to(&sent[sent_to_gitlab_com.len()..], OWN_API, &routes);

    // The consent revoked: refused on the very next call.
    agent
        .ok(Request::RevokePluginUserOrigin {
            plugin: ID.into(),
            setting: "apiUrl".into(),
        })
        .await;
    let refused = agent.invoke("current-user", &json!({})).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    assert_eq!(env.sent(), sent, "a request after the consent was revoked");

    // Back to GitLab.com: no consent needed.
    agent
        .set_settings(BTreeMap::from([("apiUrl".to_owned(), Value::Null)]))
        .await;
    pace.room_for(&env, 3).await;
    let back = agent.invoke("current-user", &json!({})).await;
    assert!(back.ok, "{back:?}");
    assert_eq!(back.data["url"], "https://gitlab.com/dev");

    network_failures(&agent).await;
    // The token is in no log line and no answer.
    assert!(
        !agent.log().contains(TOKEN),
        "the token is in the agent's log"
    );
}

/// No answer at all: a name that does not resolve and a certificate that
/// is not trusted end in the transport's codes, with its retryability
/// (`dns-failed` retryable, `tls-failed` not), and a message that does
/// not name the token.
async fn network_failures(agent: &parity::Agent) {
    for (project, code) in NETWORK_FAILURES {
        let params = json!({ "project": project });
        let outcome = agent.invoke_raw("project", &params).await;
        eprintln!("network {project}: {outcome:?}");
        let PluginActionOutcome::Failed {
            code: plugin_code,
            message,
            retryable,
        } = &outcome
        else {
            panic!("{project}: {outcome:?}\n{}", agent.log());
        };
        assert_eq!(plugin_code, code, "{outcome:?}");
        assert_eq!(*retryable, *code == "dns-failed", "{outcome:?}");
        let message = message.as_deref().unwrap_or_default();
        assert!(!message.contains(TOKEN), "{message}");
        assert_ne!(message, "");
    }
}
