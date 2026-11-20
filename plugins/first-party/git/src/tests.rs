//! Native unit tests: the definition agrees with manifest.json, the
//! manifest is valid against the schema, the translations are complete,
//! and every action behaves on recorded `git` output, with the agent
//! replaced by `MockHost`. The scenarios are the built-in's
//! (`modules/git/src/actions/tests.rs`, which runs real repositories),
//! here with the output real git printed for them. The end-to-end run
//! through the real agent, host and a fake `git` is
//! `apps/agent/tests/plugins_git.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};
use deckpilot_plugin_sdk::{
    ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, ParamValue, Params, StateOutcome,
};
use serde_json::{Value, json};

use super::*;

const MANIFEST: &str = include_str!("../manifest.json");
const SCHEMA: &str = include_str!("../../../schemas/plugin-manifest.schema.json");
const EN: &str = include_str!("../locales/en.json");
const PL: &str = include_str!("../locales/pl.json");

const OID: &str = "1234567890abcdef1234567890abcdef12345678";
const HANDLE: &str = "0123456789abcdef0123456789abcdef";
const APP: &str = "fedcba9876543210fedcba9876543210";
const SHOP: &str = "/work/shop";

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).unwrap()
}

/// NUL-terminated porcelain records, as `git status -z` prints them.
fn z(lines: &[&str]) -> String {
    let mut out = lines.join("\0");
    out.push('\0');
    out
}

/// One recorded run: exit status, stdout, stderr.
type Run = (i32, String, String);

fn ok(stdout: &str) -> Run {
    (0, stdout.to_owned(), String::new())
}

fn failed(code: i32, stderr: &str) -> Run {
    (code, String::new(), stderr.to_owned())
}

/// A stand-in agent: how the repository parameter and the project role
/// resolve, the active project, recorded git output by argument list,
/// storage, settings, opened resources.
#[derive(Debug)]
struct Agent {
    /// The `repository` parameter: unset (`None`), a binding, or a
    /// refusal (`<code>: <message>`).
    param: Option<Result<(&'static str, &'static str), &'static str>>,
    /// The repository role of the run's (or named) project.
    role: Result<(&'static str, &'static str), &'static str>,
    /// The active project: id and key.
    active: Option<(&'static str, &'static str)>,
    application: Option<&'static str>,
    git: BTreeMap<String, Run>,
    /// A refusal of every `tool.run` (`<code>: <message>`).
    tool_refusal: Option<(CallErrorCode, &'static str)>,
    settings: Value,
    runs: Vec<Vec<String>>,
    timeouts: Vec<Option<u32>>,
    resolves: Vec<deckpilot_plugin_sdk::ResourceResolve>,
    storage: BTreeMap<String, Value>,
    opened: Vec<ResourceOpen>,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            param: None,
            role: Ok(("shop.repository", SHOP)),
            active: Some(("p-shop", "SHOP")),
            application: None,
            git: BTreeMap::new(),
            tool_refusal: None,
            settings: json!({"repositoryRole": "repository", "networkTimeoutSeconds": 20}),
            runs: Vec::new(),
            timeouts: Vec::new(),
            resolves: Vec::new(),
            storage: BTreeMap::new(),
            opened: Vec::new(),
        }
    }
}

type Shared = Rc<RefCell<Agent>>;

fn agent() -> Shared {
    Rc::new(RefCell::new(Agent::default()))
}

fn handle(label: &str, path: &str, kind: &str, handle: &str) -> CallResult {
    CallResult::Ok(json!({"handle": handle, "kind": kind, "label": label, "displayPath": path}))
}

fn refusal(text: &str) -> CallResult {
    let code = if text.starts_with("not-permitted") {
        CallErrorCode::NotPermitted
    } else {
        CallErrorCode::Failed
    };
    CallResult::error(code, text)
}

