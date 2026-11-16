//! `deckpilot.jira` against the built-in `jira` module it replaces
//! (`docs/marketplace/module-migration.md` §9, P11), on the same Jira
//! answers (feature `plugins`):
//!
//! * the plugin (`plugins/first-party/jira`) is built to wasm with the
//!   `guest` profile and bundled as an installed application ships it;
//!   the real agent seeds it, the user consents to its permissions, enters
//!   the site, allows exactly that server and stores the credential, and
//!   every action runs in the real sandboxed host, invoked over IPC like
//!   the configuration app does;
//! * its HTTP goes to canned answers (`DECKPILOT_TEST_PLUGIN_HTTP`, feature
//!   `test-hooks`) and its opened pages to a file
//!   (`DECKPILOT_TEST_PLUGIN_OPEN_URL`): never a real Jira, never a
//!   browser;
//! * the built-in runs in this process against a stand-in Jira on
//!   127.0.0.1 (its site, allowed on a private host) that serves the same
//!   answers from the same function ([`answer`]), with a recording
//!   launcher. It runs first: the plugin's canned answers are exactly the
//!   requests the built-in sent, so a request the built-in does not make
//!   has no answer;
//! * every action's result is compared: the same status, message, outputs
//!   and error code, and the same requests in the same order. Where the
//!   two cannot agree the difference is listed in [`DIFFERENT_MESSAGES`]
//!   with its reason (and in `plugins/first-party/jira/README.md`);
//! * twice: Jira Cloud (REST API 3, Basic, token-paged search) and a Data
//!   Center under a path (REST API 2, Bearer, totals);
//! * Jira has no fixed host: nothing is sent, and no page opens, before
//!   the user allowed the server, after the site changed or after the
//!   consent was revoked;
//! * network failures (a name that does not resolve, a certificate that
//!   is not trusted) report the built-in's transport code on both sides.
//!
//! The plugin is pinned here as a `default`-class plugin and invoked by its
//! own action types: `migration`-class plugins are installed only by the
//! migration engine, and the legacy `jira.*` types reach the plugin only
//! after a computer's migration is cut over.
#![cfg(feature = "plugins")]

#[path = "support/https.rs"]
mod https;
#[path = "support/parity.rs"]
mod parity;
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use deckpilot_core::alias::{self, ActionAlias, ParamRule};
use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{ActionParameters, ModuleRegistry, Os, ParameterValue};
use deckpilot_ipc::plugins::{PluginActionOutcome, PluginSecret, PluginState, UserOriginStatus};
use deckpilot_ipc::{Request, Response};
use deckpilot_modules::jira;
use parity::{Answer, Outcome, Server, blocking, compare, json_answer, query};
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
/// What the user stores for Jira Cloud (`email:api-token`), for the plugin
/// and for the built-in.
const CLOUD_SECRET: &str = "dev@example.com:ATATT-parity-5e1d-not-a-real-token";
/// A Data Center personal access token.
const DC_SECRET: &str = "dc-pat-parity-93ab-not-a-real-token";
/// A Jira Cloud site, and a Data Center under a path: what the user
/// enters.
const CLOUD: &str = "https://team.atlassian.net";
const DC: &str = "https://jira.example.com/jira";
/// What the plugin answers may use of the IPC message limit.
const ANSWER_BUDGET: usize = deckpilot_ipc::framing::MAX_MESSAGE_BYTES / 2;

/// Failures whose message differs, by scenario, and why. The code is the
/// same.
const DIFFERENT_MESSAGES: &[(&str, &str)] = &[
    (
        "issue WEB-401",
        "the built-in names the credential id (`jira.token`); the plugin has a slot, not an id",
    ),
    (
        "myself without a credential",
        "the built-in names the credential id and Settings › HTTP › Credentials; the plugin \
         names its slot in Settings › Plugins › Jira",
    ),
    (
        "myself without a site",
        "each names where its site is set: Settings › Modules › Jira, Settings › Plugins › Jira",
    ),
];

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

/// The stand-in Jira: the answer to `GET <site><target>`, where `target`
/// is the path below the site with the query. The plugin's answers and
/// the built-in's (on 127.0.0.1) come from here, so both see the same
/// data. REST API 3 answers as Jira Cloud, 2 as a Data Center.
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

/// An action, its parameters, and a name for messages.
struct Scenario {
    name: &'static str,
    action: &'static str,
    params: Value,
}

fn scenario(name: &'static str, action: &'static str, params: Value) -> Scenario {
    Scenario {
        name,
        action,
        params,
    }
}

