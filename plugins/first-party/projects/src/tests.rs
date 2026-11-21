//! Native unit tests: the definition agrees with manifest.json, the
//! manifest and its translations are valid, and every action behaves, with
//! the agent replaced by `MockHost` (a profile's projects, the active one
//! and their roles, switched as the agent's `ProjectSwitcher` does). The
//! run through the real agent is `apps/agent/tests/plugins_projects.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};
use deckpilot_plugin_sdk::{ActionOutcome, CallErrorCode, CallResult, CapabilityRequest};
use serde_json::{Value, json};

use super::*;

const MANIFEST: &str = include_str!("../manifest.json");
const EN: &str = include_str!("../locales/en.json");
const PL: &str = include_str!("../locales/pl.json");
const SCHEMA: &str = include_str!("../../../schemas/plugin-manifest.schema.json");

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).unwrap()
}

#[test]
fn the_definition_matches_the_manifest() {
    assert_eq!(
        manifest_mismatches(&plugin(), &manifest()),
        Vec::<String>::new()
    );
    plugin().validate().unwrap();
}

#[test]
fn the_manifest_is_valid_and_declares_the_projects_screen() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&manifest())
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let m = manifest();
    let kinds: Vec<Value> = m["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let mut p = p.clone();
            p.as_object_mut().unwrap().remove("justification");
            p
        })
        .collect();
    assert_eq!(
        kinds,
        [
            json!({"kind": "projectContext", "select": true}),
            json!({"kind": "resources", "paths": true}),
            json!({"kind": "profile", "read": ["project"]}),
            json!({"kind": "hostViews"}),
        ]
    );
    assert_eq!(
        m["contributes"]["views"],
        json!([{"id": "projects", "kind": "projects", "context": "profile"}])
    );
    assert_eq!(m["contributes"]["navigation"][0]["view"], "projects");
    assert_eq!(m["contributes"]["statusItems"], json!(["activeProject"]));
    assert_eq!(m["contributes"]["dashboardCards"], json!(["activeProject"]));
    // The host-published variables: the built-in's `projects.current.*`.
    let variables: Vec<(&str, &str)> = m["contributes"]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["id"].as_str().unwrap(), v["source"].as_str().unwrap()))
        .collect();
    assert_eq!(
        variables,
        [
            ("current.id", "host:project.id"),
            ("current.key", "host:project.key"),
            ("current.name", "host:project.name"),
        ]
    );
    // Every action of the built-in, the deprecated ones flagged and
    // pointing at their replacement.
    let actions: Vec<(&str, bool)> = m["contributes"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["id"].as_str().unwrap(), a["deprecated"] == true))
        .collect();
    assert_eq!(
        actions,
        [
            ("cycle", false),
            ("select", false),
            ("current", false),
            ("repository", false),
            ("documentation", false),
            ("resolve-resource", false),
            ("next", true),
            ("previous", true),
            ("active", true),
        ]
    );
    assert_eq!(
        m["contributes"]["settings"]["fields"][0]["key"],
        WRAP_AROUND
    );
}

fn flat(text: &str) -> BTreeMap<String, String> {
    fn walk(prefix: &str, value: &Value, out: &mut BTreeMap<String, String>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(&key, v, out);
                }
            }
            Value::String(s) => {
                out.insert(prefix.to_owned(), s.clone());
            }
            other => panic!("{prefix}: {other}"),
        }
    }
    let mut out = BTreeMap::new();
    walk("", &serde_json::from_str(text).unwrap(), &mut out);
    out
}

#[test]
fn every_text_is_translated_into_english_and_polish() {
    let (en, pl) = (flat(EN), flat(PL));
    assert_eq!(en.keys().collect::<Vec<_>>(), pl.keys().collect::<Vec<_>>());
    for key in MANIFEST.split('"').filter_map(|s| s.strip_prefix("t:")) {
        assert!(en.contains_key(key), "en lacks {key}");
    }
    for (key, english) in messages::ALL {
        assert_eq!(
            en.get(&format!("messages.{key}")).map(String::as_str),
            Some(*english),
            "messages.{key}"
        );
    }
    assert_eq!(
        en.keys().filter(|k| k.starts_with("messages.")).count(),
        messages::ALL.len()
    );
}

// ------------------------------------------------------------ the agent