fn host(agent: &Shared) -> MockHost {
    let agent = agent.clone();
    MockHost::new(move |request| {
        let mut agent = agent.borrow_mut();
        match request {
            CapabilityRequest::SettingsGet(_) => CallResult::Ok(agent.settings.clone()),
            CapabilityRequest::ResourceResolve(r) => {
                agent.resolves.push(r.clone());
                match (r.param.as_deref(), r.role.as_deref()) {
                    (Some("repository"), None) => match agent.param {
                        None => refusal("resource-not-set: the parameter `repository` is not set"),
                        Some(Ok((label, path))) => handle(label, path, "directory", HANDLE),
                        Some(Err(text)) => refusal(text),
                    },
                    (Some("application"), None) => match agent.application {
                        None => refusal("resource-not-set: the parameter `application` is not set"),
                        Some(label) => {
                            handle(label, "/Applications/Editor.app", "application", APP)
                        }
                    },
                    (None, Some("repository")) => match agent.role {
                        Ok((label, path)) => handle(label, path, "directory", HANDLE),
                        Err(text) => refusal(text),
                    },
                    other => panic!("unexpected resolve {other:?}"),
                }
            }
            CapabilityRequest::ProfileRead(_) => CallResult::Ok(match agent.active {
                Some((id, key)) => json!({"project": {"id": id, "key": key, "name": key}}),
                None => json!({}),
            }),
            CapabilityRequest::ToolRun(run) => {
                assert_eq!(run.tool, Tool::Git);
                assert_eq!(run.cwd, HANDLE, "git runs only in the repository's handle");
                agent.runs.push(run.args.clone());
                agent.timeouts.push(run.timeout_ms);
                if let Some((code, text)) = agent.tool_refusal {
                    return CallResult::error(code, text);
                }
                let key = run.args.join(" ");
                let Some((exit, stdout, stderr)) = agent.git.get(&key).cloned() else {
                    panic!("no recorded output for `git {key}`")
                };
                CallResult::Ok(json!({"exit": exit, "stdout": stdout, "stderr": stderr}))
            }
            CapabilityRequest::StorageSet(s) => {
                agent.storage.insert(s.key.clone(), s.value.clone());
                CallResult::Ok(Value::Null)
            }
            CapabilityRequest::StorageGet(s) => {
                CallResult::Ok(agent.storage.get(&s.key).cloned().unwrap_or(Value::Null))
            }
            CapabilityRequest::ResourceOpen(o) => {
                agent.opened.push(o.clone());
                CallResult::Ok(Value::Null)
            }
            CapabilityRequest::Log(_) => CallResult::Ok(Value::Null),
            other => panic!("unexpected request {other:?}"),
        }
    })
}

fn params(value: Value) -> Params {
    serde_json::from_value(value).unwrap()
}

/// Runs `action`; its message and outputs as text, or its code and
/// message.
fn run(
    agent: &Shared,
    action: &str,
    args: Value,
) -> Result<(String, BTreeMap<String, String>), (String, String)> {
    let mut host = host(agent);
    match host.invoke_params(&plugin(), action, &params(args)) {
        ActionOutcome::Ok { message, data } => Ok((
            message.unwrap_or_default(),
            data.into_iter()
                .map(|(k, v)| {
                    let text = match v {
                        ParamValue::String(s) => s,
                        ParamValue::Bool(b) => b.to_string(),
                        ParamValue::Integer(i) => i.to_string(),
                        other => format!("{other:?}"),
                    };
                    (k, text)
                })
                .collect(),
        )),
        ActionOutcome::Failed { code, message, .. } => Err((code, message.unwrap_or_default())),
        ActionOutcome::Skipped { .. } => panic!("skipped"),
    }
}

fn status_args(untracked: &str) -> String {
    format!("status --porcelain=v2 --branch -z --untracked-files={untracked}")
}

fn record(agent: &Shared, args: &str, run: Run) {
    agent.borrow_mut().git.insert(args.to_owned(), run);
}

/// `git status` of a repository with commits on `main`, tracking
/// `origin/main`.
fn tracking_main(ahead: u32, behind: u32, changes: &[&str]) -> String {
    let oid = format!("# branch.oid {OID}");
    let ab = format!("# branch.ab +{ahead} -{behind}");
    let mut lines = vec![
        oid.as_str(),
        "# branch.head main",
        "# branch.upstream origin/main",
        ab.as_str(),
    ];
    lines.extend(changes);
    z(&lines)
}