/// Every read action, on success and on each failure Jira can answer.
fn readings() -> Vec<Scenario> {
    let issue = |name, key: &str| scenario(name, "issue", json!({ "issue": key }));
    vec![
        scenario("myself", "myself", json!({})),
        issue("issue WEB-12", "web-12"),
        issue("issue OPS-1 (fallbacks)", " OPS-1 "),
        issue("issue not a key", "WEB"),
        issue("issue WEB-404", "WEB-404"),
        issue("issue WEB-403", "WEB-403"),
        issue("issue WEB-500", "WEB-500"),
        issue("issue WEB-429", "WEB-429"),
        issue("issue WEB-401", "WEB-401"),
        issue("issue WEB-200 (not JSON)", "WEB-200"),
        issue("issue WEB-201 (another shape)", "WEB-201"),
        scenario("issues assigned to you", "issues", json!({})),
        scenario(
            "issues of a project, 5 listed",
            "issues",
            json!({"project": "web", "limit": 5}),
        ),
        scenario(
            "issues of a project, 100 listed",
            "issues",
            json!({"project": "WEB", "limit": 100, "includeDone": true}),
        ),
        scenario(
            "issues of a saved filter",
            "issues",
            json!({"source": "filter", "filterId": 12345, "includeDone": true}),
        ),
        scenario(
            "issues of a filter with more than are counted",
            "issues",
            json!({"source": "filter", "filterId": 777}),
        ),
        scenario(
            "issues of a filter that does not exist",
            "issues",
            json!({"source": "filter", "filterId": 999}),
        ),
        scenario(
            "issues of a filter without its number",
            "issues",
            json!({"source": "filter"}),
        ),
        scenario(
            "issues of a project that is no key",
            "issues",
            json!({"project": "A\" OR 1=1"}),
        ),
    ]
}

/// Issues whose request gets no answer, and the transport's code: canned
/// for the plugin, real for the built-in (a name under `.invalid`, which
/// never resolves; a local HTTPS server whose throwaway CA the built-in
/// does not trust).
const NETWORK_FAILURES: &[(&str, &str)] = &[("NET-1", "dns-failed"), ("NET-2", "tls-failed")];

// ------------------------------------------------------------- built-in

/// The built-in module, configured with `config` and the launcher, as the
/// agent registers it.
fn builtin(config: &ActionParameters, launcher: &Arc<RecordingLauncher>) -> ModuleRegistry {
    let mut modules = ModuleRegistry::new(Os::current());
    modules
        .register(Box::new(jira::JiraModule::with_launcher(launcher.clone())))
        .unwrap();
    modules.set_config(&jira::module_id(), config).unwrap();
    modules
}

fn config(site: &str, deployment: &str) -> ActionParameters {
    ActionParameters::new()
        .with(jira::SITE_URL, ParameterValue::Text(site.to_owned()))
        .with(
            jira::DEPLOYMENT,
            ParameterValue::Choice(deployment.to_owned()),
        )
        .with(jira::ALLOW_PRIVATE, ParameterValue::Boolean(true))
}

fn run_builtin(
    modules: &ModuleRegistry,
    action: &str,
    params: &Value,
    secret: Option<&str>,
) -> Outcome {
    parity::run_builtin(
        modules,
        &format!("jira.{action}"),
        &parity::parameters(params, &["source"]),
        jira::DEFAULT_CREDENTIAL,
        secret,
    )
}

/// The aliases the cut-over adds (`crates/deckpilot-modules/tests/contracts.rs`).
fn jira_alias(action: &str) -> ActionAlias {
    let legacy: &'static str = Box::leak(format!("jira.{action}").into_boxed_str());
    let target: &'static str = Box::leak(format!("{ID}.{action}").into_boxed_str());
    ActionAlias {
        legacy,
        target,
        module: "jira",
        plugin: ID,
        since: "0.6.0",
        params: if action == "open" {
            &[]
        } else {
            &[ParamRule::DropIfDefault("credential")]
        },
    }
}

/// One Jira: the site the user enters for the plugin, the built-in's
/// stand-in for it, and what is asked of both.
struct Deployment {
    /// `cloud` or `data-center`.
    kind: &'static str,
    /// The plugin's site.
    site: &'static str,
    secret: &'static str,
    /// The `Authorization` header a request carries.
    authorization: String,
    server: Server,
    scenarios: Vec<Scenario>,
    /// What the built-in answered, in the order of `scenarios`.
    builtin: Vec<Outcome>,
}

