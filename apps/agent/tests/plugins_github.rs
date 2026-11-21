//! `deckpilot.github`, the first-party plugin that replaced the removed
//! built-in `github` module (`docs/marketplace/module-migration.md` §6,
//! P8), as a regression test of every action on known GitHub answers
//! (feature `plugins`):
//!
//! * the plugin (`plugins/first-party/github`) is built to wasm with the
//!   `guest` profile and bundled as an installed application ships it;
//!   the real agent seeds it, the user consents to its permissions and
//!   stores its token, and every action runs in the real sandboxed host,
//!   invoked over IPC like the configuration app does;
//! * its HTTP goes to canned answers (`DECKPILOT_TEST_PLUGIN_HTTP`, feature
//!   `test-hooks`) built from one function ([`answer`]) for the requests
//!   the scenarios make, and its opened pages to a file
//!   (`DECKPILOT_TEST_PLUGIN_OPEN_URL`): never the real GitHub, never a
//!   browser;
//! * every scenario's result is checked: status, error code, and on
//!   success the message and the key outputs. The plugin was proven to
//!   answer exactly as the built-in did before the built-in was removed
//!   (§11 D2); these expectations are those answers. Its action
//!   definitions are checked against the built-in's frozen 0.9.0 contract
//!   (`crates/deckpilot-modules/legacy/github.json`), as the cut-over's
//!   aliases need;
//! * network failures (a name that does not resolve, a certificate that
//!   is not trusted) end in the transport's code and retryability: the
//!   plugin reads the agent's network kind (`dp_features`);
//! * a GitHub Enterprise Server the user enters in `apiUrl` (the plugin's
//!   user-origin slot) is reached only once the user allowed exactly that
//!   server (a private one only with its own consent), and only until the
//!   consent is revoked; api.github.com needs none of its own.
//!
//! The plugin is pinned here as a `default`-class plugin and invoked by its
//! own action types: `migration`-class plugins are installed only by the
//! migration engine, and the legacy `github.*` types reach the plugin only
//! after a computer's migration is cut over. That path, from an alpha.3
//! configuration through the consent, is `plugins_cutover.rs`.
#![cfg(feature = "plugins")]

#[path = "support/parity.rs"]
mod parity;
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use deckpilot_ipc::plugins::{
    PluginActionOutcome, PluginSecret, PluginSettings, PluginState, UserOriginStatus,
};
use deckpilot_ipc::{Request, Response};
use parity::{Answer, Expect, json_answer, query};
use serde_json::{Value, json};

const PLUGIN: parity::Plugin = parity::Plugin {
    id: "deckpilot.github",
    dir: "github",
    package: "deckpilot-github",
};
const ID: &str = PLUGIN.id;
const GRANTS: &[&str] = &[
    "network:api.github.com",
    "network:user-origin:apiUrl",
    "credentials:token",
    "openUrl:github.com",
    "openUrl:user-origin:apiUrl",
];
/// The token the user stores.
const TOKEN: &str = "ghp_parity-4d1f7a-not-a-real-token";
const API: &str = "https://api.github.com";
/// A GitHub Enterprise Server the user enters, as the agent normalizes it.
const GHE_API: &str = "https://ghe.example.com/api/v3";
/// What the plugin answers may use of the IPC message limit.
const ANSWER_BUDGET: usize = deckpilot_ipc::framing::MAX_MESSAGE_BYTES / 2;
/// The built-in's frozen 0.9.0 contract.
const CONTRACT: &str = include_str!("../../../crates/deckpilot-modules/legacy/github.json");

// ------------------------------------------------------- GitHub's answers