// --------------------------------------------------------- the contract

#[test]
fn the_definition_matches_the_manifest() {
    // `resource` parameters arrive as handles: text in the definition.
    let mismatches: Vec<String> = manifest_mismatches(&plugin(), &manifest())
        .into_iter()
        .filter(|m| {
            !m.ends_with("kind.type is \"resource\" in the manifest, \"text\" in the plugin")
        })
        .collect();
    assert!(mismatches.is_empty(), "{mismatches:#?}");
    let plugin = plugin();
    let ids: Vec<&str> = plugin.actions().iter().map(Action::id).collect();
    assert_eq!(
        ids,
        [
            "status",
            "branch",
            "changes",
            "last-commit",
            "remote-status",
            "open-repository"
        ]
    );
    for action in plugin.actions() {
        let expected = matches!(action.id(), "status" | "branch");
        assert_eq!(action.has_state(), expected, "{}", action.id());
    }
}

#[test]
fn the_manifest_is_valid_against_the_schema() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let manifest = manifest();
    let errors: Vec<String> = validator
        .iter_errors(&manifest)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// Every `t:` key of the manifest is in both locale files, and the two
/// have the same keys.
#[test]
fn every_text_is_translated() {
    fn keys(prefix: &str, value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    let next = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    keys(&next, v, out);
                }
            }
            Value::String(_) => out.push(prefix.to_owned()),
            _ => panic!("{prefix} is not text"),
        }
    }
    fn used(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => map.values().for_each(|v| used(v, out)),
            Value::Array(items) => items.iter().for_each(|v| used(v, out)),
            Value::String(s) => {
                if let Some(key) = s.strip_prefix("t:") {
                    out.push(key.to_owned());
                }
            }
            _ => {}
        }
    }
    let (mut en, mut pl, mut wanted) = (Vec::new(), Vec::new(), Vec::new());
    keys("", &serde_json::from_str(EN).unwrap(), &mut en);
    keys("", &serde_json::from_str(PL).unwrap(), &mut pl);
    used(&manifest(), &mut wanted);
    assert_eq!(en, pl);
    for key in wanted {
        assert!(en.contains(&key), "{key} is not translated");
    }
}

/// The tools policy declares only what the actions run; `ls-remote` is
/// the only network use and needs its sub-grant.
#[test]
fn the_tools_policy_is_what_the_actions_run() {
    let manifest = manifest();
    let tools = manifest["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "tools")
        .unwrap();
    assert_eq!(tools["tools"][0]["executable"], "git");
    assert_eq!(tools["tools"][0]["network"], true);
    let declared: Vec<(String, Vec<String>)> = tools["tools"][0]["subcommands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_owned(),
                s["options"]
                    .as_array()
                    .map(|o| o.iter().map(|o| o.as_str().unwrap().to_owned()).collect())
                    .unwrap_or_default(),
            )
        })
        .collect();
    let agent = agent();
    record(&agent, &status_args("all"), ok(&tracking_main(1, 0, &[])));
    record(&agent, &status_args("no"), ok(&tracking_main(1, 0, &[])));
    record(
        &agent,
        &format!("log -1 --no-show-signature {LOG_FORMAT}"),
        ok("a\0b\0c\0d\0e"),
    );
    record(&agent, "config --get branch.main.remote", ok("origin\n"));
    record(
        &agent,
        "config --get branch.main.merge",
        ok("refs/heads/main\n"),
    );
    record(&agent, "remote", ok("origin\n"));
    record(
        &agent,
        "rev-parse --verify -q origin/main",
        ok(&format!("{OID}\n")),
    );
    record(
        &agent,
        "ls-remote -- origin refs/heads/main",
        ok(&format!("{OID}\trefs/heads/main\n")),
    );
    for action in ["status", "branch", "changes", "last-commit"] {
        run(&agent, action, json!({})).unwrap();
    }
    run(&agent, "remote-status", json!({"refresh": true})).unwrap();
    let runs = agent.borrow().runs.clone();
    for args in &runs {
        let (sub, rest) = args.split_first().unwrap();
        let Some((_, options)) = declared.iter().find(|(name, _)| name == sub) else {
            panic!("`git {sub}` is not declared");
        };
        for option in rest
            .iter()
            .take_while(|a| *a != "--")
            .filter(|a| a.starts_with('-'))
        {
            let name = option.split_once('=').map_or(option.as_str(), |(n, _)| n);
            assert!(
                options.iter().any(|o| o == option || o == name),
                "`{option}` of `git {sub}` is not declared"
            );
        }
    }
    let used: std::collections::BTreeSet<&str> = runs.iter().map(|a| a[0].as_str()).collect();
    let declared: std::collections::BTreeSet<&str> =
        declared.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(used, declared, "every declared subcommand is used");
}