impl Deployment {
    /// Starts the stand-in and runs every scenario on the built-in.
    fn start(
        kind: &'static str,
        site: &'static str,
        secret: &'static str,
        only: Option<&[&str]>,
        launcher: &Arc<RecordingLauncher>,
    ) -> Self {
        let prefix = site
            .strip_prefix("https://")
            .and_then(|rest| rest.find('/').map(|at| &rest[at..]))
            .unwrap_or("");
        let server = Server::start(prefix, answer, "authorization");
        let modules = builtin(&config(&server.base(), kind), launcher);
        let scenarios: Vec<Scenario> = readings()
            .into_iter()
            .filter(|s| only.is_none_or(|only| only.contains(&s.name)))
            .collect();
        let local = format!("{}/", server.base());
        let shown = format!("{site}/");
        let builtin = scenarios
            .iter()
            .map(|s| {
                blocking(|| run_builtin(&modules, s.action, &s.params, Some(secret)))
                    .replacing(&local, &shown)
            })
            .collect();
        let authorization = if kind == "cloud" {
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(secret)
            )
        } else {
            format!("Bearer {secret}")
        };
        Self {
            kind,
            site,
            secret,
            authorization,
            server,
            scenarios,
            builtin,
        }
    }

    /// What the built-in asked its stand-in for, in order.
    fn asked(&self) -> Vec<String> {
        self.server
            .seen()
            .iter()
            .map(|line| {
                let (target, authorization) = line.split_once('\t').unwrap();
                assert_eq!(authorization, self.authorization, "{target}");
                assert!(!target.starts_with("OUTSIDE"), "{line}");
                target.to_owned()
            })
            .collect()
    }

    /// The canned answers of the plugin's HTTP: exactly what the built-in
    /// asked for, at the plugin's site. Longest first (the canned
    /// transport takes the first route whose URL starts the request's).
    fn routes(&self) -> Vec<Value> {
        let mut targets = self.asked();
        targets.sort_by_key(|t| std::cmp::Reverse(t.len()));
        targets.dedup();
        targets
            .iter()
            .map(|target| {
                let a = answer(self.site, target);
                json!({"url": format!("{}{target}", self.site), "status": a.status,
                       "headers": a.headers, "body": a.body})
            })
            .collect()
    }

    /// The plugin sent exactly the built-in's requests, in order, each
    /// with the credential in this deployment's scheme, and nothing to
    /// another server.
    fn same_requests(&self, sent: &[String]) {
        let scheme = if self.kind == "cloud" {
            "Basic"
        } else {
            "Bearer"
        };
        let targets: Vec<String> = sent
            .iter()
            .map(|line| {
                let (url, credential) = line.split_once('\t').unwrap_or((line, ""));
                assert_eq!(
                    credential,
                    format!("authorization: {scheme} {}", self.secret),
                    "{url}"
                );
                url.strip_prefix(self.site)
                    .unwrap_or_else(|| panic!("{url} is not on {}", self.site))
                    .to_owned()
            })
            .collect();
        assert_eq!(targets, self.asked(), "{}", self.kind);
    }
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

/// What one comparison covered: scenario, whether it succeeded, its code.
type Compared = Vec<(&'static str, bool, Option<String>)>;

/// The plugin's side of a comparison.
struct Side<'a> {
    env: &'a parity::Env,
    agent: &'a parity::Agent,
    pace: &'a parity::Pace,
}

