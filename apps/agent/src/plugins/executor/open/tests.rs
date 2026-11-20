//! `resource.open` and `url.openParam`: only bound applications and
//! handles of this invocation, only runs the user started, only literal
//! http and https URLs of the action's own parameters, the grants, the
//! per-invocation limits and the audit. Launches are recorded, never run.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{MachineResourceBinding, ResourceKey, ResourceKind as Core};
use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::{RunCause, RunContext, RunTrigger};
use deckpilot_plugin_abi::privileged::{ResourceKind, ResourceOpen};
use deckpilot_plugin_broker::{Allowed, Broker, Decision, ManualClock, PluginId};
use deckpilot_plugin_supervisor::CapabilityExecutor;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::super::resources::{InvocationResources, ParamResource, ProjectResources};
use super::super::{
    Executor, Opener, OwnVariables, PrivilegedAudit, PrivilegedEntry, PublishedVariables, Transport,
};
use super::*;

const PLUGIN: &str = "deckpilot.workspace";
const OTHER: &str = "deckpilot.git";
const GRANTS: &str = r#"[{"kind":"resources","open":true},{"kind":"openUrlFromParams"}]"#;

/// `repo/` with `src/main.rs`, and `notes.md`; removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-open-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("repo/src")).unwrap();
        std::fs::write(dir.join("repo/src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("notes.md"), "notes").unwrap();
        Self(dir)
    }

    fn path(&self, relative: &str) -> String {
        self.0.join(relative).display().to_string()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn binding(key: &str, kind: Core, value: &str) -> ParamResource {
    ParamResource {
        via_role: false,
        binding: Ok(MachineResourceBinding {
            key: ResourceKey::new(key).unwrap(),
            kind,
            value: value.into(),
        }),
    }
}

type Records = Arc<Mutex<Vec<PrivilegedEntry>>>;

struct Fixture {
    broker: Broker,
    clock: Arc<ManualClock>,
    launcher: Arc<RecordingLauncher>,
    records: Records,
    tree: Tree,
}

impl Fixture {
    fn new(grants: &str) -> Self {
        let clock = Arc::new(ManualClock::default());
        let broker = Broker::new(
            Arc::new(deckpilot_plugin_broker::MemoryAudit::new(100)),
            clock.clone(),
            deckpilot_plugin_broker::RateLimits::default(),
        );
        for plugin in [PLUGIN, OTHER] {
            broker
                .set_grants(
                    &PluginId::new(plugin).unwrap(),
                    &deckpilot_plugin_broker::ApprovedPermissions::from_json(grants).unwrap(),
                )
                .unwrap();
        }
        Self {
            broker,
            clock,
            launcher: Arc::new(RecordingLauncher::new()),
            records: Arc::new(Mutex::new(Vec::new())),
            tree: Tree::new(),
        }
    }

    fn granted() -> Self {
        Self::new(GRANTS)
    }

    /// What the engine resolved: `repository` (a folder), `file`, `editor`
    /// (an application), and the text parameter `urls`.
    fn resolved(&self, urls: &str) -> InvocationResources {
        InvocationResources {
            params: BTreeMap::from([
                (
                    "repository".to_owned(),
                    binding("shop.repository", Core::Directory, &self.tree.path("repo")),
                ),
                (
                    "file".to_owned(),
                    binding("shop.notes", Core::Other, &self.tree.path("notes.md")),
                ),
                (
                    "editor".to_owned(),
                    binding("code", Core::Application, "/usr/bin/code"),
                ),
            ]),
            urls: BTreeMap::from([(
                "urls".to_owned(),
                UrlParam {
                    value: urls.to_owned(),
                    literal: true,
                },
            )]),
            project: Some(ProjectResources {
                id: "p-run".into(),
                roles: [
                    ("docs", "https://docs.example.com/shop?x=1"),
                    ("admin", "http://192.168.1.10:8080/"),
                    ("tricky", "https://bank.example@evil.example/"),
                    ("script", "javascript:alert(1)"),
                ]
                .into_iter()
                .map(|(role, value)| {
                    let binding = binding(&format!("shop.{role}"), Core::Url, value).binding;
                    (role.to_owned(), binding)
                })
                .collect(),
            }),
            ..InvocationResources::default()
        }
    }

    /// An invocation of `plugin` in `run`, and its handles by parameter.
    fn invocation(
        &self,
        plugin: &str,
        run: RunContext,
        urls: &str,
    ) -> (Executor, BTreeMap<String, String>) {
        self.invocation_with(plugin, run, self.resolved(urls))
    }

    /// An invocation of `plugin` in `run` with `resolved`.
    fn invocation_with(
        &self,
        plugin: &str,
        run: RunContext,
        resolved: InvocationResources,
    ) -> (Executor, BTreeMap<String, String>) {
        let (events, _) = broadcast::channel(4);
        let sink = self.records.clone();
        let audit: PrivilegedAudit = Arc::new(move |entry: &PrivilegedEntry| {
            sink.lock().unwrap().push(entry.clone());
        });
        let declared = BTreeMap::from([
            ("repository".to_owned(), vec![ResourceKind::Directory]),
            ("file".to_owned(), vec![ResourceKind::File]),
            ("editor".to_owned(), vec![ResourceKind::Application]),
        ]);
        let executor = Executor::new(
            self.broker.clone(),
            "Workspace".into(),
            events,
            Transport::Network,
            OwnVariables {
                store: PublishedVariables::default(),
                namespace: "plugins.deckpilot.workspace".into(),
                generation: 0,
                declared: Vec::new(),
            },
            Opener::System,
        )
        .with_audit(audit)
        .with_launcher(Os::Linux, self.launcher.clone())
        .with_run(run);
        executor.with_resources(plugin, "1.0.0", declared, resolved)
    }

    fn key(&self) -> (Executor, BTreeMap<String, String>) {
        self.invocation(PLUGIN, RunContext::direct(RunTrigger::Key), "")
    }

    /// What the supervisor does with one call of `plugin`, a while after
    /// the last one (so the broker's rates never interfere).
    fn call(&self, executor: &mut Executor, plugin: &str, request: &Value) -> CallResult {
        self.clock.advance(Duration::from_secs(60));
        let plugin = PluginId::new(plugin).unwrap();
        let request: CapabilityRequest = serde_json::from_value(request.clone()).unwrap();
        let session = self.broker.session(plugin.clone());
        match session.authorize_in(&request, executor.run_context()) {
            Decision::Allow(allowed) => executor.execute(&plugin, &allowed),
            Decision::Deny(reason) => {
                CallResult::error(reason.call_error_code(), reason.to_string())
            }
        }
    }

    fn plans(&self) -> Vec<(String, Vec<String>)> {
        self.launcher
            .plans()
            .into_iter()
            .map(|plan| (plan.program, plan.args))
            .collect()
    }

    fn records(&self) -> Vec<PrivilegedEntry> {
        self.records.lock().unwrap().clone()
    }
}

fn open(args: &Value) -> Value {
    json!({"op": "resource.open", "args": args})
}

fn url(param: &str, index: Option<u32>) -> Value {
    match index {
        Some(index) => json!({"op": "url.openParam", "args": {"param": param, "index": index}}),
        None => json!({"op": "url.openParam", "args": {"param": param}}),
    }
}

/// The ABI code and the reason code of a refusal.
fn refused(result: &CallResult) -> (CallErrorCode, String) {
    match result {
        CallResult::Error(e) => (
            e.code,
            e.message.split(':').next().unwrap_or_default().to_owned(),
        ),
        CallResult::Ok(value) => panic!("carried out: {value}"),
    }
}

fn ok(result: &CallResult) {
    assert!(matches!(result, CallResult::Ok(Value::Null)), "{result:?}");
}

/// A bound folder or file opens in the bound editor with the editor's
/// argument plan, or with the system's handler; the record names labels
/// and the plan, never a path.
#[test]
fn bound_locations_open_in_bound_applications_with_the_editor_plans() {
    let fixture = Fixture::granted();
    let repo = fixture.tree.path("repo");
    let notes = fixture.tree.path("notes.md");
    for (editor, expected) in [
        ("vscode", vec!["--goto".to_owned(), format!("{repo}:3:4")]),
        ("cursor", vec!["--goto".to_owned(), format!("{repo}:3:4")]),
        (
            "jetbrains",
            vec![
                "--line".to_owned(),
                "3".to_owned(),
                "--column".to_owned(),
                "4".to_owned(),
                repo.clone(),
            ],
        ),
        ("zed", vec![format!("{repo}:3:4")]),
        ("sublime", vec![format!("{repo}:3:4")]),
        ("other", vec![repo.clone()]),
    ] {
        let (mut executor, handles) = fixture.key();
        let result = fixture.call(
            &mut executor,
            PLUGIN,
            &open(&json!({
                "handle": handles["repository"],
                "with": handles["editor"],
                "position": {"line": 3, "column": 4},
                "editor": editor,
            })),
        );
        ok(&result);
        assert_eq!(
            fixture.plans().last().unwrap(),
            &("/usr/bin/code".to_owned(), expected),
            "{editor}"
        );
    }
    // Without an application: the system's handler; a bound file too.
    let (mut executor, handles) = fixture.key();
    ok(&fixture.call(
        &mut executor,
        PLUGIN,
        &open(&json!({"handle": handles["repository"]})),
    ));
    ok(&fixture.call(
        &mut executor,
        PLUGIN,
        &open(&json!({"handle": handles["file"], "with": handles["editor"]})),
    ));
    let plans = fixture.plans();
    assert_eq!(
        plans[plans.len() - 2..],
        [
            ("xdg-open".to_owned(), vec![repo.clone()]),
            ("/usr/bin/code".to_owned(), vec![notes.clone()]),
        ]
    );

    let records = fixture.records();
    assert_eq!(records.len(), 8);
    let last = records.last().unwrap();
    assert_eq!(last.record.op, "resource.open");
    assert_eq!(last.record.grant, "resources:open");
    assert_eq!(last.record.outcome, None);
    assert_eq!(last.origin, "key");
    assert_eq!(last.version, "1.0.0");
    assert_eq!(
        last.record.details,
        [
            ("label", "shop.notes".to_owned()),
            ("with", "code".to_owned()),
            ("editor", "other".to_owned()),
            ("located", "false".to_owned()),
        ]
    );
    for entry in &records {
        for (_, value) in &entry.record.details {
            assert!(
                !value.contains(&*fixture.tree.0.to_string_lossy()),
                "{value}"
            );
        }
    }
}

/// Only an application the user bound, as a handle of this invocation of
/// this plugin, starts; a folder, a forged handle, another invocation's or
/// another plugin's handle, or an application as the location never does.
#[test]
fn only_bound_applications_and_this_invocations_handles_open() {
    let fixture = Fixture::granted();
    let (mut executor, handles) = fixture.key();
    let (_, earlier) = fixture.key();
    let (_, others) = fixture.invocation(OTHER, RunContext::direct(RunTrigger::Key), "");
    let forged = "0123456789abcdef0123456789abcdef";
    for (args, code) in [
        (
            json!({"handle": handles["repository"], "with": handles["repository"]}),
            "resource-kind-mismatch",
        ),
        (
            json!({"handle": handles["repository"], "with": handles["file"]}),
            "resource-kind-mismatch",
        ),
        (
            json!({"handle": handles["editor"], "with": handles["editor"]}),
            "resource-kind-mismatch",
        ),
        (
            json!({"handle": handles["editor"]}),
            "resource-kind-mismatch",
        ),
        (
            json!({"handle": handles["repository"], "with": forged}),
            "not-permitted",
        ),
        (json!({"handle": forged}), "not-permitted"),
        (json!({"handle": earlier["repository"]}), "not-permitted"),
        (
            json!({"handle": handles["repository"], "with": earlier["editor"]}),
            "not-permitted",
        ),
        (json!({"handle": others["repository"]}), "not-permitted"),
    ] {
        let result = fixture.call(&mut executor, PLUGIN, &open(&args));
        assert_eq!(refused(&result).1, code, "{args}");
    }
    // The other plugin cannot use this invocation's handles either.
    let (mut theirs, _) = fixture.invocation(OTHER, RunContext::direct(RunTrigger::Key), "");
    let result = fixture.call(
        &mut theirs,
        OTHER,
        &open(&json!({"handle": handles["repository"]})),
    );
    assert_eq!(refused(&result).1, "not-permitted");
    assert!(fixture.plans().is_empty(), "a refused open launched");
    assert!(
        fixture.records().iter().all(|e| e.record.outcome.is_some()),
        "every refusal is audited as one"
    );
}

/// A location that is gone, or an application that would read as an
/// option, does not open.
#[test]
fn locations_must_still_exist_and_applications_must_be_programs() {
    let fixture = Fixture::granted();
    let (mut executor, handles) = fixture.key();
    std::fs::remove_file(fixture.tree.0.join("notes.md")).unwrap();
    let result = fixture.call(
        &mut executor,
        PLUGIN,
        &open(&json!({"handle": handles["file"]})),
    );
    assert_eq!(refused(&result).1, "not-found");
    std::fs::remove_dir_all(fixture.tree.0.join("repo")).unwrap();
    let result = fixture.call(
        &mut executor,
        PLUGIN,
        &open(&json!({"handle": handles["repository"]})),
    );
    assert_eq!(refused(&result).1, "not-found");

    let fixture = Fixture::granted();
    let mut resolved = fixture.resolved("");
    resolved.params.insert(
        "editor".into(),
        binding("evil", Core::Application, "--inspect"),
    );
    let (executor, _) = fixture.key();
    let declared = BTreeMap::from([
        ("repository".to_owned(), vec![ResourceKind::Directory]),
        ("editor".to_owned(), vec![ResourceKind::Application]),
    ]);
    let (mut executor, handles) = executor.with_resources(PLUGIN, "1.0.0", declared, resolved);
    let result = fixture.call(
        &mut executor,
        PLUGIN,
        &open(&json!({"handle": handles["repository"], "with": handles["editor"]})),
    );
    assert_eq!(refused(&result).1, "invalid-application");
    assert!(fixture.plans().is_empty(), "nothing is launched");
}

/// Neither operation opens anything in a run the user did not start
/// (schedules, events, the agent's start, work sessions, state calls and
/// unknown runs), by the broker's decision and again by the executor's.
#[test]
fn nothing_opens_in_runs_the_user_did_not_start() {
    let fixture = Fixture::granted();
    let runs = [
        RunContext::workflow(RunCause::Schedule, None),
        RunContext::workflow(RunCause::Event, None),
        RunContext::workflow(RunCause::Startup, None),
        RunContext::workflow(RunCause::WorkSession, None),
        // State calls and simulated presses run as unknown.
        RunContext::unknown(),
        RunContext::direct(RunTrigger::Other),
    ];
    for run in runs {
        let (mut executor, handles) =
            fixture.invocation(PLUGIN, run.clone(), "https://example.com/");
        for request in [
            open(&json!({"handle": handles["repository"], "with": handles["editor"]})),
            url("urls", None),
        ] {
            let result = fixture.call(&mut executor, PLUGIN, &request);
            assert_eq!(
                refused(&result).0,
                CallErrorCode::NotPermitted,
                "{run:?} {request}"
            );
        }
        // The executor refuses on its own, whatever reaches it.
        let plugin = PluginId::new(PLUGIN).unwrap();
        let allowed = Allowed::ResourceOpen {
            request: ResourceOpen {
                handle: handles["repository"].clone(),
                with: None,
                position: None,
                editor: None,
            },
        };
        assert_eq!(
            refused(&executor.execute(&plugin, &allowed)).1,
            "not-user-initiated"
        );
        let allowed = Allowed::OpenUrlParam {
            request: OpenUrlParam {
                param: "urls".into(),
                index: None,
            },
        };
        assert_eq!(
            refused(&executor.execute(&plugin, &allowed)).1,
            "not-user-initiated"
        );
    }
    assert!(fixture.plans().is_empty(), "a page or program opened");
}

/// Without `resources:open` or `openUrl:fromParams` the broker refuses,
/// and nothing reaches a launcher.
#[test]
fn without_the_grant_nothing_opens() {
    for (grants, op) in [
        (r#"[{"kind":"resources"}]"#, "resource.open"),
        (r#"[{"kind":"resources","paths":true}]"#, "resource.open"),
        (r#"[{"kind":"resources","open":true}]"#, "url.openParam"),
        ("[]", "resource.open"),
        ("[]", "url.openParam"),
    ] {
        let fixture = Fixture::new(grants);
        let (mut executor, handles) = fixture.invocation(
            PLUGIN,
            RunContext::direct(RunTrigger::Key),
            "https://example.com/",
        );
        let request = if op == "resource.open" {
            open(&json!({"handle": handles["repository"]}))
        } else {
            url("urls", None)
        };
        let result = fixture.call(&mut executor, PLUGIN, &request);
        assert_eq!(
            refused(&result).0,
            CallErrorCode::NotPermitted,
            "{grants} {op}"
        );
        assert!(fixture.plans().is_empty(), "{grants} {op}");
    }
}

/// At most 5 resources and 10 URLs per invocation, whatever the broker's
/// rates allow.
#[test]
fn an_invocation_opens_at_most_a_few_resources_and_urls() {
    let fixture = Fixture::granted();
    let urls = (0..12)
        .map(|i| format!("https://example.com/{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut executor, handles) =
        fixture.invocation(PLUGIN, RunContext::direct(RunTrigger::Key), &urls);
    for _ in 0..MAX_RESOURCE_OPENS {
        ok(&fixture.call(
            &mut executor,
            PLUGIN,
            &open(&json!({"handle": handles["repository"]})),
        ));
    }
    let result = fixture.call(
        &mut executor,
        PLUGIN,
        &open(&json!({"handle": handles["repository"]})),
    );
    assert_eq!(
        refused(&result),
        (CallErrorCode::Throttled, "throttled".into())
    );
    for i in 0..MAX_PARAM_URLS {
        ok(&fixture.call(&mut executor, PLUGIN, &url("urls", Some(i))));
    }
    let result = fixture.call(&mut executor, PLUGIN, &url("urls", Some(0)));
    assert_eq!(
        refused(&result),
        (CallErrorCode::Throttled, "throttled".into())
    );
    assert_eq!(fixture.plans().len(), 15);
}

/// The literal http and https URLs of the action's own parameter open,
/// private and local hosts included; the record holds the origin only.
#[test]
fn literal_web_urls_of_the_actions_parameter_open() {
    let fixture = Fixture::granted();
    let urls = [
        "https://example.com/a/b?token=1#x",
        "  http://localhost:3000/  ",
        "http://192.168.1.10:8080/admin",
        "https://[::1]:8443/",
        "HTTPS://Example.COM/Path",
        "https://xn--bcher-kva.example/",
        "https://example.com.",
    ];
    let (mut executor, _) = fixture.invocation(
        PLUGIN,
        RunContext::direct(RunTrigger::Gesture),
        &urls.join("\n\n"),
    );
    for i in 0..u32::try_from(urls.len()).unwrap() {
        ok(&fixture.call(&mut executor, PLUGIN, &url("urls", Some(i))));
    }
    let opened: Vec<String> = fixture
        .plans()
        .into_iter()
        .map(|(program, mut args)| {
            assert_eq!(program, "xdg-open");
            args.pop().unwrap()
        })
        .collect();
    assert_eq!(
        opened,
        [
            "https://example.com/a/b?token=1#x",
            "http://localhost:3000/",
            "http://192.168.1.10:8080/admin",
            "https://[::1]:8443/",
            "https://example.com/Path",
            "https://xn--bcher-kva.example/",
            "https://example.com./",
        ]
    );
    let first = &fixture.records()[0];
    assert_eq!(first.record.op, "url.openParam");
    assert_eq!(first.record.grant, "openUrl:from-params");
    assert_eq!(first.origin, "gesture");
    assert_eq!(
        first.record.details,
        [
            ("param", "urls".to_owned()),
            ("index", "0".to_owned()),
            ("origin", "https://example.com".to_owned()),
        ]
    );
}

/// Every other scheme, userinfo, and a host that is not written literally
/// (IDN, percent-encoding, numeric shorthands, whitespace and backslash
/// tricks) is refused before a launcher sees it.
#[test]
fn only_literal_http_and_https_urls_open() {
    let fixture = Fixture::granted();
    let long = format!("https://example.com/{}", "a".repeat(2048));
    let bad = [
        "file:///etc/passwd",
        "data:text/html,<script>alert(1)</script>",
        "javascript:alert(1)",
        "mailto:team@example.com",
        "vscode://file/etc/passwd",
        "ftp://example.com/",
        "chrome://settings",
        "https:evil.example",
        "https:/evil.example",
        "//evil.example/",
        "example.com",
        "https://user:secret@example.com/",
        "https://bank.example@evil.example/",
        "https://@evil.example/",
        "https://user@evil.example/",
        "https://exa mple.com/",
        "https://example.com/a b",
        "https://example.com/\u{7}",
        "https://ex\tample.com/",
        "https://bücher.example/",
        "https://exаmple.com/",
        "https://example.com/\u{202e}gpj",
        "https://ex%61mple.com/",
        "http://2130706433/",
        "http://127.1/",
        "http://0x7f.0.0.1/",
        r"https:\\evil.example\",
        r"https://example.com\@evil.example/",
        "https://",
        "http:///path",
        long.as_str(),
    ];
    for value in bad {
        let (mut executor, _) =
            fixture.invocation(PLUGIN, RunContext::direct(RunTrigger::App), value);
        let result = fixture.call(&mut executor, PLUGIN, &url("urls", None));
        assert_eq!(
            refused(&result),
            (CallErrorCode::NotPermitted, "invalid-url".into()),
            "{value}"
        );
        // The refusal never repeats the value.
        let CallResult::Error(error) = result else {
            unreachable!()
        };
        assert!(!error.message.contains(value.trim()), "{value}");
    }
    assert!(fixture.plans().is_empty(), "a refused URL was opened");
}

/// Only the action's own text or list parameter, by an index it has; a
/// workflow run (whose values a mapping may have produced) is refused.
#[test]
fn only_the_actions_own_literal_parameter_opens() {
    let fixture = Fixture::granted();
    let (mut executor, _) = fixture.invocation(
        PLUGIN,
        RunContext::direct(RunTrigger::Key),
        "https://example.com/",
    );
    for (request, code) in [
        (url("urls", Some(1)), "url-param-index"),
        (url("pages", None), "url-param-missing"),
        // A resource parameter is no URL.
        (url("repository", None), "url-param-missing"),
    ] {
        let result = fixture.call(&mut executor, PLUGIN, &request);
        assert_eq!(refused(&result).1, code, "{request}");
    }
    // Another plugin does not see this invocation's values.
    let result = fixture.call(&mut executor, OTHER, &url("urls", None));
    assert_eq!(refused(&result).1, "url-param-missing");

    assert!(fixture.plans().is_empty(), "nothing is launched");

    // A default, a template, a variable or a workflow mapping produced
    // the value: refused, in a key's run and in a workflow's.
    for run in [
        RunContext::direct(RunTrigger::Key),
        RunContext::workflow(RunCause::User, None),
    ] {
        let mut resolved = fixture.resolved("https://example.com/");
        resolved.urls.get_mut("urls").unwrap().literal = false;
        let (mut executor, _) = fixture.invocation_with(PLUGIN, run.clone(), resolved);
        let result = fixture.call(&mut executor, PLUGIN, &url("urls", None));
        assert_eq!(refused(&result).1, "url-param-not-literal", "{run:?}");
    }
    assert!(fixture.plans().is_empty(), "nothing is launched");

    // A literal value in a workflow the user started opens.
    let (mut workflow, handles) = fixture.invocation(
        PLUGIN,
        RunContext::workflow(RunCause::User, None),
        "https://example.com/",
    );
    ok(&fixture.call(&mut workflow, PLUGIN, &url("urls", None)));
    ok(&fixture.call(
        &mut workflow,
        PLUGIN,
        &open(&json!({"handle": handles["repository"]})),
    ));
    assert_eq!(fixture.plans().len(), 2);
}

/// A project's URL role opens as a handle of kind `url`, in the default
/// browser only and under the URL rules; the guest and the record see its
/// label and origin, never the address.
#[test]
fn bound_web_addresses_open_in_the_default_browser_only() {
    let fixture = Fixture::granted();
    let (mut executor, handles) = fixture.key();
    let resolve = |executor: &mut Executor, role: &str| {
        let result = fixture.call(
            executor,
            PLUGIN,
            &json!({"op": "resource.resolve", "args": {"role": role, "kinds": ["url"]}}),
        );
        let CallResult::Ok(value) = result else {
            panic!("{role}: {result:?}")
        };
        assert!(!value.to_string().contains("example"), "{value}");
        assert_eq!(value["kind"], "url");
        value["handle"].as_str().unwrap().to_owned()
    };
    let docs = resolve(&mut executor, "docs");
    let admin = resolve(&mut executor, "admin");
    ok(&fixture.call(&mut executor, PLUGIN, &open(&json!({"handle": docs}))));
    ok(&fixture.call(&mut executor, PLUGIN, &open(&json!({"handle": admin}))));
    assert_eq!(
        fixture.plans(),
        [
            (
                "xdg-open".to_owned(),
                vec!["https://docs.example.com/shop?x=1".to_owned()]
            ),
            (
                "xdg-open".to_owned(),
                vec!["http://192.168.1.10:8080/".to_owned()]
            ),
        ]
    );
    let record = fixture
        .records()
        .into_iter()
        .rfind(|e| e.record.op == "resource.open")
        .unwrap();
    assert_eq!(
        record.record.details,
        [
            ("label", "shop.admin".to_owned()),
            ("with", "browser".to_owned()),
            ("origin", "http://192.168.1.10:8080".to_owned()),
        ]
    );

    // Never with an application, a position or an editor.
    for args in [
        json!({"handle": docs, "with": handles["editor"]}),
        json!({"handle": docs, "position": {"line": 1}}),
        json!({"handle": docs, "editor": "vscode"}),
    ] {
        let result = fixture.call(&mut executor, PLUGIN, &open(&args));
        assert_eq!(refused(&result).1, "bad-request", "{args}");
    }
    // A bound address that is no literal http or https URL.
    for role in ["tricky", "script"] {
        let handle = resolve(&mut executor, role);
        let result = fixture.call(&mut executor, PLUGIN, &open(&json!({"handle": handle})));
        assert_eq!(refused(&result).1, "invalid-url", "{role}");
    }
    // Not as an application, not as a location to open in one.
    let result = fixture.call(
        &mut executor,
        PLUGIN,
        &open(&json!({"handle": handles["repository"], "with": docs})),
    );
    assert_eq!(refused(&result).1, "resource-kind-mismatch");
    assert_eq!(fixture.plans().len(), 2, "a refused address was opened");

    // Only in runs the user started.
    let (mut scheduled, _) =
        fixture.invocation(PLUGIN, RunContext::workflow(RunCause::Schedule, None), "");
    let result = fixture.call(
        &mut scheduled,
        PLUGIN,
        &json!({"op": "resource.resolve", "args": {"role": "docs", "kinds": ["url"]}}),
    );
    let CallResult::Ok(value) = result else {
        panic!("{result:?}")
    };
    let result = fixture.call(
        &mut scheduled,
        PLUGIN,
        &open(&json!({"handle": value["handle"]})),
    );
    assert_eq!(refused(&result).0, CallErrorCode::NotPermitted);
    assert_eq!(fixture.plans().len(), 2);
}

/// The engine hands over the values of text and list parameters only.
#[test]
fn only_text_and_list_parameters_are_kept_for_urls() {
    use deckpilot_core::resolution::{Provenance, Provenances};
    use deckpilot_core::{ActionParameters, ParameterValue};
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/examples/hello-weather/manifest.json"
    ))
    .unwrap();
    let sample: deckpilot_plugin_manifest::Manifest = serde_json::from_str(&text).unwrap();
    let mut manifest = sample.contributes.actions()[0].clone();
    manifest.parameters = serde_json::from_value(json!([
        {"key": "urls", "label": "URLs", "required": false, "kind": {"type": "lines"}},
        {"key": "page", "label": "Page", "required": false, "kind": {"type": "text"}},
        {"key": "mode", "label": "Mode", "required": false,
         "kind": {"type": "choice", "options": ["a", "https://example.com/"]}},
        {"key": "count", "label": "Count", "required": false, "kind": {"type": "integer"}}
    ]))
    .unwrap();
    let definition = super::super::super::engine::action_type(PLUGIN, &manifest).unwrap();
    let parameters = ActionParameters::new()
        .with("urls", ParameterValue::Text("https://a.example/".into()))
        .with("page", ParameterValue::Text("https://b.example/".into()))
        .with(
            "mode",
            ParameterValue::Choice("https://example.com/".into()),
        )
        .with("count", ParameterValue::Integer(2));
    let provenance = Provenances::from_entries([
        ("urls".to_owned(), Provenance::Literal),
        ("page".to_owned(), Provenance::Template),
        ("mode".to_owned(), Provenance::Literal),
    ]);
    let param = |value: &str, literal| UrlParam {
        value: value.into(),
        literal,
    };
    let resources =
        InvocationResources::default().with_urls(&definition, &parameters, Some(&provenance));
    assert_eq!(
        resources.urls,
        BTreeMap::from([
            ("page".to_owned(), param("https://b.example/", false)),
            ("urls".to_owned(), param("https://a.example/", true)),
        ])
    );
    // Without a provenance nothing is literal.
    let unknown = InvocationResources::default().with_urls(&definition, &parameters, None);
    assert!(unknown.urls.values().all(|p| !p.literal));
}