// -------------------------------------------------------------- actions

/// `git init` without a commit: `status` succeeds, `last-commit` is
/// `no-commits` (the built-in's `a_repository_without_commits`).
#[test]
fn a_repository_without_commits() {
    let agent = agent();
    let unborn = z(&["# branch.oid (initial)", "# branch.head main"]);
    record(&agent, &status_args("all"), ok(&unborn));
    record(&agent, &status_args("no"), ok(&unborn));
    let (message, data) = run(&agent, "status", json!({})).unwrap();
    assert_eq!(message, "main (no commits yet); clean");
    assert_eq!(data["branch"], "main");
    assert_eq!(data["unborn"], "true");
    assert_eq!(data["detached"], "false");
    assert_eq!(data["clean"], "true");
    assert_eq!(data["projectKey"], "SHOP");
    assert_eq!(data["repository"], SHOP);
    assert!(!data.contains_key("head"));
    let (code, message) = run(&agent, "last-commit", json!({})).unwrap_err();
    assert_eq!(code, codes::NO_COMMITS);
    assert_eq!(message, "main has no commits yet");
    // The role came from the settings, in the run's project.
    let resolves = agent.borrow().resolves.clone();
    assert_eq!(resolves[1].role.as_deref(), Some("repository"));
    assert_eq!(resolves[1].kinds, [ResourceKind::Directory]);
}

/// The built-in's `status_changes_and_the_last_commit`: a staged, an
/// unstaged and an untracked file.
#[test]
fn status_changes_and_the_last_commit() {
    let agent = agent();
    let output = z(&[
        &format!("# branch.oid {OID}"),
        "# branch.head main",
        "1 .M N... 100644 100644 100644 aaaa aaaa a.txt",
        "1 M. N... 100644 100644 100644 bbbb cccc b.txt",
        "? new file.txt",
    ]);
    record(&agent, &status_args("all"), ok(&output));
    record(&agent, &status_args("no"), ok(&output));
    record(
        &agent,
        &format!("log -1 --no-show-signature {LOG_FORMAT}"),
        ok(&format!(
            "{OID}\0{}\0Test\x002026-11-20T10:00:00+01:00\0Add b",
            &OID[..7]
        )),
    );
    let (message, status) = run(&agent, "status", json!({})).unwrap();
    assert_eq!(message, "main; 3 changed");
    assert_eq!(status["clean"], "false");
    assert_eq!(
        (
            status["staged"].as_str(),
            status["unstaged"].as_str(),
            status["untracked"].as_str()
        ),
        ("1", "1", "1")
    );
    assert_eq!(status["head"].len(), 40);

    let (message, changes) = run(&agent, "changes", json!({"limit": 2})).unwrap();
    assert_eq!(message, "3 changed paths");
    assert_eq!(changes["changed"], "3");
    assert_eq!(changes["files"].lines().count(), 2);
    assert_eq!(changes["truncated"], "true");
    let (_, all) = run(&agent, "changes", json!({})).unwrap();
    assert_eq!(all["files"], ".M a.txt\nM. b.txt\n?? new file.txt");
    assert_eq!(all["truncated"], "false");

    let (message, last) = run(&agent, "last-commit", json!({})).unwrap();
    assert_eq!(message, "1234567 Add b");
    assert_eq!(last["subject"], "Add b");
    assert_eq!(last["author"], "Test");
    assert_eq!(last["hash"], status["head"]);
    assert!(last["date"].contains('T'));
    assert_eq!(last["shortHash"], "1234567");
}