/// A profile's projects as the agent keeps them, and what it was asked.
#[derive(Debug, Clone)]
struct Profile {
    /// Key, name; ids are `p<index>`.
    projects: Vec<(&'static str, &'static str)>,
    active: Option<usize>,
    /// Bound roles: `(project index, role) → (kind, label, path)`.
    roles: BTreeMap<(usize, &'static str), (ResourceKind, &'static str, Option<&'static str>)>,
    wrap: bool,
    /// A run the user did not start: switches are refused.
    not_user: bool,
}

fn shop() -> Profile {
    Profile {
        projects: vec![("SHOP", "Shop"), ("CMR", "Customer"), ("CMS", "Content")],
        active: Some(0),
        roles: [
            (
                (0, "repository"),
                (ResourceKind::Directory, "shop.repo", Some("/src/shop")),
            ),
            ((0, "docs"), (ResourceKind::Url, "shop.docs", None)),
            (
                (0, "compose"),
                (ResourceKind::DockerEnvironment, "shop.docker", None),
            ),
            (
                (1, "repository"),
                (ResourceKind::Directory, "cmr.repo", Some("/src/cmr")),
            ),
        ]
        .into(),
        wrap: true,
        not_user: false,
    }
}

fn info(profile: &Profile, index: usize) -> Value {
    let (key, name) = profile.projects[index];
    json!({"id": format!("p{index}"), "key": key, "name": name})
}

struct Agent {
    host: MockHost,
    profile: Rc<RefCell<Profile>>,
    switches: Rc<RefCell<Vec<String>>>,
}

fn agent(profile: Profile) -> Agent {
    let state = Rc::new(RefCell::new(profile));
    let switches = Rc::new(RefCell::new(Vec::new()));
    let (seen, switched) = (state.clone(), switches.clone());
    let host = MockHost::new(move |request| {
        let mut p = seen.borrow_mut();
        let refuse_switch = || {
            CallResult::error(
                CallErrorCode::NotPermitted,
                "privileged operations need a run the user started",
            )
        };
        match request {
            CapabilityRequest::SettingsGet(_) => CallResult::Ok(json!({ WRAP_AROUND: p.wrap })),
            CapabilityRequest::ProjectList(_) => CallResult::Ok(json!({
                "projects": (0..p.projects.len()).map(|i| info(&p, i)).collect::<Vec<_>>()
            })),
            CapabilityRequest::ProjectCurrent(_) => {
                CallResult::Ok(p.active.map_or(Value::Null, |i| info(&p, i)))
            }
            CapabilityRequest::ProjectSelect(s) => {
                if p.not_user {
                    return refuse_switch();
                }
                let Some(index) = (0..p.projects.len()).find(|i| format!("p{i}") == s.id) else {
                    return CallResult::error(
                        CallErrorCode::Failed,
                        "not-found: the active profile has no such project",
                    );
                };
                p.active = Some(index);
                switched.borrow_mut().push(format!("p{index}"));
                CallResult::Ok(info(&p, index))
            }
            CapabilityRequest::ProjectCycle(c) => {
                if p.not_user {
                    return refuse_switch();
                }
                let n = p.projects.len();
                if n == 0 {
                    return CallResult::error(
                        CallErrorCode::Failed,
                        "no-projects: this profile has no projects",
                    );
                }
                let wrap = c.wrap.unwrap_or(true);
                let next = match (c.direction, p.active) {
                    (CycleDirection::Next, Some(i)) if i + 1 == n && !wrap => {
                        return CallResult::error(
                            CallErrorCode::Failed,
                            "at-last-project: already at the last project",
                        );
                    }
                    (CycleDirection::Previous, Some(0)) if !wrap => {
                        return CallResult::error(
                            CallErrorCode::Failed,
                            "at-first-project: already at the first project",
                        );
                    }
                    (CycleDirection::Next, Some(i)) => (i + 1) % n,
                    (CycleDirection::Previous, Some(i)) => (i + n - 1) % n,
                    (CycleDirection::Next, None) => 0,
                    (CycleDirection::Previous, None) => n - 1,
                };
                p.active = Some(next);
                switched.borrow_mut().push(format!("p{next}"));
                CallResult::Ok(info(&p, next))
            }
            CapabilityRequest::ResourceResolve(r) => {
                let role = r.role.as_deref().unwrap();
                let index = match &r.project_id {
                    Some(id) => (0..p.projects.len())
                        .find(|i| format!("p{i}") == *id)
                        .unwrap(),
                    None => match p.active {
                        Some(i) => i,
                        None => {
                            return CallResult::error(
                                CallErrorCode::Unavailable,
                                "no-active-project: no project is active",
                            );
                        }
                    },
                };
                let Some((kind, label, path)) = p.roles.get(&(index, role)).copied() else {
                    return CallResult::error(
                        CallErrorCode::Failed,
                        format!(
                            "resource-role-missing: the project does not define the resource \
                             role `{role}`"
                        ),
                    );
                };
                if !r.kinds.contains(&kind) {
                    return CallResult::error(
                        CallErrorCode::NotPermitted,
                        format!("resource-kind-mismatch: `{label}` is not of that kind"),
                    );
                }
                CallResult::Ok(
                    json!({"handle": "a".repeat(32), "kind": kind, "label": label,
                                      "displayPath": path}),
                )
            }
            CapabilityRequest::LocaleTranslate(_) => {
                CallResult::error(CallErrorCode::UnsupportedOp, "an older agent")
            }
            other => panic!("unexpected request {other:?}"),
        }
    });
    Agent {
        host,
        profile: state,
        switches,
    }
}

type Answer = Result<(String, BTreeMap<String, String>), (String, String)>;

impl Agent {
    fn run(&mut self, action: &str, params: Value) -> Answer {
        match self.host.invoke(&plugin(), action, params) {
            ActionOutcome::Ok { message, data } => Ok((
                message.unwrap_or_default(),
                data.into_iter()
                    .map(|(k, v)| {
                        let v = serde_json::to_value(v).unwrap();
                        (k, v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                    })
                    .collect(),
            )),
            ActionOutcome::Failed { code, message, .. } => Err((code, message.unwrap_or_default())),
            other => panic!("{other:?}"),
        }
    }

    fn active(&self) -> Option<usize> {
        self.profile.borrow().active
    }

    fn switches(&self) -> Vec<String> {
        self.switches.borrow().clone()
    }
}

fn key_of(answer: &Answer) -> &str {
    &answer.as_ref().unwrap().1["projectKey"]
}

// ----------------------------------------------------------- switching

#[test]
fn cycle_moves_in_either_direction_and_wraps() {
    let mut agent = agent(shop());
    let answer = agent.run("cycle", json!({"direction": "next"}));
    assert_eq!(answer.as_ref().unwrap().0, "switched to CMR");
    assert_eq!(key_of(&answer), "CMR");
    assert_eq!(answer.as_ref().unwrap().1["projectId"], "p1");
    assert_eq!(answer.as_ref().unwrap().1["projectName"], "Customer");
    assert_eq!(key_of(&agent.run("cycle", json!({}))), "CMS");
    assert_eq!(key_of(&agent.run("cycle", json!({}))), "SHOP", "wraps");
    assert_eq!(
        key_of(&agent.run("cycle", json!({"direction": "previous"}))),
        "CMS"
    );
    assert_eq!(agent.switches(), ["p1", "p2", "p0", "p2"]);
}

#[test]
fn cycle_follows_the_wrap_setting() {
    let mut agent = agent(Profile {
        wrap: false,
        active: Some(2),
        ..shop()
    });
    assert_eq!(
        agent
            .run("cycle", json!({"direction": "next"}))
            .unwrap_err(),
        (
            codes::AT_LAST.to_owned(),
            "already at the last project".to_owned()
        )
    );
    agent.profile.borrow_mut().active = Some(0);
    assert_eq!(
        agent.run("previous", json!({})).unwrap_err().0,
        codes::AT_FIRST
    );
    assert_eq!(agent.switches(), Vec::<String>::new());
}

#[test]
fn cycle_without_projects_fails_and_with_one_changes_nothing() {
    let mut agent = agent(Profile {
        projects: vec![],
        active: None,
        roles: BTreeMap::new(),
        ..shop()
    });
    assert_eq!(
        agent.run("cycle", json!({})).unwrap_err().0,
        codes::NO_PROJECTS
    );
    let mut agent = self::agent(Profile {
        projects: vec![("SHOP", "Shop")],
        ..shop()
    });
    let answer = agent.run("next", json!({}));
    assert_eq!(answer.as_ref().unwrap().0, "SHOP is the only project");
    assert_eq!(agent.switches(), Vec::<String>::new());
    // A single project that is not active yet is selected.
    let mut agent = self::agent(Profile {
        projects: vec![("SHOP", "Shop")],
        active: None,
        ..shop()
    });
    assert_eq!(key_of(&agent.run("cycle", json!({}))), "SHOP");
}

#[test]
fn next_and_previous_are_cycle() {
    let mut agent = agent(shop());
    assert_eq!(key_of(&agent.run("next", json!({}))), "CMR");
    assert_eq!(key_of(&agent.run("previous", json!({}))), "SHOP");
    assert_eq!(key_of(&agent.run("previous", json!({}))), "CMS");
}

#[test]
fn switching_is_refused_in_runs_the_user_did_not_start() {
    let mut agent = agent(Profile {
        not_user: true,
        ..shop()
    });
    for (action, params) in [
        ("cycle", json!({})),
        ("next", json!({})),
        ("previous", json!({})),
        ("select", json!({"projectId": "p1"})),
    ] {
        let (code, _) = agent.run(action, params).unwrap_err();
        assert_eq!(code, "not-permitted", "{action}");
    }
    assert_eq!(agent.active(), Some(0));
    assert_eq!(agent.switches(), Vec::<String>::new());
}

#[test]
fn select_switches_to_a_project_of_the_profile() {
    let mut agent = agent(shop());
    let answer = agent.run("select", json!({"projectId": "p2"}));
    assert_eq!(answer.as_ref().unwrap().0, "selected CMS");
    assert_eq!(agent.active(), Some(2));
    // Without a project: the current one, nothing changes.
    let answer = agent.run("select", json!({}));
    assert_eq!(answer.as_ref().unwrap().0, "CMS is active");
    assert_eq!(agent.switches(), ["p2"]);
    // A project of another profile.
    assert_eq!(
        agent.run("select", json!({"projectId": "p9"})).unwrap_err(),
        (
            codes::NOT_IN_PROFILE.to_owned(),
            "the project does not belong to the active profile".to_owned()
        )
    );
    let mut agent = self::agent(Profile {
        active: None,
        ..shop()
    });
    assert_eq!(
        agent.run("select", json!({})).unwrap_err().0,
        codes::NO_ACTIVE_PROJECT
    );
}

// ------------------------------------------------------------- reading

#[test]
fn current_and_active_report_the_project_without_changing_it() {
    let mut agent = agent(shop());
    for action in ["current", "active"] {
        let (message, data) = agent.run(action, json!({})).unwrap();
        assert_eq!(message, "SHOP (Shop)");
        assert_eq!(data["projectId"], "p0");
        assert_eq!(data["projectKey"], "SHOP");
        assert_eq!(data["projectName"], "Shop");
    }
    assert_eq!(agent.switches(), Vec::<String>::new());
    let mut agent = self::agent(Profile {
        active: None,
        ..shop()
    });
    assert_eq!(
        agent.run("current", json!({})).unwrap_err(),
        (
            codes::NO_ACTIVE_PROJECT.to_owned(),
            "no project is active".to_owned()
        )
    );
}

#[test]
fn resource_actions_resolve_roles_of_the_current_or_a_named_project() {
    let mut agent = agent(shop());
    let (message, data) = agent.run("repository", json!({})).unwrap();
    assert_eq!(message, "SHOP repository: /src/shop");
    assert_eq!(data["resource"], "repository");
    assert_eq!(data["resourceKey"], "shop.repo");
    assert_eq!(data["kind"], "directory");
    assert_eq!(data["value"], "/src/shop");
    // A page: its key, never its address.
    let (message, data) = agent.run("documentation", json!({})).unwrap();
    assert_eq!(message, "SHOP docs: shop.docs");
    assert_eq!(data["kind"], "url");
    assert_eq!(data["value"], "");
    // Docker environments through the second lookup.
    let (_, data) = agent
        .run("resolve-resource", json!({"resource": " compose "}))
        .unwrap();
    assert_eq!(data["kind"], "dockerEnvironment");
    assert_eq!(data["resource"], "compose");
    // Another project of the profile, named by the action.
    let (message, data) = agent.run("repository", json!({"projectId": "p1"})).unwrap();
    assert_eq!(message, "CMR repository: /src/cmr");
    assert_eq!(data["projectKey"], "CMR");
    assert_eq!(agent.active(), Some(0), "nothing switched");
}

#[test]
fn missing_resources_are_reported() {
    let mut agent = agent(shop());
    assert_eq!(
        agent
            .run("documentation", json!({"projectId": "p1"}))
            .unwrap_err()
            .0,
        "resource-role-missing"
    );
    assert_eq!(
        agent
            .run("resolve-resource", json!({"resource": "  "}))
            .unwrap_err(),
        (
            codes::INVALID_PARAMETERS.to_owned(),
            "no resource configured".to_owned()
        )
    );
    assert_eq!(
        agent
            .run("repository", json!({"projectId": "p7"}))
            .unwrap_err()
            .0,
        codes::NOT_IN_PROFILE
    );
    let mut agent = self::agent(Profile {
        active: None,
        ..shop()
    });
    assert_eq!(
        agent.run("repository", json!({})).unwrap_err().0,
        codes::NO_ACTIVE_PROJECT
    );
}

#[test]
fn messages_follow_the_apps_language() {
    let mut host = MockHost::localized("pl", &[("messages.current", "{{key}}: {{name}}")]);
    // `allow_all` answers no project: the current one is missing.
    match host.invoke(&plugin(), "current", json!({})) {
        ActionOutcome::Failed { code, .. } => assert_eq!(code, codes::NO_ACTIVE_PROJECT),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        fill(
            messages::RESOLVED.1,
            &[
                ("key", arg("A")),
                ("role", arg("docs")),
                ("value", arg("x"))
            ]
        ),
        "A docs: x"
    );
}
