//! Native unit tests: the definition agrees with manifest.json, the
//! manifest and its translations are valid, and every action behaves, with
//! the agent replaced by `MockHost`. The run through the real agent, its
//! handles and its launcher is `apps/agent/tests/plugins_workspace.rs`.

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

/// The manifest as the plugin sees its parameters: a `resource`
/// parameter is an optional text (its handle).
fn as_the_guest_sees_it(mut manifest: Value) -> Value {
    for action in manifest["contributes"]["actions"].as_array_mut().unwrap() {
        for param in action["parameters"].as_array_mut().unwrap() {
            if param["kind"]["type"] == "resource" {
                param["kind"] = json!({"type": "text"});
                param["required"] = json!(false);
            }
        }
    }
    manifest
}

#[test]
fn the_definition_matches_the_manifest() {
    assert_eq!(
        manifest_mismatches(&plugin(), &as_the_guest_sees_it(manifest())),
        Vec::<String>::new()
    );
    plugin().validate().unwrap();
}

#[test]
fn the_manifest_is_valid_with_minimal_permissions() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&manifest())
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let kinds: Vec<Value> = manifest()["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let mut p = p.clone();
            p.as_object_mut().unwrap().remove("justification");
            p
        })
        .collect();
    // No `paths`: the plugin names bindings, never their paths. No hosts:
    // it opens only what the user typed into the action.
    assert_eq!(
        kinds,
        [
            json!({"kind": "resources", "open": true}),
            json!({"kind": "openUrl", "hosts": [], "fromParams": true}),
            json!({"kind": "profile", "read": ["project"]}),
        ]
    );
    // Projects is optional, and no action requires it: the project roles
    // resolve through the core's project context, which the built-in
    // projects module provides as well until it is migrated.
    let manifest = manifest();
    assert_eq!(manifest["dependencies"][0]["id"], "deckpilot.projects");
    assert_eq!(manifest["dependencies"][0]["optional"], true);
    assert!(
        manifest["contributes"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a.get("requiresDependencies").is_none())
    );
}

/// Flattened `key → text` of a locale file.
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

/// Every `t:` key the manifest uses.
fn manifest_keys(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            if let Some(key) = s.strip_prefix("t:") {
                out.push(key.to_owned());
            }
        }
        Value::Array(items) => items.iter().for_each(|v| manifest_keys(v, out)),
        Value::Object(map) => map.values().for_each(|v| manifest_keys(v, out)),
        _ => {}
    }
}