#[test]
fn a_detached_head() {
    let agent = agent();
    record(
        &agent,
        &status_args("no"),
        ok(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head (detached)",
        ])),
    );
    let (message, branch) = run(&agent, "branch", json!({})).unwrap();
    assert_eq!(message, "detached at 1234567890ab");
    assert_eq!(branch["detached"], "true");
    assert_eq!(branch["head"], OID);
    assert!(!branch.contains_key("branch"));
    let (message, remote) = run(&agent, "remote-status", json!({})).unwrap();
    assert_eq!(remote["hasUpstream"], "false");
    assert_eq!(remote["source"], "cached");
    assert_eq!(message, "HEAD is detached, so there is no upstream");
}

/// The built-in's `repositories_of_explicit_projects_and_resources`.
#[test]
fn repositories_of_explicit_projects_and_resources() {
    let agent = agent();
    record(
        &agent,
        &status_args("no"),
        ok(&z(&["# branch.oid (initial)", "# branch.head main"])),
    );
    // A named project that is not the active one: its role, no key (the
    // plugin reads only the active project's key).
    let (_, data) = run(&agent, "branch", json!({"projectId": "p-cms"})).unwrap();
    assert!(!data.contains_key("projectKey"));
    let named = agent.borrow().resolves.last().cloned().unwrap();
    assert_eq!(named.project_id.as_deref(), Some("p-cms"));
    // The active project named explicitly: its key.
    let (_, data) = run(&agent, "branch", json!({"projectId": "p-shop"})).unwrap();
    assert_eq!(data["projectKey"], "SHOP");

    // A fixed resource: no project key.
    agent.borrow_mut().param = Some(Ok(("cms.repository", "/work/cms")));
    let (_, by_resource) = run(&agent, "branch", json!({"repository": HANDLE})).unwrap();
    assert_eq!(by_resource["repository"], "/work/cms");
    assert!(!by_resource.contains_key("projectKey"));
    agent.borrow_mut().param = None;

    // A project of another profile.
    agent.borrow_mut().role =
        Err("not-permitted: the project is neither the run's nor one the action's parameters name");
    let (code, message) = run(&agent, "status", json!({"projectId": "p-home"})).unwrap_err();
    assert_eq!(code, codes::PROJECT_NOT_IN_PROFILE);
    assert_eq!(message, "the project does not belong to this profile");

    // Without a project there is nothing to fall back to.
    agent.borrow_mut().role = Err("no-active-project: no project is active");
    let (code, message) = run(&agent, "status", json!({})).unwrap_err();
    assert_eq!(code, "no-active-project");
    assert_eq!(message, "no project is active and no repository is set");
}