/// One page of `items`, GitHub style: `per_page` and `page` from the
/// query, a `link` to the next page (on `base`) while there is one.
fn page(base: &str, target: &str, key: &str, items: &[Value], extra: Value) -> Answer {
    let query = query(target);
    let per_page: usize = query.get("per_page").map_or(30, |v| v.parse().unwrap());
    let page: usize = query.get("page").map_or(1, |v| v.parse().unwrap());
    let start = ((page - 1) * per_page).min(items.len());
    let end = (page * per_page).min(items.len());
    let mut body = extra;
    body[key] = Value::Array(items[start..end].to_vec());
    let mut answer = json_answer(200, &body);
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

/// Open pull requests found by a search `q`.
fn search_items(q: &str) -> Vec<Value> {
    let site = q.contains("repo:octo/site");
    let count = match (
        site,
        q.contains("author:@me"),
        q.contains("review-requested:@me"),
    ) {
        (true, false, false) => 73,
        (true, false, true) => 2,
        (false, true, false) => 3,
        (false, false, true) => 5,
        _ => 0,
    };
    (1..=count)
        .map(|i| {
            let repo = if site || i % 2 == 1 {
                "octo/site"
            } else {
                "octo/api"
            };
            let number = 1000 - i;
            let title = if i == 1 {
                "Fix\tthe\u{202e} build\nnow".to_owned()
            } else {
                format!("Change number {i}")
            };
            json!({
                "number": number,
                "title": title,
                "draft": i % 5 == 0,
                "repository_url": format!("{API}/repos/{repo}"),
                "user": {"login": format!("dev{}", i % 3)},
                "html_url": format!("https://github.com/{repo}/pull/{number}")
            })
        })
        .collect()
}

fn check_runs(reference: &str) -> Vec<Value> {
    let count = match reference {
        "main" => 7,
        "sha9" => 130,
        "huge" => 620,
        _ => 0,
    };
    (0..count)
        .map(|i| {
            let (status, conclusion) = match i % 7 {
                0 => ("in_progress", Value::Null),
                1 => ("completed", json!("failure")),
                2 => ("completed", json!("cancelled")),
                3 => ("completed", json!("neutral")),
                4 => ("completed", json!("skipped")),
                _ => ("completed", json!("success")),
            };
            json!({"name": format!("check-{i}"), "status": status, "conclusion": conclusion})
        })
        .collect()
}

fn statuses(reference: &str) -> (Vec<Value>, &'static str) {
    match reference {
        "main" => (
            vec![
                json!({"context": "ci/a", "state": "success"}),
                json!({"context": "ci/b", "state": "pending"}),
                json!({"context": "ci/c\u{7}", "state": "error"}),
            ],
            "0123abc",
        ),
        "sha9" => (
            vec![json!({"context": "ci/legacy", "state": "failure"})],
            "sha9",
        ),
        _ => (Vec::new(), "f00d"),
    }
}

/// GitHub's answer to `GET <base><target>`, where `target` is the path
/// and query: the plugin's canned answers come from here.
fn answer(base: &str, target: &str) -> Answer {
    let path = target.split('?').next().unwrap();
    let not_found = || json_answer(404, &json!({"message": "Not Found"}));
    match path {
        "/user" => json_answer(
            200,
            &json!({"login": "octocat", "name": "Mona\nOcto", "html_url": "https://github.com/octocat"}),
        )
        .header("x-ratelimit-remaining", "4321"),
        "/repos/octo/site" => json_answer(
            200,
            &json!({
                "full_name": "octo/site", "default_branch": "main", "private": true,
                "archived": false, "open_issues_count": 12, "stargazers_count": 345,
                "forks_count": 6, "html_url": "https://github.com/octo/site",
                "description": "The\u{202e} web site\nof octo", "pushed_at": "2026-11-15T08:00:00Z"
            }),
        ),
        "/repos/octo/bare" => json_answer(
            200,
            &json!({
                "full_name": "octo/bare", "default_branch": "trunk", "private": false,
                "archived": true, "open_issues_count": 0, "description": null, "pushed_at": null
            }),
        ),
        "/repos/octo/forbidden" => json_answer(
            403,
            &json!({"message": "Resource protected by organization SAML enforcement"}),
        ),
        "/repos/octo/broken" => json_answer(502, &json!({"message": "Server Error"})),
        "/repos/octo/limited" => json_answer(403, &json!({"message": "API rate limit exceeded"}))
            .header("x-ratelimit-remaining", "0")
            .header("x-ratelimit-reset", "1000000000"),
        "/repos/octo/secondary" => json_answer(429, &json!({})).header("retry-after", "30"),
        "/repos/octo/revoked" => json_answer(401, &json!({"message": "Bad credentials"})),
        "/repos/octo/html" => parity::html_answer(),
        "/repos/octo/shape" => json_answer(200, &json!({"name": "shape"})),
        "/repos/octo/site/pulls/9" => json_answer(
            200,
            &json!({"head": {"sha": "sha9"}, "html_url": "https://github.com/octo/site/pull/9"}),
        ),
        "/search/issues" => {
            let items = search_items(&query(target)["q"]);
            page(
                base,
                target,
                "items",
                &items,
                json!({"total_count": items.len(), "incomplete_results": false}),
            )
        }
        _ => {
            let Some(rest) = path.strip_prefix("/repos/octo/site/commits/") else {
                return not_found();
            };
            match rest.split_once('/') {
                Some((reference, "check-runs")) => {
                    let runs = check_runs(&parity::decode(reference));
                    page(
                        base,
                        target,
                        "check_runs",
                        &runs,
                        json!({"total_count": runs.len()}),
                    )
                }
                Some((reference, "status")) => {
                    let (list, sha) = statuses(&parity::decode(reference));
                    page(
                        base,
                        target,
                        "statuses",
                        &list,
                        json!({"sha": sha, "state": "pending"}),
                    )
                }
                _ => not_found(),
            }
        }
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

/// Every read action, on success and on each failure GitHub can answer.
#[allow(clippy::too_many_lines)] // one list
fn readings() -> Vec<Scenario> {
    let repository = |name, repo: &str, expect| {
        scenario(name, "repository", json!({"repository": repo}), expect)
    };
    let failed = Expect::Failed;
    vec![
        scenario(
            "current-user",
            "current-user",
            json!({}),
            Expect::Ok(
                "signed in to GitHub as octocat",
                &[
                    ("login", "octocat"),
                    ("name", "Mona Octo"),
                    ("url", "https://github.com/octocat"),
                    ("rateLimitRemaining", "4321"),
                ],
            ),
        ),
        repository(
            "repository octo/site",
            "octo/site",
            Expect::Ok(
                "read octo/site",
                &[
                    ("fullName", "octo/site"),
                    ("defaultBranch", "main"),
                    ("private", "true"),
                    ("archived", "false"),
                    ("openIssues", "12"),
                    ("stars", "345"),
                    ("forks", "6"),
                    ("url", "https://github.com/octo/site"),
                    ("description", "The  web site of octo"),
                    ("pushedAt", "2026-11-15T08:00:00Z"),
                ],
            ),
        ),
        repository(
            "repository octo/bare (fallbacks)",
            "octo/bare.git",
            Expect::Ok(
                "read octo/bare",
                &[
                    ("fullName", "octo/bare"),
                    ("defaultBranch", "trunk"),
                    ("private", "false"),
                    ("archived", "true"),
                    ("openIssues", "0"),
                    ("stars", "0"),
                    ("forks", "0"),
                    ("url", "https://github.com/octo/bare"),
                ],
            ),
        ),
        repository(
            "repository not a repository",
            "not a repo",
            failed("invalid-repository"),
        ),
        repository(
            "repository octo/missing",
            "octo/missing",
            failed("not-found"),
        ),
        repository(
            "repository octo/forbidden",
            "octo/forbidden",
            failed("forbidden"),
        ),
        repository(
            "repository octo/broken",
            "octo/broken",
            failed("github-error"),
        ),
        repository(
            "repository octo/limited",
            "octo/limited",
            failed("rate-limited"),
        ),
        repository(
            "repository octo/secondary",
            "octo/secondary",
            failed("rate-limited"),
        ),
        repository(
            "repository octo/revoked",
            "octo/revoked",
            failed("unauthorized"),
        ),
        repository(
            "repository octo/html",
            "octo/html",
            failed("invalid-response"),
        ),
        repository(
            "repository octo/shape",
            "octo/shape",
            failed("invalid-response"),
        ),
        scenario(
            "pull-requests of a repository",
            "pull-requests",
            json!({"repository": "octo/site"}),
            Expect::Ok(
                "73 open pull requests in octo/site (all)",
                &[
                    ("count", "73"),
                    ("listed", "20"),
                    ("truncated", "true"),
                    ("firstUrl", "https://github.com/octo/site/pull/999"),
                ],
            ),
        ),
        scenario(
            "pull-requests, 70 listed",
            "pull-requests",
            json!({"repository": "octo/site", "limit": 70}),
            Expect::Ok(
                "73 open pull requests in octo/site (all)",
                &[
                    ("count", "73"),
                    ("listed", "70"),
                    ("truncated", "true"),
                    ("firstUrl", "https://github.com/octo/site/pull/999"),
                ],
            ),
        ),
        scenario(
            "pull-requests, 100 listed",
            "pull-requests",
            json!({"repository": "octo/site", "limit": 100}),
            Expect::Ok(
                "73 open pull requests in octo/site (all)",
                &[
                    ("count", "73"),
                    ("listed", "73"),
                    ("truncated", "false"),
                    ("firstUrl", "https://github.com/octo/site/pull/999"),
                ],
            ),
        ),
        scenario(
            "pull-requests, mine",
            "pull-requests",
            json!({"filter": "author-me"}),
            Expect::Ok(
                "3 open pull requests (author-me)",
                &[
                    ("count", "3"),
                    ("listed", "3"),
                    ("truncated", "false"),
                    ("firstUrl", "https://github.com/octo/site/pull/999"),
                    (
                        "items",
                        "octo/site#999 Fix the  build now (@dev1)\n\
                         octo/api#998 Change number 2 (@dev2)\n\
                         octo/site#997 Change number 3 (@dev0)",
                    ),
                ],
            ),
        ),
        scenario(
            "pull-requests, review requested in a repository",
            "pull-requests",
            json!({"repository": "octo/site", "filter": "review-requested", "limit": 1}),
            Expect::Ok(
                "2 open pull requests in octo/site (review-requested)",
                &[
                    ("count", "2"),
                    ("listed", "1"),
                    ("truncated", "true"),
                    ("firstUrl", "https://github.com/octo/site/pull/999"),
                    ("items", "octo/site#999 Fix the  build now (@dev1)"),
                ],
            ),
        ),
        scenario(
            "pull-requests, all without a repository",
            "pull-requests",
            json!({"filter": "all"}),
            failed("invalid-repository"),
        ),
        scenario(
            "review-requests",
            "review-requests",
            json!({}),
            Expect::Ok(
                "5 pull requests wait for your review",
                &[
                    ("count", "5"),
                    ("listed", "5"),
                    ("truncated", "false"),
                    ("firstUrl", "https://github.com/octo/site/pull/999"),
                ],
            ),
        ),
        scenario(
            "review-requests in a repository",
            "review-requests",
            json!({"repository": "octo/site", "limit": 1}),
            Expect::Ok(
                "2 pull requests wait for your review in octo/site",
                &[
                    ("count", "2"),
                    ("listed", "1"),
                    ("truncated", "true"),
                    ("firstUrl", "https://github.com/octo/site/pull/999"),
                ],
            ),
        ),
        scenario(
            "checks of a branch",
            "checks",
            json!({"repository": "octo/site", "ref": "main"}),
            Expect::Ok(
                "octo/site@main: failing (5 passed, 3 failed, 2 pending)",
                &[
                    ("state", "failing"),
                    ("sha", "0123abc"),
                    ("total", "10"),
                    ("passed", "5"),
                    ("failed", "3"),
                    ("pending", "2"),
                    ("failing", "check-1\ncheck-2\nci/c"),
                    ("truncated", "false"),
                    ("url", "https://github.com/octo/site/commit/0123abc"),
                ],
            ),
        ),
        scenario(
            "checks of a pull request (paged)",
            "checks",
            json!({"repository": "octo/site", "pullRequest": 9, "ref": "ignored"}),
            Expect::Ok(
                "octo/site#9: failing (73 passed, 39 failed, 19 pending)",
                &[
                    ("state", "failing"),
                    ("sha", "sha9"),
                    ("total", "131"),
                    ("truncated", "false"),
                    ("url", "https://github.com/octo/site/pull/9"),
                ],
            ),
        ),
        scenario(
            "checks beyond 500 (truncated)",
            "checks",
            json!({"repository": "octo/site", "ref": "huge"}),
            Expect::Ok(
                "octo/site@huge: failing (284 passed, 144 failed, 72 pending)",
                &[
                    ("state", "failing"),
                    ("sha", "f00d"),
                    ("total", "500"),
                    ("truncated", "true"),
                    ("url", "https://github.com/octo/site/commit/f00d"),
                ],
            ),
        ),
        scenario(
            "checks of a missing pull request",
            "checks",
            json!({"repository": "octo/site", "pullRequest": 404}),
            failed("not-found"),
        ),
        scenario(
            "checks of an invalid ref",
            "checks",
            json!({"repository": "octo/site", "ref": "a..b"}),
            failed("invalid-ref"),
        ),
        scenario(
            "checks without a ref",
            "checks",
            json!({"repository": "octo/site"}),
            failed("invalid-parameters"),
        ),
    ]
}

/// Most requests one scenario sends (a list of more than 500).
const MOST_REQUESTS: usize = 14;

/// Repositories whose request gets no answer, and the transport's code (a
/// name that never resolves, a certificate that is not trusted), canned.
const NETWORK_FAILURES: &[(&str, &str)] = &[
    ("octo/unresolved", "dns-failed"),
    ("octo/untrusted", "tls-failed"),
];

/// The plugin's canned answers: [`answer`] for every URL the plugin asks
/// for in [`readings`], and every further page their links lead to; and
/// the GitHub Enterprise Server's signed-in user.
fn plugin_routes() -> Vec<Value> {
    let mut urls: Vec<String> = [
        "/user",
        "/repos/octo/site",
        "/repos/octo/bare",
        "/repos/octo/missing",
        "/repos/octo/forbidden",
        "/repos/octo/broken",
        "/repos/octo/limited",
        "/repos/octo/secondary",
        "/repos/octo/revoked",
        "/repos/octo/html",
        "/repos/octo/shape",
        "/repos/octo/site/pulls/9",
        "/repos/octo/site/pulls/404",
    ]
    .iter()
    .map(|p| format!("{API}{p}"))
    .collect();
    let search = |q: &str, per_page: usize| {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("q", q),
                ("sort", "updated"),
                ("order", "desc"),
                ("per_page", &per_page.to_string()),
            ])
            .finish();
        format!("{API}/search/issues?{query}")
    };
    urls.extend([
        search("is:pr is:open repo:octo/site", 20),
        search("is:pr is:open repo:octo/site", 50),
        search("is:pr is:open archived:false author:@me", 20),
        search("is:pr is:open repo:octo/site review-requested:@me", 1),
        search("is:pr is:open archived:false review-requested:@me", 20),
    ]);
    for reference in ["main", "sha9", "huge"] {
        let commits = format!("{API}/repos/octo/site/commits/{reference}");
        urls.push(format!("{commits}/check-runs?per_page=50&filter=latest"));
        urls.push(format!("{commits}/status?per_page=50"));
    }
    let failures: Vec<(String, &str)> = NETWORK_FAILURES
        .iter()
        .map(|(repository, code)| (format!("{API}/repos/{repository}"), *code))
        .collect();
    let mut routes = parity::routes(API, answer, &urls, &failures);
    routes.extend(parity::routes(
        GHE_API,
        answer,
        &[format!("{GHE_API}/user")],
        &[],
    ));
    routes
}

async fn settings(agent: &parity::Agent, api_url: Value) -> PluginSettings {
    agent
        .set_settings(BTreeMap::from([("apiUrl".to_owned(), api_url)]))
        .await
}

// ---------------------------------------------------------------- test

/// Every action of `deckpilot.github` on known answers, with the results
/// the built-in gave.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // the steps of one run, in order
async fn the_plugin_answers_as_the_builtin_did() {
    let routes = plugin_routes();
    let env = parity::Env::new(PLUGIN, &routes);
    let agent = env.start().await;
    let pace = parity::Pace::start();

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
    let problems = parity::contract_problems("github", ID, CONTRACT, &summary.actions);
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
        without.message.contains("Settings › Plugins › GitHub"),
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

    let ghe_opened = github_enterprise(&env, &agent).await;
    let ghe_sent = env.sent().len();

    // Every read.
    let mut covered = Vec::new();
    for s in readings() {
        pace.room_for(&env, MOST_REQUESTS).await;
        let outcome = agent.invoke(s.action, &s.params).await;
        parity::check(s.name, &outcome, s.expect, &[], &mut problems);
        covered.push((s.name, s.expect.ok(), s.expect.code()));
    }

    // Open: the page opened, shown without its query. (One page per 2 s
    // for a plugin.)
    let pages = [
        (
            "open a pull request",
            json!({"repository": "octo/site", "pullRequest": 12}),
            Expect::Ok(
                "opened https://github.com/octo/site/pull/12",
                &[("url", "https://github.com/octo/site/pull/12")],
            ),
        ),
        (
            "open a page",
            json!({"url": " https://GitHub.com/octo/site/pull/12/files?diff=split#top "}),
            // Shown without its query.
            Expect::Ok(
                "opened https://github.com/octo/site/pull/12/files",
                &[("url", "https://github.com/octo/site/pull/12/files")],
            ),
        ),
        (
            "open another host",
            json!({"url": "https://gist.github.com/octo/1"}),
            Expect::Failed("url-not-allowed"),
        ),
        (
            "open nothing",
            json!({}),
            Expect::Failed("invalid-repository"),
        ),
    ];
    for (i, (name, params, expect)) in pages.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(2_100)).await;
        }
        let outcome = agent.invoke("open", params).await;
        parity::check(name, &outcome, *expect, &[], &mut problems);
        covered.push((*name, expect.ok(), expect.code()));
    }
    assert_eq!(
        env.opened()[ghe_opened..],
        [
            "https://github.com/octo/site/pull/12",
            "https://github.com/octo/site/pull/12/files?diff=split#top"
        ]
    );

    assert!(
        problems.is_empty(),
        "{}\n{}",
        problems.join("\n"),
        agent.log()
    );
    // The scenarios cover success and every failure code of the module.
    let codes: BTreeSet<_> = covered.iter().filter_map(|(_, _, c)| *c).collect();
    for code in [
        "invalid-repository",
        "invalid-ref",
        "unauthorized",
        "forbidden",
        "not-found",
        "rate-limited",
        "github-error",
        "invalid-response",
        "url-not-allowed",
        "invalid-parameters",
    ] {
        assert!(codes.contains(code), "no scenario ends in {code}");
    }

    // What went out: only GET to api.github.com, each with the token as a
    // bearer token, and only what the scenarios ask for. (The GitHub
    // Enterprise Server's requests are checked by `github_enterprise`.)
    let sent = env.sent()[ghe_sent..].to_vec();
    assert_ne!(sent.len(), 0);
    for line in &sent {
        let (url, auth) = line.split_once('\t').unwrap_or((line, ""));
        assert!(url.starts_with("https://api.github.com/"), "{line}");
        assert_eq!(auth, format!("authorization: Bearer {TOKEN}"), "{url}");
    }
    assert_eq!(parity::unrouted(&sent, &routes), Vec::<String>::new());

    network_failures(&agent).await;
    assert!(
        !agent.log().contains(TOKEN),
        "the token is in the agent's log"
    );
}

