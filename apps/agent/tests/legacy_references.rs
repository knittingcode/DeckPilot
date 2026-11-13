//! A database written by DeckPilot before module-owned variables (`main`
//! at 2939c4a, schema 9) whose actions, layout overrides and workflow node
//! overrides use the legacy ids `system.project.current.*` and
//! `system.work.session.*`, plus shared variables at every scope.
//!
//! `fixtures/legacy-references/` holds that database and `expected.json`,
//! what that agent did with it: the device status and key images of the
//! simulated MK.2, every action's result, a click whose binding overrides
//! the message, a workflow whose node overrides it, the shared variable
//! after a project change, and every result again with the projects and
//! work session modules disabled. Both were written by
//! `write_the_fixture_with_this_agent` (ignored; run it with
//! `DECKPILOT_LEGACY_FIXTURE=<dir>`) on that commit.
//!
//! The current agent must do exactly the same with the database and must
//! not rewrite a stored row while doing it.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_agent::GestureHooks;
use deckpilot_core::gesture::{BoundGestures, GestureFired, GestureTimings};
use deckpilot_core::variables::{
    ScopeKey, VariableDefinition, VariableKey, VariableRef, VariableScope, VariableType,
    VariableValue,
};
use deckpilot_core::{
    ActionInstanceId, ActionParameters, BindingTarget, ControlBinding, DeviceLayout, ModuleId,
    ModuleSettings, OnFailure, PageId, ParameterValue, Profile, Project, ProjectKey,
    RuntimeOutcome, Trigger, Workflow,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::{AgentPaths, Request, Response};
use deckpilot_modules::general;
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore};
use serde_json::{Value, json};
use support::{TIMEOUT, TempDir, TestAgent, registry};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/legacy-references"
);
/// Protects demonstration data only.
const PASSWORD: &str = "deckpilot-dev";
const SERIAL: &str = "LEGACY-1";

/// The actions, in the order they are run, and their messages.
const ACTIONS: [(&str, Message); 5] = [
    (
        "Project template",
        Message::Template(
            "{{ system.project.current.key }}|{{ system.project.current.id }}|{{ system.project.current.name }}",
        ),
    ),
    (
        "Project reference",
        Message::Reference("system.project.current.key"),
    ),
    (
        "Session template",
        Message::Template("{{ system.work.session.state }}|{{ system.work.session.canFinish }}"),
    ),
    (
        "Shared",
        Message::Template("{{ shared.team.name }} on {{ system.project.current.key }}"),
    ),
    ("Plain", Message::Text("plain")),
];

#[derive(Clone, Copy)]
enum Message {
    Text(&'static str),
    Template(&'static str),
    Reference(&'static str),
}

impl Message {
    fn value(self) -> ParameterValue {
        match self {
            Self::Text(text) => ParameterValue::Text(text.into()),
            Self::Template(text) => ParameterValue::Template(text.into()),
            Self::Reference(text) => ParameterValue::Reference(VariableRef::parse(text).unwrap()),
        }
    }
}

fn message(value: ParameterValue) -> ActionParameters {
    ActionParameters::new().with("message", value)
}

/// "Legacy": projects SHOP (default) and CMR; key 0 runs "Project
/// template", key 1 clicks "Plain" with a legacy template as override, key
/// 2 runs "Legacy flow" (a graph whose node overrides "Plain" the same
/// way, then "Session template").
fn profile() -> Profile {
    let mut profile = Profile::new("Legacy");
    for (key, name) in [("SHOP", "Shop"), ("CMR", "Cream")] {
        profile
            .projects
            .push(Project::new(ProjectKey::new(key).unwrap(), name));
    }
    profile.default_project = Some(profile.projects[0].id);
    let mut ids = BTreeMap::new();
    for (name, value) in ACTIONS {
        let instance = registry()
            .create_instance(name, general::log_id(), &message(value.value()))
            .unwrap();
        ids.insert(name, instance.id);
        profile.actions.push(instance);
    }
    let workflow = support::graph_workflow(
        "Legacy flow",
        support::graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "override", "type": "action", "action": ids["Plain"],
                 "overrides": {"message": {"type": "template",
                     "value": "node {{ system.project.current.key }} {{ shared.team.name }}"}}},
                {"id": "session", "type": "action", "action": ids["Session template"]},
            ]),
            &json!([
                {"from": "start", "to": "override"},
                {"from": "override", "to": "session"},
            ]),
        ),
    );
    let steps = Workflow::new("Legacy steps")
        .step(ids["Project template"], OnFailure::Stop)
        .step(ids["Shared"], OnFailure::Stop);
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.pages[0].bindings = vec![
        ControlBinding::new(
            key(0),
            Trigger::Press,
            BindingTarget::action(ids["Project template"]),
        ),
        ControlBinding::new(
            key(1),
            Trigger::Click,
            BindingTarget::Action {
                action: ids["Plain"],
                overrides: message(ParameterValue::Template(
                    "key {{ system.project.current.key }} {{ system.work.session.state }}".into(),
                )),
            },
        ),
        ControlBinding::new(
            key(2),
            Trigger::Press,
            BindingTarget::Workflow {
                workflow: workflow.id,
            },
        ),
    ];
    profile.workflows.push(workflow);
    profile.workflows.push(steps);
    profile.layouts.push(layout);
    profile
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