/// The built-in's `missing_repositories_and_git`.
#[test]
fn missing_repositories_and_git() {
    let agent = agent();
    record(
        &agent,
        &status_args("all"),
        failed(
            128,
            "fatal: not a git repository (or any of the parent directories): .git\n",
        ),
    );
    let (code, message) = run(&agent, "status", json!({})).unwrap_err();
    assert_eq!(code, codes::NOT_A_REPOSITORY);
    assert_eq!(message, "/work/shop is not a Git repository");

    record(
        &agent,
        &status_args("all"),
        failed(
            128,
            "fatal: detected dubious ownership in repository at '/work/shop'\n",
        ),
    );
    assert_eq!(
        run(&agent, "status", json!({})).unwrap_err().0,
        codes::UNSAFE_REPOSITORY
    );

    agent.borrow_mut().role =
        Err("resource-unbound: `none.repository` is not bound on this computer");
    let (code, message) = run(&agent, "status", json!({})).unwrap_err();
    assert_eq!(code, "resource-unbound");
    assert_eq!(message, "`none.repository` is not bound on this computer");

    agent.borrow_mut().role = Ok(("shop.repository", SHOP));
    agent.borrow_mut().tool_refusal = Some((
        CallErrorCode::Unavailable,
        "tool-not-found: git was not found; install it or set its location in DeckPilot's settings",
    ));
    assert_eq!(
        run(&agent, "status", json!({})).unwrap_err().0,
        codes::GIT_NOT_FOUND
    );
    // Not consented to: the agent's refusal, as the shared code.
    agent.borrow_mut().tool_refusal = Some((
        CallErrorCode::NotPermitted,
        "permission `tools:git` is not granted",
    ));
    assert_eq!(
        run(&agent, "status", json!({})).unwrap_err().0,
        "not-permitted"
    );
    agent.borrow_mut().tool_refusal = Some((
        CallErrorCode::Failed,
        "timed-out: the program did not finish within 10000 ms and was stopped",
    ));
    assert_eq!(run(&agent, "status", json!({})).unwrap_err().0, "timeout");
    // The agent's own check of the work tree.
    for refusal in [
        "not-a-repository: the bound folder is not a git work tree",
        "tool-outside-binding: the repository's work tree is not inside the bound folder",
    ] {
        agent.borrow_mut().tool_refusal = Some((CallErrorCode::Failed, refusal));
        let (code, message) = run(&agent, "status", json!({})).unwrap_err();
        assert_eq!(code, codes::NOT_A_REPOSITORY);
        assert!(
            message.starts_with("/work/shop is not a Git repository"),
            "{message}"
        );
    }
    // Any other failure of git is `git-failed` with its first line.
    agent.borrow_mut().tool_refusal = None;
    record(
        &agent,
        &status_args("all"),
        failed(1, "error: something\nmore\n"),
    );
    let (code, message) = run(&agent, "status", json!({})).unwrap_err();
    assert_eq!(code, codes::GIT_FAILED);
    assert_eq!(message, "git status failed: error: something");
    // Output git would never print.
    record(&agent, &status_args("all"), ok("not porcelain\0"));
    let (code, message) = run(&agent, "status", json!({})).unwrap_err();
    assert_eq!(code, codes::GIT_FAILED);
    assert!(
        message.starts_with("unexpected output from git status"),
        "{message}"
    );
}

/// The built-in's `remote_status_from_what_was_last_fetched`.
#[test]
fn remote_status_from_what_was_last_fetched() {
    let agent = agent();
    record(
        &agent,
        &status_args("no"),
        ok(&z(&[&format!("# branch.oid {OID}"), "# branch.head main"])),
    );
    record(&agent, "config --get branch.main.remote", failed(1, ""));
    let (message, no_upstream) = run(&agent, "remote-status", json!({})).unwrap();
    assert_eq!(no_upstream["hasUpstream"], "false");
    assert_eq!(no_upstream["branch"], "main");
    assert_eq!(message, "main has no upstream branch");

    record(&agent, &status_args("no"), ok(&tracking_main(1, 0, &[])));
    record(&agent, "config --get branch.main.remote", ok("origin\n"));
    record(
        &agent,
        "rev-parse --verify -q origin/main",
        ok(&format!("{OID}\n")),
    );
    let (message, cached) = run(&agent, "remote-status", json!({})).unwrap();
    assert_eq!(
        message,
        "main → origin/main: 1 ahead, 0 behind as last fetched"
    );
    assert_eq!(cached["source"], "cached");
    assert_eq!(cached["upstream"], "origin/main");
    assert_eq!(cached["remote"], "origin");
    assert_eq!(
        (cached["ahead"].as_str(), cached["behind"].as_str()),
        ("1", "0")
    );
    assert_eq!(cached["trackingHead"], OID);
    assert!(!cached.contains_key("remoteHead"));
    // No network without `refresh`.
    assert!(!agent.borrow().runs.iter().any(|r| r[0] == "ls-remote"));
}