/// GitHub Enterprise Server in `apiUrl`, the plugin's user-origin slot:
/// nothing is sent there and nothing opened before the user allowed
/// exactly that server, a private one needs its own consent, and a
/// revoked consent refuses the very next call. Back on api.github.com,
/// no consent is needed. Returns how many pages it opened.
async fn github_enterprise(env: &parity::Env, agent: &parity::Agent) -> usize {
    let entered = settings(agent, json!("https://GHE.example.com:443/api/v3/")).await;
    let origin = &entered.origins[0];
    assert_eq!(origin.setting, "apiUrl");
    assert_eq!(origin.origin.as_deref(), Some(GHE_API), "{origin:?}");
    assert_eq!(origin.status, UserOriginStatus::NeedsConsent, "{origin:?}");
    assert_eq!(origin.credentials, ["token"]);
    assert!(origin.opens_pages && !origin.private_host, "{origin:?}");
    assert!(origin.private_network_allowed, "{origin:?}");

    // Entered, not allowed: no request, no token, no page.
    let before = env.sent();
    let refused = agent.invoke("current-user", &json!({})).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    let refused = agent
        .invoke("open", &json!({"repository": "octo/site"}))
        .await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    assert_eq!(env.sent(), before, "a request before the consent");
    assert_eq!(
        env.opened(),
        Vec::<String>::new(),
        "a page before the consent"
    );

    // The user allows exactly that server.
    agent
        .ok(Request::GrantPluginUserOrigin {
            plugin: ID.into(),
            setting: "apiUrl".into(),
            origin: GHE_API.into(),
            private_network: false,
        })
        .await;
    let user = agent.invoke("current-user", &json!({})).await;
    assert!(user.ok, "{user:?}\n{}", agent.log());
    assert_eq!(user.data["login"], "octocat");
    let new = env.sent()[before.len()..].to_vec();
    assert_eq!(
        new,
        [format!("{GHE_API}/user\tauthorization: Bearer {TOKEN}")],
        "only the allowed server, with the token"
    );
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    let page = agent
        .invoke(
            "open",
            &json!({"repository": "octo/site", "pullRequest": 12}),
        )
        .await;
    assert!(page.ok, "{page:?}");
    assert_eq!(env.opened(), ["https://ghe.example.com/octo/site/pull/12"]);
    // github.com is not that server's web host.
    let other = agent
        .invoke("open", &json!({"url": "https://github.com/octo/site"}))
        .await;
    assert_eq!(other.code.as_deref(), Some("url-not-allowed"), "{other:?}");

    // Revoked: refused on the very next call.
    agent
        .ok(Request::RevokePluginUserOrigin {
            plugin: ID.into(),
            setting: "apiUrl".into(),
        })
        .await;
    let before = env.sent();
    let refused = agent.invoke("current-user", &json!({})).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    assert_eq!(
        env.sent(),
        before,
        "a request after the consent was revoked"
    );

    // A server on a private network: allowing it needs the separate
    // private-network consent.
    let private = settings(agent, json!("https://10.0.0.5/api/v3")).await;
    assert!(private.origins[0].private_host, "{:?}", private.origins);
    let refused = agent
        .request(Request::GrantPluginUserOrigin {
            plugin: ID.into(),
            setting: "apiUrl".into(),
            origin: "https://10.0.0.5/api/v3".into(),
            private_network: false,
        })
        .await;
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.to_string().contains("private network")),
        "{refused:?}"
    );

    // Back to api.github.com: no consent of its own.
    let public = settings(agent, Value::Null).await;
    assert_eq!(public.effective["apiUrl"], json!(API));
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    1
}

/// No answer at all: a name that does not resolve and a certificate that
/// is not trusted end in the transport's codes, with its retryability
/// (`dns-failed` retryable, `tls-failed` not), and a message that does
/// not name the token.
async fn network_failures(agent: &parity::Agent) {
    for (repository, code) in NETWORK_FAILURES {
        let params = json!({"repository": repository});
        let outcome = agent.invoke_raw("repository", &params).await;
        eprintln!("network {repository}: {outcome:?}");
        let PluginActionOutcome::Failed {
            code: plugin_code,
            message,
            retryable,
        } = &outcome
        else {
            panic!("{repository}: {outcome:?}\n{}", agent.log());
        };
        assert_eq!(plugin_code, code, "{outcome:?}");
        assert_eq!(*retryable, *code == "dns-failed", "{outcome:?}");
        let message = message.as_deref().unwrap_or_default();
        assert!(!message.contains(TOKEN), "{message}");
        assert_ne!(message, "");
    }
}