/// `team.name`: "crew" globally, "profile crew" for the profile, "shop
/// crew" for SHOP.
fn store_variables(store: &SqliteStore, profile: &Profile) {
    let name = VariableKey::new("team.name").unwrap();
    store
        .save_shared_variable(&VariableDefinition::new(
            name.clone(),
            VariableType::String,
            &[
                VariableScope::Global,
                VariableScope::Profile,
                VariableScope::Project,
            ],
        ))
        .unwrap();
    for (scope, value) in [
        (ScopeKey::Global, "crew"),
        (
            ScopeKey::Profile {
                profile: profile.id,
            },
            "profile crew",
        ),
        (
            ScopeKey::Project {
                profile: profile.id,
                project: profile.projects[0].id,
            },
            "shop crew",
        ),
    ] {
        store
            .set_shared_variable_value(&name, &scope, &VariableValue::String(value.into()))
            .unwrap();
    }
}

/// Records what each dispatched gesture did.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl GestureHooks for Recorder {
    fn bindings(
        &self,
        _device: &DeviceId,
        _page: Option<PageId>,
        _control: ControlId,
    ) -> Option<(BoundGestures, GestureTimings)> {
        None
    }

    fn fired(&self, _fired: &GestureFired) {}

    fn dispatched(&self, _fired: &GestureFired, outcome: &RuntimeOutcome) {
        if let RuntimeOutcome::Action { result, .. } = outcome {
            self.0
                .lock()
                .unwrap()
                .push(format!("{:?}: {}", result.status, result.message));
        }
    }
}

struct Data {
    paths: AgentPaths,
    key: DatabaseKey,
}

impl Data {
    /// A copy of `from` (database and vault) in `dir`.
    fn copy(from: &Path, dir: &TempDir) -> Self {
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        for file in ["deckpilot.db", "deckpilot.vault.json"] {
            std::fs::copy(from.join(file), paths.data_dir.join(file)).unwrap();
        }
        let key = Vault::new(&paths.database)
            .unlock(&Password::new(PASSWORD))
            .unwrap();
        Self { paths, key }
    }

    fn store(&self) -> SqliteStore {
        SqliteStore::open(&self.paths.database, &Encryption::Key(self.key.clone())).unwrap()
    }