impl Side<'_> {
    /// Every scenario of `deployment` on the plugin, against what the
    /// built-in answered.
    async fn compare(&self, deployment: &Deployment, problems: &mut Vec<String>) -> Compared {
        let mut compared = Vec::new();
        for (s, builtin) in deployment.scenarios.iter().zip(&deployment.builtin) {
            self.pace.room_for(self.env, 8).await;
            let plugin = self.agent.invoke(s.action, &s.params).await;
            compare(s.name, builtin, &plugin, DIFFERENT_MESSAGES, problems);
            compared.push((s.name, builtin.ok, builtin.code.clone()));
        }
        compared
    }

    /// Open on both sides: the same page opened, shown without its
    /// query. (One page per 2 s for a plugin.)
    async fn compare_opens(
        &self,
        opens: &ModuleRegistry,
        pages: &[(&'static str, Value)],
        problems: &mut Vec<String>,
    ) -> Compared {
        let mut compared = Vec::new();
        for (name, params) in pages {
            tokio::time::sleep(Duration::from_millis(2_100)).await;
            let plugin = self.agent.invoke("open", params).await;
            let builtin = blocking(|| run_builtin(opens, "open", params, None));
            compare(name, &builtin, &plugin, DIFFERENT_MESSAGES, problems);
            compared.push((*name, builtin.ok, builtin.code));
        }
        compared
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

/// Every action of `deckpilot.jira` against the built-in, on the same
/// answers: identical results and requests, the documented differences
/// aside; on Jira Cloud and on a Data Center.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // the steps of one comparison, in order
async fn the_plugin_answers_as_the_builtin_does() {
    let launcher = Arc::new(RecordingLauncher::new());
    // The built-in first: what it asks for is what the plugin may ask for.
    let cloud = Deployment::start("cloud", CLOUD, CLOUD_SECRET, None, &launcher);
    let dc = Deployment::start(
        "data-center",
        DC,
        DC_SECRET,
        Some(ON_DATA_CENTER),
        &launcher,
    );
    assert_eq!(dc.scenarios.len(), ON_DATA_CENTER.len());
    let mut routes: Vec<Value> = NETWORK_FAILURES
        .iter()
        .map(|(issue, code)| {
            json!({"url": format!("{CLOUD}/rest/api/3/issue/{issue}?"), "status": 0,
                   "body": "", "fail": code})
        })
        .collect();
    routes.extend(cloud.routes());
    routes.extend(dc.routes());
    let env = parity::Env::new(PLUGIN, &routes);
    let agent = env.start().await;
    let pace = parity::Pace::start();
    let plugin_side = Side {
        env: &env,
        agent: &agent,
        pace: &pace,
    };
    let opens = builtin(
        &ActionParameters::new().with(jira::SITE_URL, ParameterValue::Text(CLOUD.into())),
        &launcher,
    );

    // Seeded, waiting for the user's consent to its permissions.
    let summary = match agent.request(Request::ListPlugins).await.unwrap() {
        Response::Plugins { plugins } => plugins
            .into_iter()
            .find(|p| p.id == ID)
            .unwrap_or_else(|| panic!("not seeded\n{}", agent.log())),
        other => panic!("unexpected {other:?}"),
    };
    assert_ne!(summary.state, PluginState::Enabled, "{}", agent.log());

    // The definitions as the agent's engine registers them: §5.3's rules
    // hold against the built-in's, the check the registry makes before it
    // routes a legacy type to the plugin.
    let registry = opens.action_registry();
    let mut problems = Vec::new();
    for action in &summary.actions {
        let definition = action.definition.as_ref().expect("every action converts");
        let alias = jira_alias(&action.action);
        let builtin = registry
            .definition(&alias.legacy_id())
            .unwrap_or_else(|| panic!("{} is not a built-in action", alias.legacy));
        for problem in alias::incompatibilities(&alias, builtin, definition) {
            problems.push(format!("{}: {problem}", alias.legacy));
        }
    }
    assert_eq!(summary.actions.len(), 4);
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

    // No site yet: neither side sends anything.
    let name = "myself without a site";
    let unset = builtin(&ActionParameters::new(), &launcher);
    let plugin = agent.invoke("myself", &json!({})).await;
    let without = blocking(|| run_builtin(&unset, "myself", &json!({}), Some(CLOUD_SECRET)));
    assert_eq!(plugin.code.as_deref(), Some("invalid-site-url"));
    compare(name, &without, &plugin, DIFFERENT_MESSAGES, &mut problems);

    // ---- Jira Cloud. The site entered, not allowed: nothing is sent, no
    // page opens; the plugin says where to allow it.
    plugin_side
        .enter(CLOUD, "https://Team.Atlassian.net/", "cloud")
        .await;
    plugin_side.refused("before the consent").await;
    let refused = agent.invoke("myself", &json!({})).await;
    assert_eq!(
        refused.message,
        "the Jira site team.atlassian.net is not allowed for this plugin; allow it in Settings \
         › Plugins › Jira › Configure"
    );
    // Allowed, no credential yet.
    plugin_side.allow(CLOUD).await;
    let name = "myself without a credential";
    let plugin = agent.invoke("myself", &json!({})).await;
    let cloud_reads = builtin(&config(&cloud.server.base(), "cloud"), &launcher);
    let without = blocking(|| run_builtin(&cloud_reads, "myself", &json!({}), None));
    assert_eq!(plugin.code.as_deref(), Some("credential-missing"));
    compare(name, &without, &plugin, DIFFERENT_MESSAGES, &mut problems);
    plugin_side.store(CLOUD_SECRET).await;
    assert_eq!(env.sent().len(), 0);

    let mut compared = plugin_side.compare(&cloud, &mut problems).await;
    let pages = [
        ("open an issue", json!({"issue": " web-12 "})),
        (
            "open a page",
            json!({"url": " https://Team.Atlassian.net/issues/?filter=12345#top "}),
        ),
        (
            "open another site",
            json!({"url": "https://other.atlassian.net/browse/WEB-12"}),
        ),
        ("open what is no issue", json!({"issue": "nope"})),
        ("open nothing", json!({})),
    ];
    compared.extend(
        plugin_side
            .compare_opens(&opens, &pages, &mut problems)
            .await,
    );
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
    let codes: BTreeSet<_> = compared.iter().filter_map(|(_, _, c)| c.clone()).collect();
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
    assert!(compared.iter().filter(|(_, ok, _)| *ok).count() >= 9);
    let sent_to_cloud = env.sent();
    cloud.same_requests(&sent_to_cloud);

    pace.room_for(&env, 3).await;
    network_failures(&env, &agent).await;
    let sent_to_cloud = env.sent();

    // ---- a Data Center under a path: another site, another credential.
    // The changed site ends the consent at once.
    plugin_side.enter(DC, DC, "data-center").await;
    plugin_side.store(DC_SECRET).await;
    plugin_side.refused("after the site changed").await;
    plugin_side.allow(DC).await;
    let before = problems.len();
    plugin_side.compare(&dc, &mut problems).await;
    let dc_opens = builtin(
        &ActionParameters::new().with(jira::SITE_URL, ParameterValue::Text(DC.into())),
        &launcher,
    );
    let pages = [
        (
            "open an issue of the Data Center",
            json!({"issue": "OPS-7"}),
        ),
        (
            "open a page beside the Data Center",
            json!({"url": "https://jira.example.com/confluence/Home"}),
        ),
        (
            "open the Cloud site from the Data Center",
            json!({"url": "https://team.atlassian.net/browse/WEB-12"}),
        ),
    ];
    plugin_side
        .compare_opens(&dc_opens, &pages, &mut problems)
        .await;
    assert!(
        problems.len() == before,
        "{}\n{}",
        problems[before..].join("\n"),
        agent.log()
    );
    assert_eq!(
        env.opened()[on_cloud.len()..],
        ["https://jira.example.com/jira/browse/OPS-7"]
    );
    let sent = env.sent();
    dc.same_requests(&sent[sent_to_cloud.len()..]);
    // Everything both sides opened, in order: the same pages.
    let started: Vec<String> = launcher
        .plans()
        .iter()
        .map(|p| p.args.last().unwrap().clone())
        .collect();
    assert_eq!(env.opened(), started);

    // The consent revoked: refused on the very next call.
    agent
        .ok(Request::RevokePluginUserOrigin {
            plugin: ID.into(),
            setting: "siteUrl".into(),
        })
        .await;
    plugin_side.refused("after the consent was revoked").await;

    // Neither secret is in a log line.
    let log = agent.log();
    for secret in [CLOUD_SECRET, DC_SECRET] {
        assert!(!log.contains(secret), "a secret is in the agent's log");
        let token = secret.rsplit(':').next().unwrap();
        assert!(!log.contains(token), "a token is in the agent's log");
    }
}

/// No answer at all: a name that does not resolve and a certificate that
/// is not trusted end in the built-in's transport codes on both sides,
/// with the transport's retryability for the plugin (`dns-failed`
/// retryable, `tls-failed` not). The messages are each side's
/// explanation of the same failure; neither names the secret.
async fn network_failures(env: &parity::Env, agent: &parity::Agent) {
    let launcher = Arc::new(RecordingLauncher::new());
    let root = env.dir.path().join("https-root");
    std::fs::create_dir_all(&root).unwrap();
    let untrusted = https::HttpsServer::start(&root);
    let unresolved = builtin(&config("https://jira.invalid", "cloud"), &launcher);
    let untrusted_site = builtin(
        &config(&format!("https://127.0.0.1:{}", untrusted.port), "cloud"),
        &launcher,
    );
    for (issue, code) in NETWORK_FAILURES {
        let params = json!({ "issue": issue });
        let modules = if *code == "dns-failed" {
            &unresolved
        } else {
            &untrusted_site
        };
        let builtin = blocking(|| run_builtin(modules, "issue", &params, Some(CLOUD_SECRET)));
        assert_eq!(
            builtin.code.as_deref(),
            Some(*code),
            "the built-in: {builtin:?}"
        );
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
        for text in [message.as_deref().unwrap_or_default(), &builtin.message] {
            assert!(!text.contains("ATATT"), "{text}");
            assert_ne!(text, "");
        }
    }
}