/// The built-in's `refreshing_asks_the_remote_without_changing_the_repository`.
#[test]
fn refreshing_asks_the_remote() {
    let agent = agent();
    let moved = "abcdefabcdefabcdefabcdefabcdefabcdefabcd";
    record(&agent, &status_args("no"), ok(&tracking_main(0, 0, &[])));
    record(&agent, "config --get branch.main.remote", ok("origin\n"));
    record(
        &agent,
        "config --get branch.main.merge",
        ok("refs/heads/main\n"),
    );
    record(&agent, "remote", ok("origin\n"));
    record(
        &agent,
        "rev-parse --verify -q origin/main",
        ok(&format!("{OID}\n")),
    );
    record(
        &agent,
        "ls-remote -- origin refs/heads/main",
        ok(&format!("{OID}\trefs/heads/main\n")),
    );
    let refresh = json!({"refresh": true});
    let (message, fresh) = run(&agent, "remote-status", refresh.clone()).unwrap();
    assert_eq!(fresh["source"], "network");
    assert_eq!(fresh["stale"], "false");
    assert!(
        message.ends_with("; the remote has not moved since"),
        "{message}"
    );
    // The network time limit of the settings, in milliseconds.
    let timeouts = agent.borrow().timeouts.clone();
    assert_eq!(timeouts.last(), Some(&Some(20_000)));

    // Someone else pushed.
    record(
        &agent,
        "ls-remote -- origin refs/heads/main",
        ok(&format!("{moved}\trefs/heads/main\n")),
    );
    let (message, moved_on) = run(&agent, "remote-status", refresh.clone()).unwrap();
    assert_eq!(moved_on["stale"], "true");
    assert_eq!(moved_on["remoteHead"], moved);
    assert_eq!(moved_on["trackingHead"], OID);
    assert!(
        message.ends_with("; the remote has moved since"),
        "{message}"
    );

    // The branch does not exist on the remote.
    record(&agent, "ls-remote -- origin refs/heads/main", ok(""));
    let (message, gone) = run(&agent, "remote-status", refresh.clone()).unwrap();
    assert!(!gone.contains_key("stale"));
    assert!(
        message.ends_with("; refs/heads/main does not exist on the remote"),
        "{message}"
    );

    // The remote cannot be reached.
    record(
        &agent,
        "ls-remote -- origin refs/heads/main",
        failed(
            128,
            "fatal: unable to access 'https://host/a.git/': Could not resolve host: host\n",
        ),
    );
    assert_eq!(
        run(&agent, "remote-status", refresh.clone()).unwrap_err().0,
        codes::NETWORK_FAILED
    );

    // A remote that is no longer configured.
    record(&agent, "config --get branch.main.remote", ok("gone\n"));
    let (code, message) = run(&agent, "remote-status", refresh.clone()).unwrap_err();
    assert_eq!(code, codes::NO_REMOTE);
    assert_eq!(message, "the remote gone is not configured");

    // A tracking branch whose remote was removed.
    record(
        &agent,
        &status_args("no"),
        ok(&z(&[&format!("# branch.oid {OID}"), "# branch.head main"])),
    );
    let (code, message) = run(&agent, "remote-status", refresh).unwrap_err();
    assert_eq!(code, codes::NO_REMOTE);
    assert_eq!(message, "the remote gone of main is not configured");
    let (message, _) = run(&agent, "remote-status", json!({})).unwrap();
    assert_eq!(
        message,
        "main tracks the remote gone, which is not configured"
    );
}

#[test]
fn opening_a_repository() {
    let agent = agent();
    agent.borrow_mut().application = Some("tools.editor");
    let (message, data) = run(&agent, "open-repository", json!({"application": APP})).unwrap();
    assert_eq!(message, "opened /work/shop");
    assert_eq!(data["repository"], SHOP);
    let opened = agent.borrow().opened.clone();
    assert_eq!(
        opened,
        [ResourceOpen {
            handle: HANDLE.into(),
            with: Some(APP.into()),
            position: None,
            editor: None
        }]
    );
    // Without an application: the file manager (the system's default).
    agent.borrow_mut().application = None;
    run(&agent, "open-repository", json!({})).unwrap();
    assert_eq!(agent.borrow().opened[1].with, None);
    // Nothing ran.
    assert!(agent.borrow().runs.is_empty());
}

// ------------------------------------------------------------ key state