#[test]
fn every_text_is_translated_into_english_and_polish() {
    let (en, pl) = (flat(EN), flat(PL));
    assert_eq!(
        en.keys().collect::<Vec<_>>(),
        pl.keys().collect::<Vec<_>>(),
        "en and pl have the same keys"
    );
    let mut used = Vec::new();
    manifest_keys(&manifest(), &mut used);
    for key in &used {
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

/// What a stand-in agent knows: bound parameters and roles (by name, with
/// their kind), the active project, and what the plugin asked it to open.
#[derive(Default)]
struct World {
    params: BTreeMap<&'static str, ResourceKind>,
    roles: BTreeMap<&'static str, ResourceKind>,
    project: Option<&'static str>,
    /// Refuse every resolve as `not-permitted`.
    no_resources: bool,
    /// Refuse the nth open (from 0) as throttled.
    throttle_from: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Opened {
    Resource {
        what: String,
        with: Option<String>,
        position: Option<Position>,
        editor: Option<EditorPlan>,
    },
    Url {
        index: Option<u32>,
    },
}

struct Agent {
    host: MockHost,
    opened: Rc<RefCell<Vec<Opened>>>,
}

fn agent(world: World) -> Agent {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let seen = opened.clone();
    // Handles by label: a hex string unique enough for the test.
    let handles: Rc<RefCell<BTreeMap<String, String>>> = Rc::default();
    let host = MockHost::new(move |request| {
        let issue = |label: &str| {
            let mut handles = handles.borrow_mut();
            let n = handles.len();
            handles
                .entry(format!("{:032x}", n + 1))
                .or_insert_with(|| label.to_owned());
            format!("{:032x}", n + 1)
        };
        let label_of = |handle: &str, handles: &Rc<RefCell<BTreeMap<String, String>>>| {
            handles.borrow().get(handle).cloned().unwrap_or_default()
        };
        match request {
            CapabilityRequest::ResourceResolve(r) => {
                if world.no_resources {
                    return CallResult::error(
                        CallErrorCode::NotPermitted,
                        "not-permitted: the resources are not permitted",
                    );
                }
                let (name, kind) = match (&r.param, &r.role) {
                    (Some(p), _) => match world.params.get(p.as_str()) {
                        Some(kind) => (p.clone(), *kind),
                        None => {
                            return CallResult::error(
                                CallErrorCode::Unavailable,
                                format!("resource-not-set: the parameter `{p}` is not set"),
                            );
                        }
                    },
                    (None, Some(role)) => {
                        if world.project.is_none() {
                            return CallResult::error(
                                CallErrorCode::Unavailable,
                                "no-active-project: no project is active",
                            );
                        }
                        match world.roles.get(role.as_str()) {
                            Some(kind) => (role.clone(), *kind),
                            None => {
                                return CallResult::error(
                                    CallErrorCode::Failed,
                                    format!(
                                        "resource-role-missing: the project does not define \
                                         the resource role `{role}`"
                                    ),
                                );
                            }
                        }
                    }
                    _ => unreachable!(),
                };
                if !r.kinds.contains(&kind) {
                    return CallResult::error(
                        CallErrorCode::NotPermitted,
                        format!("resource-kind-mismatch: `{name}` is not of the kind asked for"),
                    );
                }
                let label = format!("{name}-binding");
                CallResult::Ok(json!({"handle": issue(&label), "kind": kind, "label": label}))
            }
            CapabilityRequest::ResourceOpen(o) => {
                let n = seen.borrow().len();
                if world.throttle_from.is_some_and(|from| n >= from) {
                    return CallResult::error(CallErrorCode::Throttled, "rate limited");
                }
                // Like the agent: a line needs a file (here: a path with a
                // dot in its last name).
                if o.position.is_some()
                    && o.path
                        .as_deref()
                        .is_some_and(|p| !p.rsplit('/').next().unwrap_or_default().contains('.'))
                {
                    return CallResult::error(
                        CallErrorCode::BadRequest,
                        "invalid-position: a line needs a file, not a folder",
                    );
                }
                seen.borrow_mut().push(Opened::Resource {
                    what: match &o.path {
                        Some(path) => format!("{}/{path}", label_of(&o.handle, &handles)),
                        None => label_of(&o.handle, &handles),
                    },
                    with: o.with.as_ref().map(|w| label_of(w, &handles)),
                    position: o.position,
                    editor: o.editor,
                });
                CallResult::Ok(Value::Null)
            }
            CapabilityRequest::OpenUrlParam(o) => {
                let n = seen.borrow().len();
                if world.throttle_from.is_some_and(|from| n >= from) {
                    return CallResult::error(CallErrorCode::Throttled, "rate limited");
                }
                assert_eq!(o.param, "urls");
                seen.borrow_mut().push(Opened::Url { index: o.index });
                CallResult::Ok(Value::Null)
            }
            CapabilityRequest::ProfileRead(_) => CallResult::Ok(match world.project {
                Some(key) => json!({"project": {"id": "p1", "key": key, "name": "Shop"}}),
                None => json!({}),
            }),
            CapabilityRequest::LocaleTranslate(_) => {
                CallResult::error(CallErrorCode::UnsupportedOp, "an older agent")
            }
            other => panic!("unexpected request {other:?}"),
        }
    });
    Agent { host, opened }
}

/// `(ok, code, message, data)`.
type Answer = (bool, String, String, BTreeMap<String, String>);

impl Agent {
    fn run(&mut self, action: &str, params: Value) -> Answer {
        match self.host.invoke(&plugin(), action, params) {
            ActionOutcome::Ok { message, data } => (
                true,
                String::new(),
                message.unwrap_or_default(),
                data.into_iter()
                    .map(|(k, v)| {
                        let v = serde_json::to_value(v).unwrap();
                        (k, v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                    })
                    .collect(),
            ),
            ActionOutcome::Failed { code, message, .. } => {
                (false, code, message.unwrap_or_default(), BTreeMap::new())
            }
            other => panic!("{other:?}"),
        }
    }

    fn opened(&self) -> Vec<Opened> {
        self.opened.borrow().clone()
    }
}

fn resource(
    what: &str,
    with: Option<&str>,
    position: Option<Position>,
    editor: Option<EditorPlan>,
) -> Opened {
    Opened::Resource {
        what: format!("{what}-binding"),
        with: with.map(|w| format!("{w}-binding")),
        position,
        editor,
    }
}

// ------------------------------------------------------- open-in-editor

/// The built-in's `path`: a file inside the bound folder (the agent checks
/// that it stays inside), at a line; a path with a bound file is refused
/// before anything opens.
#[test]
fn open_in_editor_opens_a_path_inside_the_bound_folder() {
    let world = |location| World {
        params: [
            ("application", ResourceKind::Application),
            ("location", location),
        ]
        .into(),
        ..World::default()
    };
    let mut agent = agent(world(ResourceKind::Directory));
    let (ok, _, message, data) = agent.run(
        "open-in-editor",
        json!({"application": "a".repeat(32), "location": "b".repeat(32),
               "path": " src/main.rs ", "editor": "zed", "line": 7}),
    );
    assert!(ok, "{message}");
    assert_eq!(
        message,
        "opened location-binding/src/main.rs in application-binding"
    );
    assert_eq!(data["path"], "location-binding/src/main.rs");
    assert_eq!(data["located"], "true");
    assert_eq!(
        agent.opened(),
        [Opened::Resource {
            what: "location-binding/src/main.rs".into(),
            with: Some("application-binding".into()),
            position: Some(Position {
                line: 7,
                column: None
            }),
            editor: Some(EditorPlan::Zed),
        }]
    );
    // A folder inside the folder, without a line.
    let mut agent = self::agent(world(ResourceKind::Directory));
    let (ok, _, _, data) = agent.run(
        "open-in-editor",
        json!({"application": "a".repeat(32), "location": "b".repeat(32), "path": "docs"}),
    );
    assert!(ok);
    assert_eq!(data["path"], "location-binding/docs");

    let mut agent = self::agent(world(ResourceKind::File));
    let (ok, code, message, _) = agent.run(
        "open-in-editor",
        json!({"application": "a".repeat(32), "location": "b".repeat(32), "path": "x.rs"}),
    );
    assert!(!ok);
    assert_eq!(
        (code.as_str(), message.as_str()),
        (codes::INVALID_PATH, messages::PATH_NEEDS_FOLDER.1)
    );
    assert_eq!(agent.opened(), []);
}

#[test]
fn open_in_editor_opens_a_bound_file_at_a_line_in_the_bound_editor() {
    let mut agent = agent(World {
        params: [
            ("application", ResourceKind::Application),
            ("location", ResourceKind::File),
        ]
        .into(),
        ..World::default()
    });
    let handles = json!({"application": "a".repeat(32), "location": "b".repeat(32)});
    let (ok, _, message, data) = agent.run(
        "open-in-editor",
        json!({"application": handles["application"], "location": handles["location"],
               "editor": "vscode", "line": 12, "column": 3}),
    );
    assert!(ok, "{message}");
    assert_eq!(message, "opened location-binding in application-binding");
    assert_eq!(data["path"], "location-binding");
    assert_eq!(data["application"], "application-binding");
    assert_eq!(data["located"], "true");
    assert_eq!(
        agent.opened(),
        [resource(
            "location",
            Some("application"),
            Some(Position {
                line: 12,
                column: Some(3)
            }),
            Some(EditorPlan::Vscode)
        )]
    );
}

#[test]
fn open_in_editor_defaults_to_the_projects_repository() {
    let mut agent = agent(World {
        params: [("application", ResourceKind::Application)].into(),
        roles: [("repository", ResourceKind::Directory)].into(),
        project: Some("SHOP"),
        ..World::default()
    });
    let (ok, _, message, data) = agent.run(
        "open-in-editor",
        json!({"application": "a".repeat(32), "editor": "other"}),
    );
    assert!(ok, "{message}");
    assert_eq!(data["path"], "repository-binding");
    assert_eq!(data["located"], "false");
    assert_eq!(
        agent.opened(),
        [resource(
            "repository",
            Some("application"),
            None,
            Some(EditorPlan::Other)
        )]
    );
    // An unknown editor kind passes no line: `located` stays false.
    let mut agent2 = self::agent(World {
        params: [
            ("application", ResourceKind::Application),
            ("location", ResourceKind::File),
        ]
        .into(),
        ..World::default()
    });
    let (_, _, _, data) = agent2.run(
        "open-in-editor",
        json!({"application": "a".repeat(32), "location": "b".repeat(32), "line": 4}),
    );
    assert_eq!(data["located"], "false");
}

#[test]
fn open_in_editor_refuses_what_the_built_in_refuses() {
    let world = || World {
        params: [
            ("application", ResourceKind::Application),
            ("location", ResourceKind::Directory),
        ]
        .into(),
        ..World::default()
    };
    let base = json!({"application": "a".repeat(32), "location": "b".repeat(32)});
    let with = |extra: Value| {
        let mut params = base.clone();
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        params
    };
    for (params, code, message) in [
        (
            with(json!({"line": 3})),
            codes::INVALID_POSITION,
            "a line needs a file, not a folder",
        ),
        (
            with(json!({"column": 3})),
            codes::INVALID_POSITION,
            "a column needs a line",
        ),
        // The agent refuses a line for a folder inside the folder.
        (
            with(json!({"path": "src", "line": 3})),
            codes::INVALID_POSITION,
            "a line needs a file, not a folder",
        ),
    ] {
        let mut agent = agent(world());
        let (ok, got, text, _) = agent.run("open-in-editor", params.clone());
        assert!(!ok, "{params}");
        assert_eq!((got.as_str(), text.as_str()), (code, message), "{params}");
        assert_eq!(agent.opened(), [], "{params}");
    }
    // Without an active project the repository is the agent's reason.
    let mut agent = agent(World {
        params: [("application", ResourceKind::Application)].into(),
        ..World::default()
    });
    let (ok, code, message, _) =
        agent.run("open-in-editor", json!({"application": "a".repeat(32)}));
    assert!(!ok);
    assert_eq!(
        (code.as_str(), message.as_str()),
        ("no-active-project", "no project is active")
    );
    // The agent's refusal to open (a run the user did not start) is the
    // action's failure.
    let mut agent = self::agent(World {
        throttle_from: Some(0),
        ..world()
    });
    let (ok, code, _, _) = agent.run("open-in-editor", base.clone());
    assert!(!ok);
    assert_eq!(code, "throttled");
}

// ----------------------------------------------------------- open-urls

#[test]
fn open_urls_opens_each_line_the_user_typed_by_its_index() {
    let mut agent = agent(World::default());
    let (ok, _, message, data) = agent.run(
        "open-urls",
        json!({"urls": "https://example.com/a\n\n  http://localhost:3000/  \nhttps://ci.example/x?token=1"}),
    );
    assert!(ok, "{message}");
    assert_eq!(message, "opened 3 pages");
    assert_eq!(data["opened"], "3");
    assert_eq!(data["failed"], "0");
    assert_eq!(
        data["urls"],
        "https://example.com/a\nhttp://localhost:3000/\nhttps://ci.example/x?token=1"
    );
    // Indexes among the non-empty lines, as the agent counts them.
    assert_eq!(
        agent.opened(),
        [
            Opened::Url { index: Some(0) },
            Opened::Url { index: Some(1) },
            Opened::Url { index: Some(2) }
        ]
    );
}

#[test]
fn open_urls_checks_every_page_before_opening_any() {
    for (urls, code) in [
        ("", codes::NO_URLS),
        ("  \n ", codes::NO_URLS),
        (
            "https://a.example\nmailto:team@example.com",
            codes::INVALID_URL,
        ),
        ("javascript:alert(1)", codes::INVALID_URL),
        ("example.com", codes::INVALID_URL),
        (&"https://a.example/\n".repeat(11), codes::TOO_MANY_URLS),
    ] {
        let mut agent = agent(World::default());
        let (ok, got, _, _) = agent.run("open-urls", json!({"urls": urls}));
        assert!(!ok, "{urls}");
        assert_eq!(got, code, "{urls}");
        assert_eq!(agent.opened(), [], "{urls}");
    }
}

#[test]
fn open_urls_reports_pages_it_could_not_open_and_the_default_browser() {
    let mut agent = agent(World {
        throttle_from: Some(1),
        params: [("browser", ResourceKind::Application)].into(),
        ..World::default()
    });
    let (ok, _, message, data) = agent.run(
        "open-urls",
        json!({"urls": "https://a.example/\nhttps://b.example/", "browser": "c".repeat(32)}),
    );
    assert!(ok);
    assert_eq!(
        message,
        "opened 1 pages; 1 could not be opened (in the default browser)"
    );
    assert_eq!(data["opened"], "1");
    assert_eq!(data["failed"], "1");
    assert_eq!(data["urls"], "https://a.example/");
    // Nothing opened: the first refusal is the failure.
    let mut agent = self::agent(World {
        throttle_from: Some(0),
        ..World::default()
    });
    let (ok, code, _, _) = agent.run("open-urls", json!({"urls": "https://a.example/"}));
    assert!(!ok);
    assert_eq!(code, "throttled");
}

// -------------------------------------------------------- open-project

fn project_world() -> World {
    World {
        roles: [
            ("repository", ResourceKind::Directory),
            ("editor", ResourceKind::Application),
            ("docs", ResourceKind::Url),
            ("dashboard", ResourceKind::Url),
            ("notes", ResourceKind::Directory),
        ]
        .into(),
        project: Some("SHOP"),
        ..World::default()
    }
}

#[test]
fn open_project_opens_the_repository_docs_and_pages_of_the_active_project() {
    let mut agent = agent(project_world());
    let (ok, _, message, data) = agent.run(
        "open-project",
        json!({"editor": "zed", "editorRole": "editor", "repositoryRole": "repository",
               "docsRole": "docs", "urlRoles": "dashboard\nci"}),
    );
    assert!(ok, "{message}");
    assert_eq!(
        message,
        "opened repository, docs, dashboard of SHOP; missing: ci: the project does not define \
         the resource role `ci`"
    );
    assert_eq!(data["projectKey"], "SHOP");
    assert_eq!(data["opened"], "3");
    assert_eq!(data["openedRoles"], "repository\ndocs\ndashboard");
    assert_eq!(data["missingCount"], "1");
    assert_eq!(
        agent.opened(),
        [
            resource("repository", Some("editor"), None, Some(EditorPlan::Zed)),
            resource("docs", None, None, None),
            resource("dashboard", None, None, None),
        ]
    );
}

#[test]
fn open_project_reports_missing_and_wrong_kinds_and_fails_with_nothing() {
    // A folder as the docs opens with the system's handler; a page role
    // bound to a folder is missing.
    let mut agent = agent(project_world());
    let (ok, _, _, data) = agent.run(
        "open-project",
        json!({"repositoryRole": "", "docsRole": "notes", "urlRoles": "notes"}),
    );
    assert!(ok);
    assert_eq!(data["openedRoles"], "notes");
    assert_eq!(
        data["missing"],
        "notes: `notes` is not of the kind asked for"
    );
    // Nothing chosen.
    let mut agent = self::agent(project_world());
    let (ok, code, message, _) = agent.run(
        "open-project",
        json!({"repositoryRole": " ", "docsRole": ""}),
    );
    assert!(!ok);
    assert_eq!(code, codes::NOTHING_OPENED);
    assert_eq!(
        message,
        "nothing of SHOP could be opened: no resources were chosen"
    );
    // No editor bound: the repository is missing, with why.
    let mut world = project_world();
    world.roles.remove("editor");
    let mut agent = self::agent(world);
    let (ok, code, message, _) = agent.run("open-project", json!({"docsRole": ""}));
    assert!(!ok);
    assert_eq!(code, codes::NOTHING_OPENED);
    assert_eq!(
        message,
        "nothing of SHOP could be opened: editor: the project does not define the resource \
         role `editor`"
    );
    // Too many pages.
    let mut agent = self::agent(project_world());
    let (ok, code, _, _) = agent.run(
        "open-project",
        json!({"urlRoles": "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk"}),
    );
    assert!(!ok);
    assert_eq!(code, codes::TOO_MANY_URLS);
}

#[test]
fn open_project_needs_an_active_project_and_its_resources() {
    let mut agent = agent(World::default());
    let (ok, code, message, _) = agent.run("open-project", json!({}));
    assert!(!ok);
    assert_eq!(
        (code.as_str(), message.as_str()),
        ("no-active-project", "no project is active")
    );
    assert_eq!(agent.opened(), []);
    let mut agent = self::agent(World {
        no_resources: true,
        ..project_world()
    });
    let (ok, code, _, _) = agent.run("open-project", json!({}));
    assert!(!ok);
    assert_eq!(code, "not-permitted");
}

#[test]
fn messages_follow_the_apps_language() {
    let mut host = MockHost::localized(
        "pl",
        &[("messages.opened-pages", "otwarte strony: {{count}}")],
    );
    match host.invoke(
        &plugin(),
        "open-urls",
        json!({"urls": "https://a.example/"}),
    ) {
        ActionOutcome::Ok { message, .. } => {
            assert_eq!(message.as_deref(), Some("otwarte strony: 1"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn refusals_carry_the_agents_reason_code() {
    let mut host = MockHost::new(|_| {
        CallResult::error(
            CallErrorCode::Unavailable,
            "no-active-project: no project is active",
        )
    });
    let error = host
        .context()
        .resource_role("repository", &[ResourceKind::Directory])
        .unwrap_err();
    assert_eq!(
        refusal(&error),
        (
            "no-active-project".to_owned(),
            "no project is active".to_owned()
        )
    );
    let mut host = MockHost::new(|_| CallResult::error(CallErrorCode::Throttled, "rate limited"));
    let error = host.context().open_url_param("urls", None).unwrap_err();
    assert_eq!(refusal(&error).0, "throttled");
}
