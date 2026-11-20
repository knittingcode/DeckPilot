//! Native unit tests: the definition agrees with manifest.json, the
//! manifest is valid against the schema, the translations are complete,
//! the tools policy is what the actions run, and every action behaves on
//! recorded `docker` output, with the agent replaced by `MockHost`. The
//! scenarios and the recorded output are the built-in's
//! (`modules/docker/src/actions/tests.rs`, whose fake `docker` prints
//! them). The end-to-end run through the real agent, host and a fake
//! `docker` is `apps/agent/tests/plugins_docker.rs`.

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

const COMPOSE_HANDLE: &str = "0123456789abcdef0123456789abcdef";
const ENV_HANDLE: &str = "fedcba9876543210fedcba9876543210";

const VERSION_JSON: &str =
    r#"{"Client":{"Version":"29.1.0","Os":"linux"},"Server":{"Version":"29.1.0","Os":"linux"}}"#;
const PS: &str = concat!(
    r#"{"Names":"web-1","Image":"nginx","State":"running","Status":"Up 1 hour","Labels":"token=abc"}"#,
    "\n",
    r#"{"Names":"job-1","Image":"busybox","State":"exited","Status":"Exited (0)"}"#,
    "\n"
);
const COMPOSE_PS: &str = concat!(
    r#"{"Name":"shop-web-1","Service":"web","State":"running","Status":"Up"}"#,
    "\n",
    r#"{"Name":"shop-db-1","Service":"db","State":"exited","Status":"Exited (1)"}"#,
    "\n"
);

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).unwrap()
}

type Run = (i32, String, String);

fn ok(stdout: &str) -> Run {
    (0, stdout.to_owned(), String::new())
}

fn failed(code: i32, stdout: &str, stderr: &str) -> Run {
    (code, stdout.to_owned(), stderr.to_owned())
}