fn key_state(agent: &Shared, action: &str, args: Value) -> StateOutcome {
    host(agent).key_state(&plugin(), action, args)
}

/// The built-in's `clean_dirty_and_conflicted_work_trees` and
/// `detached_and_unborn_heads` on the state function.
#[test]
fn clean_dirty_and_conflicted_work_trees() {
    let oid = format!("# branch.oid {OID}");
    let clean = porcelain::parse(&z(&[&oid, "# branch.head main"])).unwrap();
    assert_eq!(
        state::state_of(&clean, true),
        KeyState::new().detail("main").indicator(Indicator::Ok)
    );
    let dirty = porcelain::parse(&z(&[&oid, "# branch.head feature/x", "? notes.txt"])).unwrap();
    assert_eq!(
        state::state_of(&dirty, true),
        KeyState::new()
            .detail("feature/x")
            .indicator(Indicator::Warning)
    );
    let conflict = porcelain::parse(&z(&[
        &oid,
        "# branch.head main",
        "u UU N... 100644 100644 100644 100644 \
         1111111111111111111111111111111111111111 \
         2222222222222222222222222222222222222222 \
         3333333333333333333333333333333333333333 a.txt",
    ]))
    .unwrap();
    assert_eq!(
        state::state_of(&conflict, true).indicator,
        Some(Indicator::Error)
    );
    assert_eq!(
        state::state_of(&dirty, false),
        KeyState::new().detail("feature/x")
    );
    let detached = porcelain::parse(&z(&[&oid, "# branch.head (detached)"])).unwrap();
    assert_eq!(
        state::state_of(&detached, false).detail.as_deref(),
        Some("@1234567")
    );
    let unborn = porcelain::parse(&z(&["# branch.oid (initial)", "# branch.head main"])).unwrap();
    assert_eq!(
        state::state_of(&unborn, false).detail.as_deref(),
        Some("main")
    );
}

/// Keys show what the last run found; a state call never runs git.
#[test]
fn keys_show_the_last_run() {
    let agent = agent();
    // Before any run: the key as designed.
    assert_eq!(
        key_state(&agent, "status", json!({})),
        StateOutcome::Ok(KeyState::new())
    );

    record(
        &agent,
        &status_args("all"),
        ok(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head main",
            "? notes.txt",
        ])),
    );
    run(&agent, "status", json!({})).unwrap();
    let runs = agent.borrow().runs.len();
    assert_eq!(
        key_state(&agent, "status", json!({})),
        StateOutcome::Ok(KeyState::new().detail("main").indicator(Indicator::Warning))
    );
    // A status run is a branch run too.
    assert_eq!(
        key_state(&agent, "branch", json!({})),
        StateOutcome::Ok(KeyState::new().detail("main"))
    );
    assert_eq!(agent.borrow().runs.len(), runs, "a state call runs nothing");
    // Another project's keys are their own.
    assert_eq!(
        key_state(&agent, "status", json!({"projectId": "p-cms"})),
        StateOutcome::Ok(KeyState::new())
    );

    // A failed run: the key shows the failure.
    record(
        &agent,
        &status_args("all"),
        failed(128, "fatal: not a git repository\n"),
    );
    run(&agent, "status", json!({})).unwrap_err();
    match key_state(&agent, "status", json!({})) {
        StateOutcome::Error(failure) => assert_eq!(failure.code, codes::NOT_A_REPOSITORY),
        other => panic!("{other:?}"),
    }
    // Other actions have no key state.
    assert!(matches!(
        key_state(&agent, "changes", json!({})),
        StateOutcome::Error(_)
    ));
}

#[test]
fn ls_remote_output_and_commits_parse() {
    let out = format!("{OID}\trefs/heads/main\nabc\trefs/heads/other\n");
    assert_eq!(
        parse_ls_remote(&out, "refs/heads/main").as_deref(),
        Some(OID)
    );
    assert_eq!(parse_ls_remote(&out, "refs/heads/x"), None);
    assert_eq!(
        parse_commit("only\0three\0fields").unwrap_err().code(),
        codes::GIT_FAILED
    );
}