    /// Every row of every table except run history and launches, read
    /// without DeckPilot's open, with the schema version. Values are
    /// compared as stored (text as its bytes).
    fn rows(&self) -> BTreeMap<String, Vec<String>> {
        let raw = rusqlite::Connection::open(&self.paths.database).unwrap();
        raw.pragma_update(None, "key", format!("x'{}'", self.key.to_hex().as_str()))
            .unwrap();
        let version: u32 = raw
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let mut tables: Vec<String> = raw
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .filter(|name: &String| {
                !matches!(
                    name.as_str(),
                    "workflow_executions" | "app_launches" | "sqlite_sequence"
                )
            })
            .collect();
        tables.sort();
        let mut all = BTreeMap::from([("user_version".to_owned(), vec![version.to_string()])]);
        for table in tables {
            // `apply_module_schemas` stamps `updated_at` on every start, even
            // when no migration runs (as before); the versions must stay.
            let columns = if table == "module_schemas" {
                "module_id, version"
            } else {
                "*"
            };
            let mut statement = raw
                .prepare(&format!("SELECT {columns} FROM \"{table}\" ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join(" | "))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            all.insert(table, rows);
        }
        all
    }
}

/// FNV-1a, enough to tell images apart.
fn digest(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

async fn wait_connected(agent: &TestAgent, device: &DeviceId) -> Value {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if let Some(info) = status
            .devices
            .iter()
            .find(|d| d.id == *device && d.state == ConnectionState::Connected && d.page.is_some())
        {
            return json!({
                "model": info.id.model.as_str(),
                "modelName": info.model_name,
                "layout": serde_json::to_value(&info.layout).unwrap(),
                "page": info.page,
                "activeProfile": status.active_profile.as_ref().map(|p| p.name.clone()),
                "activeProject": status.active_project.as_ref().map(|p| p.key.to_string()),
            });
        }
        assert!(Instant::now() < deadline, "{device} did not show a page");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The key images once they stop changing.
async fn stable_frames(mock: &MockBackend, device: &DeviceId) -> Value {
    let deadline = Instant::now() + TIMEOUT;
    let mut last = BTreeMap::new();
    let mut since = Instant::now();
    loop {
        let frames: BTreeMap<String, String> = mock
            .frames(device)
            .iter()
            .map(|(control, frame)| {
                (
                    format!("{control:?}"),
                    format!(
                        "{}x{} {}",
                        frame.width(),
                        frame.height(),
                        digest(frame.rgb())
                    ),
                )
            })
            .collect();
        if frames != last {
            last = frames;
            since = Instant::now();
        } else if !last.is_empty() && since.elapsed() > Duration::from_millis(500) {
            return serde_json::to_value(last).unwrap();
        }
        assert!(Instant::now() < deadline, "key images did not settle");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn run_actions(agent: &TestAgent, profile: &Profile) -> Value {
    let mut results = serde_json::Map::new();
    for (name, _) in ACTIONS {
        let action: ActionInstanceId = profile.actions.iter().find(|a| a.name == name).unwrap().id;
        let outcome = match agent.request(Request::RunAction { action }).await {
            Ok(Response::ActionResult { result }) => {
                format!("{:?}: {}", result.status, result.message)
            }
            Ok(other) => panic!("unexpected {other:?}"),
            Err(error) => format!("error: {error}"),
        };
        results.insert(name.to_owned(), outcome.into());
    }
    results.into()
}

async fn run_workflow(agent: &TestAgent, profile: &Profile, name: &str) -> Value {
    let workflow = profile
        .workflows
        .iter()
        .find(|w| w.name == name)
        .unwrap()
        .id;
    let started = agent.start_workflow(workflow).await.unwrap();
    let record = agent.wait_finished(started.id).await;
    json!({
        "state": format!("{:?}", record.summary.state),
        "nodes": record
            .nodes
            .iter()
            .map(|n| format!("{}: {:?}: {}", n.node, n.status, n.message))
            .collect::<Vec<_>>(),
    })
}

async fn click(
    agent: &TestAgent,
    hooks: &Recorder,
    device: &DeviceId,
    control: ControlId,
) -> Value {
    let before = hooks.0.lock().unwrap().len();
    for input in [ControlInput::Press, ControlInput::Release] {
        agent
            .request(Request::SimulateInput {
                device: device.clone(),
                control,
                input,
            })
            .await
            .unwrap();
    }
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(done) = hooks.0.lock().unwrap().get(before) {
            return done.clone().into();
        }
        assert!(Instant::now() < deadline, "the click did nothing");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn set_enabled(agent: &TestAgent, data: &Data, module: &str, enabled: bool) {
    data.store()
        .set_module_state(
            &ModuleId::new(module).unwrap(),
            &ModuleSettings {
                enabled,
                config: ActionParameters::new(),
            },
        )
        .unwrap();
    agent.request(Request::ReloadConfiguration).await.unwrap();
}

/// What the agent does with the data. Between the read-only part and the
/// part that changes the active project and module states, the agent is
/// stopped and every stored row is compared with before.
async fn observe(data: &Data) -> Value {
    let profile = data
        .store()
        .load_profile(data.store().active_profile().unwrap().unwrap())
        .unwrap()
        .unwrap();
    let before = data.rows();

    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), SERIAL);
    let hooks = Arc::new(Recorder::default());
    let agent =
        TestAgent::start_with_gestures(&data.paths, &data.key, &mock, None, Some(hooks.clone()));
    agent.unlock().await;
    let status = wait_connected(&agent, &device).await;
    let frames = stable_frames(&mock, &device).await;
    let actions = run_actions(&agent, &profile).await;
    let key_override = click(&agent, &hooks, &device, key(1)).await;
    let flow = run_workflow(&agent, &profile, "Legacy flow").await;
    let steps = run_workflow(&agent, &profile, "Legacy steps").await;
    agent.shutdown().await;
    let after = data.rows();
    for (table, rows) in &before {
        assert_eq!(Some(rows), after.get(table), "rows of {table} changed");
    }
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "tables were added or removed"
    );

    let agent =
        TestAgent::start_with_gestures(&data.paths, &data.key, &mock, None, Some(hooks.clone()));
    agent.unlock().await;
    wait_connected(&agent, &device).await;
    agent
        .request(Request::SelectProject {
            project: profile.projects[1].id,
        })
        .await
        .unwrap();
    let other_project = run_actions(&agent, &profile).await;
    set_enabled(&agent, data, "projects", false).await;
    set_enabled(&agent, data, "work-session", false).await;
    let modules_disabled = run_actions(&agent, &profile).await;
    let flow_modules_disabled = run_workflow(&agent, &profile, "Legacy flow").await;
    agent.shutdown().await;

    json!({
        "status": status,
        "frames": frames,
        "actions": actions,
        "keyOverride": key_override,
        "flow": flow,
        "steps": steps,
        "otherProject": other_project,
        "modulesDisabled": modules_disabled,
        "flowModulesDisabled": flow_modules_disabled,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_references_behave_as_before_and_no_row_is_rewritten() {
    let dir = TempDir::new("deckpilot-legacy-references");
    let data = Data::copy(Path::new(FIXTURE), &dir);
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(FIXTURE).join("expected.json")).unwrap(),
    )
    .unwrap();
    let observed = observe(&data).await;
    for (part, value) in expected.as_object().unwrap() {
        assert_eq!(
            &observed[part], value,
            "{part} differs from the agent that wrote the fixture"
        );
    }
}

/// Writes `fixtures/legacy-references` into `$DECKPILOT_LEGACY_FIXTURE`
/// with this agent: the database (opened once by the agent, as after a
/// normal start) and `expected.json`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "writes the fixture; run on the commit the fixture comes from"]
async fn write_the_fixture_with_this_agent() {
    let out = PathBuf::from(std::env::var_os("DECKPILOT_LEGACY_FIXTURE").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let dir = TempDir::new("deckpilot-legacy-fixture");
    let paths = AgentPaths::in_dir(dir.path().join("data"));
    paths.create_dirs().unwrap();
    let key = Vault::new(&paths.database)
        .with_test_cost()
        .initialize(&Password::new(PASSWORD))
        .unwrap();
    let profile = profile();
    {
        let store = SqliteStore::open(&paths.database, &Encryption::Key(key.clone())).unwrap();
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
        store_variables(&store, &profile);
    }
    // A first start, as after any normal use.
    let mock = MockBackend::new();
    let agent = TestAgent::start(&paths, &key, &mock, None);
    agent.unlock().await;
    agent.shutdown().await;
    for file in ["deckpilot.db", "deckpilot.vault.json"] {
        std::fs::copy(paths.data_dir.join(file), out.join(file)).unwrap();
    }

    let check = TempDir::new("deckpilot-legacy-fixture-check");
    let observed = observe(&Data::copy(&out, &check)).await;
    std::fs::write(
        out.join("expected.json"),
        serde_json::to_string_pretty(&observed).unwrap() + "\n",
    )
    .unwrap();
}