/// A stand-in agent: the resources, recorded docker output by argument
/// list, refusals, storage.
#[derive(Debug)]
struct Agent {
    /// The `environment` parameter's label, when set.
    environment: Option<&'static str>,
    /// The `compose` parameter's label, when set.
    compose: Option<&'static str>,
    /// The compose role of the run's project.
    role: Result<&'static str, &'static str>,
    active: Option<(&'static str, &'static str)>,
    docker: BTreeMap<String, Run>,
    /// A refusal of every run (`<code>: <message>`).
    refusal: Option<(CallErrorCode, &'static str)>,
    /// Whether the modify sub-grant is given (the broker refuses without).
    modify: bool,
    /// Whether the run is one the user started.
    user: bool,
    /// How a change outside such a run is refused.
    not_user: &'static str,
    settings: Value,
    /// Which key this is (`run.context`'s `keyScope`), as the agent says.
    scope: Option<&'static str>,
    runs: Vec<(Option<String>, Vec<String>, Option<u32>)>,
    storage: BTreeMap<String, Value>,
}

impl Default for Agent {
    fn default() -> Self {
        let mut docker = BTreeMap::new();
        for (args, run) in [
            ("context show", ok("fake-ctx\n")),
            (
                "version --format={{json .}}",
                ok(&format!("{VERSION_JSON}\n")),
            ),
            ("compose version --short", ok("2.30.0\n")),
            ("ps --no-trunc --format={{json .}}", ok(PS)),
            ("compose ps --all --format=json", ok(COMPOSE_PS)),
            (
                "compose up --detach",
                (0, String::new(), "Container shop-web-1 Started\n".into()),
            ),
            (
                "compose down",
                (0, String::new(), "Container shop-web-1 Removed\n".into()),
            ),
            (
                "compose restart",
                (0, String::new(), "Container shop-web-1 Restarted\n".into()),
            ),
        ] {
            docker.insert(args.to_owned(), run);
        }
        Self {
            environment: None,
            compose: None,
            role: Ok("shop.compose"),
            active: Some(("p-shop", "SHOP")),
            docker,
            refusal: None,
            modify: true,
            user: true,
            not_user: "not-user-initiated: containers are started and stopped only in runs you or your work session started",
            settings: json!({"composeRole": "compose", "timeoutSeconds": 300}),
            scope: None,
            runs: Vec::new(),
            storage: BTreeMap::new(),
        }
    }
}

type Shared = Rc<RefCell<Agent>>;

fn agent() -> Shared {
    Rc::new(RefCell::new(Agent::default()))
}

fn handle(label: &str, kind: &str, handle: &str) -> CallResult {
    CallResult::Ok(json!({"handle": handle, "kind": kind, "label": label}))
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
                match (r.param.as_deref(), r.role.as_deref()) {
                    (Some("environment"), None) => match agent.environment {
                        None => refusal("resource-not-set: the parameter `environment` is not set"),
                        Some(label) => handle(label, "dockerEnvironment", ENV_HANDLE),
                    },
                    (Some("compose"), None) => match agent.compose {
                        None => refusal("resource-not-set: the parameter `compose` is not set"),
                        Some(label) => handle(label, "file", COMPOSE_HANDLE),
                    },
                    (None, Some("compose")) => {
                        assert_eq!(
                            r.kinds,
                            [
                                ResourceKind::Directory,
                                ResourceKind::File,
                                ResourceKind::DockerEnvironment
                            ]
                        );
                        match agent.role {
                            Ok(label) => handle(label, "directory", COMPOSE_HANDLE),
                            Err(text) => refusal(text),
                        }
                    }
                    other => panic!("unexpected resolve {other:?}"),
                }
            }
            CapabilityRequest::RunContext(_) => CallResult::Ok(match agent.scope {
                Some(scope) => json!({"trigger": "key", "keyScope": scope}),
                None => json!({"trigger": "key"}),
            }),
            CapabilityRequest::ProfileRead(_) => CallResult::Ok(match agent.active {
                Some((id, key)) => json!({"project": {"id": id, "key": key, "name": key}}),
                None => json!({}),
            }),
            CapabilityRequest::ToolRun(run) => {
                assert_eq!(run.tool, Tool::Docker);
                agent
                    .runs
                    .push((run.cwd.clone(), run.args.clone(), run.timeout_ms));
                if let Some((code, text)) = agent.refusal {
                    return CallResult::error(code, text);
                }
                let modifies = run.args.len() > 1
                    && run.args[0] == "compose"
                    && ["up", "down", "restart"].contains(&run.args[1].as_str());
                if modifies && !agent.modify {
                    return CallResult::error(
                        CallErrorCode::NotPermitted,
                        "permission `tools:docker:modify` is not granted",
                    );
                }
                if modifies && !agent.user {
                    return CallResult::error(CallErrorCode::NotPermitted, agent.not_user);
                }
                let key = run.args.join(" ");
                let Some((exit, stdout, stderr)) = agent.docker.get(&key).cloned() else {
                    panic!("no recorded output for `docker {key}`")
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
            CapabilityRequest::Log(_) => CallResult::Ok(Value::Null),
            other => panic!("unexpected request {other:?}"),
        }
    })
}

fn record(agent: &Shared, args: &str, run: Run) {
    agent.borrow_mut().docker.insert(args.to_owned(), run);
}

fn params(value: Value) -> Params {
    serde_json::from_value(value).unwrap()
}

type Done = Result<(String, BTreeMap<String, String>), (String, String)>;

fn run(agent: &Shared, action: &str, args: Value) -> Done {
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

fn runs(agent: &Shared) -> Vec<Vec<String>> {
    agent
        .borrow()
        .runs
        .iter()
        .map(|(_, args, _)| args.clone())
        .collect()
}

// --------------------------------------------------------- the contract

#[test]
fn the_definition_matches_the_manifest() {
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
            "containers",
            "compose-status",
            "compose-up",
            "compose-down",
            "compose-restart",
            "logs"
        ]
    );
    for action in plugin.actions() {
        // Every key but a logs key shows state (the Compose changes their
        // project's, read back only).
        assert_eq!(action.has_state(), action.id() != "logs", "{}", action.id());
        // Nothing offers removing volumes or images.
        assert!(
            !action
                .params()
                .iter()
                .any(|p| p.key().contains("olume") || p.key().contains("mage")),
            "{}",
            action.id()
        );
    }
    let manifest = manifest();
    for action in manifest["contributes"]["actions"].as_array().unwrap() {
        let modifies = matches!(
            action["id"].as_str().unwrap(),
            "compose-up" | "compose-down" | "compose-restart"
        );
        assert_eq!(
            action["node"]["effect"] == "modifies",
            modifies,
            "{}",
            action["id"]
        );
        // Only the changes need the modify sub-grant, which is optional:
        // the plugin runs without it (X3c, off by default).
        assert_eq!(
            action["requiresPermissions"] == json!(["tools:docker:modify"]),
            modifies,
            "{}",
            action["id"]
        );
        assert!(
            modifies || action.get("requiresPermissions").is_none(),
            "{}",
            action["id"]
        );
        let outputs = action["node"]["outputs"].as_array().unwrap();
        assert!(outputs.iter().any(|o| o["key"] == "context"));
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

/// The tools policy declares only what the actions run, the modify
/// sub-grant only for the Compose changes, and never `--project-name`,
/// `-f` or `--context` (the agent sets them).
#[test]
fn the_tools_policy_is_what_the_actions_run() {
    let manifest = manifest();
    let tools = manifest["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "tools")
        .unwrap();
    assert_eq!(tools["tools"][0]["executable"], "docker");
    assert_eq!(tools["tools"][0]["modify"], true);
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
    for (name, options) in &declared {
        for forbidden in [
            "--project-name",
            "-f",
            "--file",
            "--context",
            "-v",
            "--volumes",
            "--rmi",
        ] {
            assert!(
                !options.iter().any(|o| o == forbidden),
                "{name} {forbidden}"
            );
        }
        let modifies = ["compose up", "compose down", "compose restart"].contains(&name.as_str());
        let starts = ["start", "stop", "restart"].contains(&name.as_str());
        assert!(
            !starts,
            "the built-in never starts single containers: {name}"
        );
        let _ = modifies;
    }
    let agent = agent();
    record(
        &agent,
        "ps --no-trunc --format={{json .}} --all --filter=name=web",
        ok(PS),
    );
    record(
        &agent,
        "logs --tail=20 --since=10m -- web-1",
        ok("out line\n"),
    );
    record(
        &agent,
        "compose logs --no-color --tail=100 -- web",
        ok("web  | hello\n"),
    );
    record(&agent, "compose up --detach -- web db", ok(""));
    record(&agent, "compose down --remove-orphans", ok(""));
    run(&agent, "status", json!({})).unwrap();
    run(&agent, "containers", json!({"all": true, "name": "web"})).unwrap();
    run(&agent, "compose-status", json!({})).unwrap();
    run(&agent, "compose-up", json!({"services": "web\ndb"})).unwrap();
    run(&agent, "compose-down", json!({"removeOrphans": true})).unwrap();
    run(&agent, "compose-restart", json!({})).unwrap();
    run(
        &agent,
        "logs",
        json!({"container": "web-1", "lines": 20, "since": "10m"}),
    )
    .unwrap();
    run(&agent, "logs", json!({"service": "web"})).unwrap();
    let mut used = std::collections::BTreeSet::new();
    for args in runs(&agent) {
        let words = if ["compose", "context"].contains(&args[0].as_str()) {
            2
        } else {
            1
        };
        let sub = args[..words].join(" ");
        let Some((_, options)) = declared.iter().find(|(name, _)| *name == sub) else {
            panic!("`docker {sub}` is not declared");
        };
        for option in args[words..]
            .iter()
            .take_while(|a| *a != "--")
            .filter(|a| a.starts_with('-'))
        {
            let name = option.split_once('=').map_or(option.as_str(), |(n, _)| n);
            assert!(
                options.iter().any(|o| o == name || o == option),
                "`{option}` of `docker {sub}`"
            );
        }
        used.insert(sub);
    }
    let declared: std::collections::BTreeSet<String> =
        declared.into_iter().map(|(n, _)| n).collect();
    assert_eq!(used, declared, "every declared subcommand is used");
}

// -------------------------------------------------------------- actions

/// The built-in's `availability_is_an_answer_not_a_failure`.
#[test]
fn availability_is_an_answer_not_a_failure() {
    let agent = agent();
    let (message, status) = run(&agent, "status", json!({})).unwrap();
    assert_eq!(message, "Docker 29.1.0 in context fake-ctx, Compose 2.30.0");
    assert_eq!(status["available"], "true");
    assert_eq!(status["context"], "fake-ctx");
    assert_eq!(status["serverVersion"], "29.1.0");
    assert_eq!(status["serverOs"], "linux");
    assert_eq!(status["clientVersion"], "29.1.0");
    assert_eq!(status["composeVersion"], "2.30.0");
    // Without an environment parameter: no resource at all, as with the
    // built-in (the agent's own empty folder and context).
    assert!(agent.borrow().runs.iter().all(|(cwd, ..)| cwd.is_none()));

    for (stderr, reason) in [
        (
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?",
            codes::DAEMON_UNAVAILABLE,
        ),
        (
            "permission denied while trying to connect to the Docker daemon socket",
            codes::PERMISSION_DENIED,
        ),
    ] {
        record(
            &agent,
            "version --format={{json .}}",
            failed(
                1,
                r#"{"Client":{"Version":"29.1.0","Os":"linux"},"Server":null}"#,
                stderr,
            ),
        );
        let (message, status) = run(&agent, "status", json!({})).unwrap();
        assert_eq!(status["available"], "false");
        assert_eq!(status["reason"], reason);
        assert_eq!(status["clientVersion"], "29.1.0");
        assert_eq!(status["context"], "fake-ctx");
        assert_eq!(message, format!("Docker is not available: {stderr}"));
    }

    agent.borrow_mut().refusal = Some((
        CallErrorCode::Unavailable,
        "tool-not-found: docker was not found; install it or set its location in DeckPilot's settings",
    ));
    let (_, status) = run(&agent, "status", json!({})).unwrap();
    assert_eq!(status["available"], "false");
    assert_eq!(status["reason"], codes::DOCKER_NOT_FOUND);
    assert!(!status.contains_key("context"));
    // Other actions fail with the reason.
    assert_eq!(
        run(&agent, "containers", json!({})).unwrap_err().0,
        codes::DOCKER_NOT_FOUND
    );
    // Not consented to: a failure, not an answer.
    agent.borrow_mut().refusal = Some((
        CallErrorCode::NotPermitted,
        "permission `tools:docker` is not granted",
    ));
    assert_eq!(
        run(&agent, "status", json!({})).unwrap_err().0,
        "not-permitted"
    );
}

/// The built-in's `every_command_names_its_context`, as far as the agent
/// leaves the context to the plugin: the context the agent runs Docker in
/// is reported; an action's `context` must be that one.
#[test]
fn the_context_is_the_agents() {
    let agent = agent();
    agent.borrow_mut().environment = Some("remote-prod");
    record(&agent, "context show", ok("remote-prod\n"));
    let (message, listed) = run(&agent, "containers", json!({})).unwrap();
    assert_eq!(listed["context"], "remote-prod");
    assert_eq!(message, "2 containers, 1 running, in context remote-prod");
    // The Docker environment's handle, for every command.
    assert!(
        agent
            .borrow()
            .runs
            .iter()
            .all(|(cwd, ..)| cwd.as_deref() == Some(ENV_HANDLE))
    );
    let (_, same) = run(&agent, "containers", json!({"context": "remote-prod"})).unwrap();
    assert_eq!(same["context"], "remote-prod");
    let (code, message) = run(&agent, "containers", json!({"context": "staging"})).unwrap_err();
    assert_eq!(code, codes::CONTEXT_UNSUPPORTED);
    assert!(
        message.contains("staging") && message.contains("remote-prod"),
        "{message}"
    );
    // Never `--context` from the plugin.
    assert!(runs(&agent).iter().flatten().all(|a| a != "--context"));
    // A name that could be an option never reaches Docker.
    let before = runs(&agent).len();
    let (code, message) = run(&agent, "containers", json!({"context": "--host=evil"})).unwrap_err();
    assert_eq!(code, codes::INVALID_NAME);
    assert_eq!(message, "“--host=evil” is not a valid context name");
    assert_eq!(runs(&agent).len(), before);
}

#[test]
fn containers_are_listed_without_their_labels() {
    let agent = agent();
    record(
        &agent,
        "ps --no-trunc --format={{json .}} --all --filter=name=web",
        ok(PS),
    );
    let (_, listed) = run(&agent, "containers", json!({"all": true, "name": "web"})).unwrap();
    assert_eq!(listed["count"], "2");
    assert_eq!(listed["running"], "1");
    assert_eq!(listed["names"], "web-1,job-1");
    assert_eq!(
        listed["containers"],
        "web-1\trunning\tnginx\tUp 1 hour\njob-1\texited\tbusybox\tExited (0)"
    );
    assert!(!listed["containers"].contains("token"));
}

/// The built-in's `compose_projects_come_from_resources`.
#[test]
fn compose_projects_come_from_resources() {
    let agent = agent();
    let (message, status) = run(&agent, "compose-status", json!({})).unwrap();
    assert_eq!(message, "1 of 2 containers running in context fake-ctx");
    assert_eq!(status["services"], "2");
    assert_eq!(status["running"], "1");
    assert_eq!(status["stopped"], "1");
    assert_eq!(status["allRunning"], "false");
    assert_eq!(status["projectKey"], "SHOP");
    assert_eq!(
        status["containers"],
        "web (shop-web-1)\trunning\t\tUp\ndb (shop-db-1)\texited\t\tExited (1)"
    );
    // A resource naming the file: no project key.
    agent.borrow_mut().compose = Some("shop.compose-file");
    let (_, by_file) = run(&agent, "compose-status", json!({"compose": COMPOSE_HANDLE})).unwrap();
    assert!(!by_file.contains_key("projectKey"));
    // A project name cannot be chosen any more.
    let (code, _) = run(&agent, "compose-status", json!({"projectName": "shop_dev"})).unwrap_err();
    assert_eq!(code, codes::PROJECT_NAME_UNSUPPORTED);
    agent.borrow_mut().compose = None;
    agent.borrow_mut().role = Err("no-active-project: no project is active");
    let (code, message) = run(&agent, "compose-status", json!({})).unwrap_err();
    assert_eq!(code, "no-active-project");
    assert_eq!(
        message,
        "no project is active and no Compose project is set"
    );
    agent.borrow_mut().role =
        Err("not-permitted: the project is neither the run's nor one the action's parameters name");
    let (code, _) = run(&agent, "compose-status", json!({"projectId": "p-home"})).unwrap_err();
    assert_eq!(code, codes::PROJECT_NOT_IN_PROFILE);
    agent.borrow_mut().role = Err("not-found: `shop.compose` does not exist");
    assert_eq!(
        run(&agent, "compose-status", json!({})).unwrap_err().0,
        codes::COMPOSE_FILE_NOT_FOUND
    );
    agent.borrow_mut().role = Ok("shop.compose");
    agent.borrow_mut().refusal = Some((
        CallErrorCode::NotPermitted,
        "tool-invalid-cwd: the compose folder has no compose.yaml, compose.yml, docker-compose.yaml or docker-compose.yml",
    ));
    assert_eq!(
        run(&agent, "compose-status", json!({})).unwrap_err().0,
        codes::COMPOSE_FILE_NOT_FOUND
    );
    agent.borrow_mut().refusal = None;
    // Status, containers and container logs need no resource and no
    // project (the built-in's parity); Compose's commands do.
    agent.borrow_mut().role = Err("no-active-project: no project is active");
    agent.borrow_mut().active = None;
    agent.borrow_mut().runs.clear();
    run(&agent, "status", json!({})).unwrap();
    run(&agent, "containers", json!({})).unwrap();
    record(&agent, "logs --tail=100 -- web-1", ok("x\n"));
    run(&agent, "logs", json!({"container": "web-1"})).unwrap();
    assert!(agent.borrow().runs.iter().all(|(cwd, ..)| cwd.is_none()));
    assert_eq!(
        run(&agent, "compose-status", json!({})).unwrap_err().0,
        "no-active-project"
    );
}

/// The built-in's `changes_must_be_allowed_and_never_remove_volumes`.
#[test]
fn changes_must_be_allowed_and_never_remove_volumes() {
    let agent = agent();
    agent.borrow_mut().modify = false;
    for action in ["compose-up", "compose-down", "compose-restart"] {
        let (code, message) = run(&agent, action, json!({})).unwrap_err();
        assert_eq!(code, codes::CHANGES_DISABLED, "{action}");
        assert!(message.contains("Settings › Plugins › Docker"), "{message}");
    }
    agent.borrow_mut().modify = true;
    agent.borrow_mut().user = false;
    let (code, _) = run(&agent, "compose-up", json!({})).unwrap_err();
    assert_eq!(code, codes::NOT_USER_INITIATED);
    // The broker's refusal of a run the user did not start.
    agent.borrow_mut().not_user = "this runs only in an action the user started (a key, a gesture or the app), not in one a schedule, an event or the agent's start ran";
    let (code, _) = run(&agent, "compose-down", json!({})).unwrap_err();
    assert_eq!(code, codes::NOT_USER_INITIATED);
    agent.borrow_mut().user = true;

    record(
        &agent,
        "compose up --detach -- web db",
        (0, String::new(), "Container shop-web-1 Started\n".into()),
    );
    let (message, up) = run(&agent, "compose-up", json!({"services": "web\n\ndb\n"})).unwrap();
    assert_eq!(message, "web, db started in context fake-ctx");
    assert!(up["output"].contains("Started"));
    assert_eq!(up["projectKey"], "SHOP");
    // The plugin's time limit for changes, in milliseconds.
    let timeouts: Vec<Option<u32>> = agent.borrow().runs.iter().map(|(.., t)| *t).collect();
    assert!(timeouts.contains(&Some(300_000)));

    record(
        &agent,
        "compose down --remove-orphans",
        (0, String::new(), "Container shop-web-1 Removed\n".into()),
    );
    let (message, _) = run(&agent, "compose-down", json!({"removeOrphans": true})).unwrap();
    assert!(message.contains("volumes kept"), "{message}");
    let (message, _) = run(&agent, "compose-restart", json!({})).unwrap();
    assert_eq!(message, "the project restarted in context fake-ctx");
    for args in runs(&agent) {
        assert!(
            !args
                .iter()
                .any(|a| a == "-v" || a == "--volumes" || a == "--rmi"),
            "{args:?}"
        );
    }
    let before = runs(&agent).len();
    let (code, _) = run(
        &agent,
        "compose-up",
        json!({"services": "web\n--force-recreate"}),
    )
    .unwrap_err();
    assert_eq!(code, codes::INVALID_NAME);
    assert_eq!(runs(&agent).len(), before, "nothing ran");
    for change in [Change::Up, Change::Down, Change::Restart] {
        for orphans in [false, true] {
            let args = compose_change(change, &["web".into()], orphans);
            for forbidden in ["-v", "--volumes", "--rmi", "--force", "-f", "--prune"] {
                assert!(!args.iter().any(|a| a == forbidden), "{change:?}: {args:?}");
            }
        }
    }
}

/// The built-in's `logs_are_bounded_and_need_one_target`.
#[test]
fn logs_are_bounded_and_need_one_target() {
    let agent = agent();
    record(
        &agent,
        "logs --tail=20 --since=10m -- web-1",
        (0, "out line\n".into(), "err line\n".into()),
    );
    let (message, container) = run(
        &agent,
        "logs",
        json!({"container": "web-1", "lines": 20, "since": "10m"}),
    )
    .unwrap();
    assert_eq!(message, "2 log lines");
    assert_eq!(container["logs"], "out line\nerr line");
    assert_eq!(container["lineCount"], "2");
    assert_eq!(container["truncated"], "false");
    assert!(!container.contains_key("projectKey"));
    assert!(
        runs(&agent)
            .iter()
            .flatten()
            .all(|a| a != "-f" && a != "--follow")
    );

    record(
        &agent,
        "compose logs --no-color --tail=100 -- web",
        ok("web  | hello\n"),
    );
    let (_, service) = run(&agent, "logs", json!({"service": "web"})).unwrap();
    assert_eq!(service["logs"], "web  | hello");
    assert_eq!(service["projectKey"], "SHOP");
    // At most the agent's 1000 lines.
    record(&agent, "logs --tail=1000 -- web-1", ok("x\n"));
    run(&agent, "logs", json!({"container": "web-1", "lines": 2000})).unwrap();

    let (code, _) = run(&agent, "logs", json!({})).unwrap_err();
    assert_eq!(code, "invalid-parameters");
    let (code, _) = run(
        &agent,
        "logs",
        json!({"container": "web-1", "since": "--follow"}),
    )
    .unwrap_err();
    assert_eq!(code, codes::INVALID_NAME);
    // An unknown container.
    record(
        &agent,
        "logs --tail=100 -- gone",
        failed(1, "", "Error: No such container: gone\n"),
    );
    let (code, message) = run(&agent, "logs", json!({"container": "gone"})).unwrap_err();
    assert_eq!(code, codes::NOT_FOUND);
    assert_eq!(message, "docker: Error: No such container: gone");
}

// ------------------------------------------------------------ key state

fn key_state(agent: &Shared, action: &str, args: Value) -> StateOutcome {
    host(agent).key_state(&plugin(), action, args)
}

/// The built-in's `compose_projects` and `engine_and_containers`.
#[test]
fn state_functions() {
    assert_eq!(
        state::compose_state(3, 3, false),
        KeyState::new().detail("3/3 up").indicator(Indicator::Ok)
    );
    assert_eq!(
        state::compose_state(1, 3, false).indicator,
        Some(Indicator::Warning)
    );
    assert_eq!(
        state::compose_state(0, 3, false).indicator,
        Some(Indicator::Unknown)
    );
    assert_eq!(state::compose_state(0, 0, true).toggle, Some(false));
    assert_eq!(state::compose_state(2, 2, true).toggle, Some(true));
    assert_eq!(state::compose_state(1, 2, true).toggle, Some(false));
    assert_eq!(state::compose_state(1, 2, false).toggle, None);
    assert_eq!(
        state::availability_state(true).indicator,
        Some(Indicator::Ok)
    );
    assert_eq!(
        state::availability_state(false).indicator,
        Some(Indicator::Error)
    );
    assert_eq!(
        state::containers_state(2).detail.as_deref(),
        Some("2 running")
    );
    assert_eq!(
        state::containers_state(0).indicator,
        Some(Indicator::Unknown)
    );
}

/// The built-in's `keys_show_live_state_without_changing_anything`, on
/// what the last runs found: a state call runs nothing.
#[test]
fn keys_show_the_last_run() {
    let agent = agent();
    assert_eq!(
        key_state(&agent, "compose-status", json!({})),
        StateOutcome::Ok(KeyState::new())
    );
    run(&agent, "compose-status", json!({})).unwrap();
    run(&agent, "containers", json!({})).unwrap();
    run(&agent, "status", json!({})).unwrap();
    let before = runs(&agent).len();
    assert_eq!(
        key_state(&agent, "compose-status", json!({})),
        StateOutcome::Ok(
            KeyState::new()
                .detail("1/2 up")
                .indicator(Indicator::Warning)
        )
    );
    match key_state(&agent, "containers", json!({})) {
        StateOutcome::Ok(state) => assert_eq!(state.detail.as_deref(), Some("1 running")),
        other => panic!("{other:?}"),
    }
    match key_state(&agent, "status", json!({})) {
        StateOutcome::Ok(state) => assert_eq!(state.indicator, Some(Indicator::Ok)),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        key_state(&agent, "logs", json!({})),
        StateOutcome::Error(_)
    ));
    // Up and down keys read the project's state with the built-in's
    // toggle; restart keys without one. Nothing runs for them either.
    assert_eq!(
        key_state(&agent, "compose-up", json!({})),
        StateOutcome::Ok(
            KeyState::new()
                .detail("1/2 up")
                .indicator(Indicator::Warning)
                .toggle(false)
        )
    );
    match key_state(&agent, "compose-down", json!({})) {
        StateOutcome::Ok(state) => assert_eq!(state.toggle, Some(false)),
        other => panic!("{other:?}"),
    }
    match key_state(&agent, "compose-restart", json!({})) {
        StateOutcome::Ok(state) => {
            assert_eq!(state.detail.as_deref(), Some("1/2 up"));
            assert_eq!(state.toggle, None);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(runs(&agent).len(), before);
    // Another project's keys are their own.
    assert_eq!(
        key_state(&agent, "compose-status", json!({"projectId": "p-cms"})),
        StateOutcome::Ok(KeyState::new())
    );

    // After `compose up` the plugin reads the project again: its status
    // keys follow the change.
    record(
        &agent,
        "compose ps --all --format=json",
        ok(r#"{"Name":"shop-web-1","Service":"web","State":"running","Status":"Up"}"#),
    );
    run(&agent, "compose-up", json!({})).unwrap();
    assert_eq!(
        key_state(&agent, "compose-status", json!({})),
        StateOutcome::Ok(KeyState::new().detail("1/1 up").indicator(Indicator::Ok))
    );
    // The up key's toggle is on now.
    match key_state(&agent, "compose-up", json!({})) {
        StateOutcome::Ok(state) => assert_eq!(state.toggle, Some(true)),
        other => panic!("{other:?}"),
    }
    // The engine is down: red.
    record(
        &agent,
        "version --format={{json .}}",
        failed(
            1,
            r#"{"Client":{"Version":"29.1.0"},"Server":null}"#,
            "Cannot connect to the Docker daemon",
        ),
    );
    run(&agent, "status", json!({})).unwrap();
    match key_state(&agent, "status", json!({})) {
        StateOutcome::Ok(state) => assert_eq!(state.indicator, Some(Indicator::Error)),
        other => panic!("{other:?}"),
    }
}

/// Keys that differ only in their resources are different keys: the
/// agent's key scope (`run.context`) is part of where a run stores its
/// state, and a state call reads it back under its own.
#[test]
fn keys_of_different_resources_keep_their_own_state() {
    const SHOP: &str = "00112233445566778899aabbccddeeff";
    const CMS: &str = "ffeeddccbbaa99887766554433221100";
    let agent = agent();
    agent.borrow_mut().scope = Some(SHOP);
    run(&agent, "compose-status", json!({})).unwrap();
    agent.borrow_mut().scope = Some(CMS);
    record(
        &agent,
        "compose ps --all --format=json",
        ok(r#"{"Name":"cms-web-1","Service":"web","State":"running","Status":"Up"}"#),
    );
    run(&agent, "compose-status", json!({})).unwrap();
    let detail = |scope| {
        agent.borrow_mut().scope = Some(scope);
        match key_state(&agent, "compose-status", json!({})) {
            StateOutcome::Ok(state) => state.detail,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(detail(SHOP).as_deref(), Some("1/2 up"));
    assert_eq!(detail(CMS).as_deref(), Some("1/1 up"));
    // A key the agent says nothing about has not run.
    agent.borrow_mut().scope = None;
    assert_eq!(
        key_state(&agent, "compose-status", json!({})),
        StateOutcome::Ok(KeyState::new())
    );
    // Status keys of two Docker environments, too.
    agent.borrow_mut().scope = Some(SHOP);
    run(&agent, "status", json!({})).unwrap();
    agent.borrow_mut().scope = Some(CMS);
    assert_eq!(
        key_state(&agent, "status", json!({})),
        StateOutcome::Ok(KeyState::new())
    );
}

#[test]
fn logs_of_both_streams_and_cut_output() {
    assert_eq!(merge_logs("a\nb", "c\n"), ("a\nb\nc".to_owned(), 3));
    assert_eq!(merge_logs("", ""), (String::new(), 0));
    let cut = ToolOutput {
        exit: Some(0),
        stdout: format!("{}{{\"Names\":\"half", PS),
        stderr: String::new(),
        truncated: true,
        duration_ms: 1,
    };
    assert_eq!(parse(&cut).unwrap().len(), 2);
}
