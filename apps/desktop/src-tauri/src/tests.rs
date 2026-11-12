//! Exercises the commands through Tauri's mock IPC runtime, using the same
//! argument names and JSON shapes as the frontend (`src/lib/api.ts`).

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use deckpilot_agent::{Agent, AgentOptions, InstanceLock};
use deckpilot_device::{mock::MockBackend, models};
use deckpilot_ipc::AgentPaths;

use crate::state::AppState;
use deckpilot_keystore::MemoryKeyStore;
use deckpilot_presence::{Method, PresenceCheck, Scripted, Unavailable};
use std::sync::Arc;

/// Origin the bundled frontend is served from. Requests from any other origin
/// are treated as remote and rejected by Tauri's access control.
const APP_ORIGIN: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

/// Password of the test databases. Not used anywhere else.
const PASSWORD: &str = "test password 1";

struct Harness {
    window: WebviewWindow<tauri::test::MockRuntime>,
    paths: AgentPaths,
}

impl Harness {
    fn new() -> Self {
        Self::new_on(Self::temp_paths())
    }

    /// A password-protected configuration in `paths`.
    fn new_on(paths: AgentPaths) -> Self {
        let harness = Self::without_setup(paths);
        // First start: the user sets a password, which creates the
        // encrypted database.
        harness.ok(
            "setup_password",
            json!({ "input": { "password": PASSWORD, "confirmation": PASSWORD } }),
        );
        harness
    }

    fn temp_paths() -> AgentPaths {
        let data_dir = std::env::temp_dir().join(format!(
            "deckpilot-desktop-test-{}",
            deckpilot_core::ProfileId::new()
        ));
        AgentPaths::in_dir(data_dir)
    }

    /// The app started on `paths` without any user interaction. The
    /// computer offers no system authentication.
    fn without_setup(paths: AgentPaths) -> Self {
        Self::with_presence(paths, Arc::new(Unavailable))
    }

    /// As `without_setup`, with a scripted Touch ID (never the real one).
    fn with_presence(paths: AgentPaths, presence: Arc<dyn PresenceCheck>) -> Self {
        let app = mock_builder()
            .manage(AppState::with_presence(paths.clone(), presence).expect("open state"))
            .manage(crate::updates::UpdateManager::default())
            .manage(crate::commands::portability::PendingImport::default())
            .manage(crate::windows::WindowRegistry::default())
            .invoke_handler(crate::commands::handler())
            .build(mock_context(noop_assets()))
            .expect("build app");
        let window = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .expect("build window");
        Self { window, paths }
    }

    fn error(&self, cmd: &str, args: Value) -> String {
        self.call(cmd, args)
            .expect_err(cmd)
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Unlocks the app (and a running agent) as the unlock screen does.
    fn unlock(&self) {
        self.ok("unlock", json!({ "password": PASSWORD }));
    }

    fn call(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: APP_ORIGIN.parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: tauri::http::HeaderMap::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(&self.window, request).map(|body| body.deserialize().unwrap())
    }

    fn ok(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args)
            .unwrap_or_else(|error| panic!("{cmd} failed: {error}"))
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.paths.data_dir);
    }
}

// A single end-to-end scenario reads better than artificially split helpers.
#[allow(clippy::too_many_lines)]
#[test]
fn configuration_flow_through_ipc() {
    let h = Harness::new();

    let profiles = h.ok("list_profiles", json!({}));
    assert_eq!(profiles["profiles"], json!([]));
    assert_eq!(profiles["activeProfileId"], Value::Null);

    let profile = h.ok(
        "create_profile",
        json!({ "name": "Work", "description": "" }),
    );
    let profile_id = profile["id"].as_str().unwrap().to_owned();
    assert_eq!(
        h.ok("list_profiles", json!({}))["activeProfileId"],
        json!(profile_id)
    );

    let mut project_ids = Vec::new();
    for key in ["SHOP", "CMR", "CMS"] {
        let project = h.ok(
            "create_project",
            json!({
                "profileId": profile_id,
                "project": {
                    "key": key,
                    "name": "",
                    "resources": { "repository": format!("{}.repository", key.to_lowercase()) }
                }
            }),
        );
        assert_eq!(project["name"], key);
        project_ids.push(project["id"].as_str().unwrap().to_owned());
    }
    let duplicate = h.call(
        "create_project",
        json!({ "profileId": profile_id, "project": { "key": "shop", "name": "" } }),
    );
    assert!(duplicate.is_err());

    // SHOP -> CMR -> CMS -> SHOP
    for expected in [1, 2, 0] {
        let active = h.ok(
            "cycle_project",
            json!({ "profileId": profile_id, "direction": "next" }),
        );
        assert_eq!(active, json!(project_ids[expected]));
    }
    let active = h.ok(
        "cycle_project",
        json!({ "profileId": profile_id, "direction": "previous" }),
    );
    assert_eq!(active, json!(project_ids[2]));

    let machine = h.ok(
        "set_resource_binding",
        json!({ "key": "shop.repository", "kind": "directory", "value": "/src/shop" }),
    );
    assert_eq!(machine["bindings"][0]["value"], "/src/shop");

    let action = h.ok(
        "create_action",
        json!({
            "profileId": profile_id,
            "action": {
                "name": "Where am I",
                "actionType": "projects.repository",
                "parameters": { "project_id": null }
            }
        }),
    );
    let action_id = action["id"].as_str().unwrap().to_owned();

    // Without a running agent, runtime state is written to storage and
    // actions cannot run.
    h.ok(
        "select_project",
        json!({ "profileId": profile_id, "projectId": project_ids[1] }),
    );
    let refused = h.call(
        "run_action",
        json!({ "profileId": profile_id, "actionId": action_id }),
    );
    assert!(refused.unwrap_err().as_str().unwrap().contains("agent"));
    assert_eq!(h.ok("agent_status", json!({}))["running"], false);

    let invalid = h.call(
        "create_action",
        json!({
            "profileId": profile_id,
            "action": { "name": "Broken", "actionType": "general.log", "parameters": {} }
        }),
    );
    assert!(invalid.unwrap_err().as_str().unwrap().contains("message"));

    let overview = h.ok("get_overview", json!({}));
    assert_eq!(overview["activeProject"]["key"], "CMR");
    assert_eq!(overview["resourceBindingCount"], 1);

    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    assert_eq!(view["profile"]["actions"].as_array().unwrap().len(), 1);
    assert_eq!(view["issues"], json!([]));

    h.ok(
        "delete_action",
        json!({ "profileId": profile_id, "actionId": action_id }),
    );
    h.ok(
        "delete_project",
        json!({ "profileId": profile_id, "projectId": project_ids[1] }),
    );
    h.ok(
        "remove_resource_binding",
        json!({ "key": "shop.repository" }),
    );
    h.ok("delete_profile", json!({ "profileId": profile_id }));
    assert_eq!(h.ok("list_profiles", json!({}))["profiles"], json!([]));
}

#[test]
fn lists_static_catalogs() {
    let h = Harness::new();
    let types = h.ok("list_action_types", json!({}));
    assert!(
        types
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "general.log")
    );
    let models = h.ok("list_device_models", json!({}));
    assert_eq!(models[0]["id"], "elgato.streamdeck.mk2");
    assert_eq!(models[0]["support"], "unverified");
    assert_eq!(models.as_array().unwrap().len(), 3);
}

fn new_profile(h: &Harness, name: &str) -> String {
    let profile = h.ok("create_profile", json!({ "name": name, "description": "" }));
    profile["id"].as_str().unwrap().to_owned()
}

fn log_action(h: &Harness, profile_id: &str, name: &str, message: &str) -> Value {
    h.ok(
        "create_action",
        json!({
            "profileId": profile_id,
            "action": {
                "name": name,
                "actionType": "general.log",
                "parameters": { "message": { "type": "text", "value": message } },
                "description": "Says hello",
                "icon": "megaphone",
                "label": "{project.key}"
            }
        }),
    )
}

#[test]
fn actions_can_be_edited_duplicated_and_are_protected_while_used() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let action = log_action(&h, &profile_id, "Hello", "hi");
    assert_eq!(action["icon"], "megaphone");
    assert_eq!(action["label"], "{project.key}");

    // Edit: same id, new parameters; invalid edits are rejected.
    let mut edited = action.clone();
    edited["name"] = json!("Hello again");
    edited["parameters"]["message"]["value"] = json!("hi again");
    let saved = h.ok(
        "update_action",
        json!({ "profileId": profile_id, "action": edited }),
    );
    assert_eq!(saved["id"], action["id"]);
    let mut invalid = saved.clone();
    invalid["parameters"] = json!({});
    assert!(
        h.call(
            "update_action",
            json!({ "profileId": profile_id, "action": invalid })
        )
        .is_err()
    );
    let mut bad_icon = saved.clone();
    bad_icon["icon"] = json!("Not An Icon!");
    assert!(
        h.call(
            "update_action",
            json!({ "profileId": profile_id, "action": bad_icon })
        )
        .is_err()
    );

    // The same type instantiated twice with different parameters.
    let copy = h.ok(
        "duplicate_action",
        json!({ "profileId": profile_id, "actionId": action["id"], "name": "Hello — copy" }),
    );
    assert_ne!(copy["id"], action["id"]);

    // A workflow uses the copy, so it cannot be deleted.
    let workflow = h.ok(
        "save_workflow",
        json!({
            "profileId": profile_id,
            "workflow": {
                "id": "00000000-0000-4000-8000-000000000001",
                "name": "Start Work",
                "steps": [{ "action": copy["id"], "onFailure": "continue" }]
            }
        }),
    );
    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    let usages = &view["actionUsages"][copy["id"].as_str().unwrap()];
    assert_eq!(usages[0]["type"], "workflowStep");
    let refused = h.call(
        "delete_action",
        json!({ "profileId": profile_id, "actionId": copy["id"] }),
    );
    assert!(
        refused
            .unwrap_err()
            .as_str()
            .unwrap()
            .contains("still used")
    );

    h.ok(
        "delete_workflow",
        json!({ "profileId": profile_id, "workflowId": workflow["id"] }),
    );
    h.ok(
        "delete_action",
        json!({ "profileId": profile_id, "actionId": copy["id"] }),
    );
}

#[test]
fn workflows_are_validated_and_keep_their_step_order() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let first = log_action(&h, &profile_id, "First", "1");
    let second = log_action(&h, &profile_id, "Second", "2");
    let id = "00000000-0000-4000-8000-000000000002";
    let save = |steps: Value, name: &str| {
        h.call(
            "save_workflow",
            json!({ "profileId": profile_id, "workflow": { "id": id, "name": name, "steps": steps } }),
        )
    };

    // Unknown action, empty name and a bad override are all reported.
    let errors = save(
        json!([
            { "action": "00000000-0000-4000-8000-0000000000ff" },
            { "action": first["id"], "overrides": { "message": { "type": "integer", "value": 1 } } }
        ]),
        " ",
    )
    .unwrap_err();
    let message = errors.as_str().unwrap();
    assert!(
        message.contains("name") && message.contains("step 1") && message.contains("step 2"),
        "{message}"
    );

    // Reordering is saved as given.
    save(
        json!([{ "action": first["id"] }, { "action": second["id"] }]),
        "Start Work",
    )
    .unwrap();
    save(
        json!([
            { "action": second["id"], "condition": { "type": "always" } },
            { "action": first["id"], "condition": { "type": "previousSucceeded" }, "onFailure": "continue" }
        ]),
        "Start Work",
    )
    .unwrap();
    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    let steps = &view["profile"]["workflows"][0]["steps"];
    assert_eq!(view["profile"]["workflows"].as_array().unwrap().len(), 1);
    assert_eq!(steps[0]["action"], second["id"]);
    assert_eq!(steps[1]["condition"]["type"], "previousSucceeded");
}

#[test]
fn duplicated_profiles_are_isolated() {
    let h = Harness::new();
    let work = new_profile(&h, "Work");
    let action = log_action(&h, &work, "Hello", "work");
    let copy = h.ok(
        "duplicate_profile",
        json!({ "profileId": work, "name": "Home" }),
    );
    let home = copy["id"].as_str().unwrap().to_owned();

    let home_view = h.ok("get_profile", json!({ "profileId": home }));
    let mut home_action = home_view["profile"]["actions"][0].clone();
    assert_ne!(home_action["id"], action["id"]);
    home_action["parameters"]["message"]["value"] = json!("home");
    h.ok(
        "update_action",
        json!({ "profileId": home, "action": home_action }),
    );

    let work_view = h.ok("get_profile", json!({ "profileId": work }));
    assert_eq!(
        work_view["profile"]["actions"][0]["parameters"]["message"]["value"],
        "work"
    );
    h.ok(
        "update_profile_details",
        json!({ "profileId": home, "name": "Personal", "description": "x" }),
    );
    let profiles = h.ok("list_profiles", json!({}));
    assert!(
        profiles["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "Personal")
    );
}

#[test]
fn projects_reorder_and_resources_are_checked() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let mut ids = Vec::new();
    for key in ["SHOP", "CMR", "CMS"] {
        let project = h.ok(
            "create_project",
            json!({
                "profileId": profile_id,
                "project": { "key": key, "name": "", "resources": { "repository": format!("{}.repository", key.to_lowercase()) } }
            }),
        );
        ids.push(project["id"].as_str().unwrap().to_owned());
    }
    h.ok(
        "move_project",
        json!({ "profileId": profile_id, "projectId": ids[2], "index": 0 }),
    );
    h.ok(
        "set_default_project",
        json!({ "profileId": profile_id, "projectId": ids[1] }),
    );
    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    let keys: Vec<_> = view["profile"]["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].clone())
        .collect();
    assert_eq!(keys, [json!("CMS"), json!("SHOP"), json!("CMR")]);
    assert_eq!(view["profile"]["defaultProject"], json!(ids[1]));

    let updated = h.ok(
        "update_project",
        json!({ "profileId": profile_id, "projectId": ids[0], "project": { "key": "SHOP2", "name": "Shop", "resources": {} } }),
    );
    assert_eq!(updated["id"], json!(ids[0]));

    let existing = h.paths.data_dir.display().to_string();
    h.ok(
        "set_resource_binding",
        json!({ "key": "cmr.repository", "kind": "directory", "value": existing }),
    );
    h.ok("set_resource_binding", json!({ "key": "cms.repository", "kind": "directory", "value": "/definitely/missing/path" }));
    let checks = h.ok("check_resources", json!({ "profileId": profile_id }));
    let status = |key: &str| {
        checks
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["key"] == key)
            .map(|c| c["status"].clone())
    };
    assert_eq!(status("cmr.repository"), Some(json!("ok")));
    assert_eq!(status("cms.repository"), Some(json!("missing")));
    // shop.repository was removed from SHOP2, so nothing refers to it any more.
    assert_eq!(status("shop.repository"), None);
}

#[test]
fn layouts_are_designed_offline_and_validated() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let action = log_action(&h, &profile_id, "Hello", "hi");
    for model in [
        "elgato.streamdeck.mk2",
        "elgato.streamdeck.xl",
        "elgato.streamdeck.plus",
    ] {
        let mut layout = h.ok(
            "create_layout",
            json!({ "profileId": profile_id, "modelId": model, "name": "Main", "generate": false }),
        );
        let page = layout["homePage"].clone();
        layout["pages"][0]["bindings"] = json!([{
            "control": { "kind": "key", "index": 0 },
            "trigger": { "type": "press" },
            "target": { "type": "action", "action": action["id"] },
            "label": "{project.key}"
        }]);
        h.ok(
            "save_layout",
            json!({ "profileId": profile_id, "layout": layout }),
        );
        let preview = h.ok(
            "preview_layout",
            json!({ "profileId": profile_id, "layout": layout, "pageId": page, "projectId": null }),
        );
        let first = &preview[0];
        assert_eq!(first["control"]["index"], 0);
        assert_eq!(first["face"]["style"], "action");
        assert_eq!(first["face"]["title"], "-", "no project is active");
    }

    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    let mut mk2 = view["profile"]["layouts"][0].clone();
    // Key 15 does not exist on an MK.2, and keys cannot rotate.
    mk2["pages"][0]["bindings"] = json!([
        { "control": { "kind": "key", "index": 15 }, "trigger": { "type": "press" }, "target": { "type": "back" } },
        { "control": { "kind": "key", "index": 1 }, "trigger": { "type": "rotateClockwise" }, "target": { "type": "back" } }
    ]);
    let error = h
        .call(
            "save_layout",
            json!({ "profileId": profile_id, "layout": mk2 }),
        )
        .unwrap_err();
    let message = error.as_str().unwrap();
    assert!(
        message.contains("key 15") && message.contains("RotateClockwise"),
        "{message}"
    );

    // The rejected edit did not change the stored layout.
    let stored = h.ok("get_profile", json!({ "profileId": profile_id }));
    assert_eq!(
        stored["profile"]["layouts"][0]["pages"][0]["bindings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let copy = h.ok(
        "duplicate_layout",
        json!({ "profileId": profile_id, "layoutId": mk2["id"], "name": "Copy" }),
    );
    h.ok(
        "activate_layout",
        json!({ "profileId": profile_id, "layoutId": copy }),
    );
    h.ok(
        "delete_layout",
        json!({ "profileId": profile_id, "layoutId": copy }),
    );
    let after = h.ok("get_profile", json!({ "profileId": profile_id }));
    assert_eq!(after["profile"]["layouts"].as_array().unwrap().len(), 3);
}

#[test]
fn updates_report_their_configuration_and_store_preferences() {
    let h = Harness::new();
    let state = h.ok("update_state", json!({}));
    assert_eq!(state["currentVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(state["preferences"]["mode"], "notifyOnly");
    assert_eq!(state["phase"], "idle");
    if option_env!("DECKPILOT_UPDATER_PUBKEY").is_none() {
        assert_eq!(state["configured"], false);
        let refused = h.call("check_for_updates", json!({})).unwrap_err();
        assert!(refused.as_str().unwrap().contains("not configured"));
    }
    let saved = h.ok(
        "set_update_preferences",
        json!({ "preferences": { "mode": "downloadAndAsk", "automaticChecks": true, "checkIntervalHours": 0, "lastCheck": null } }),
    );
    assert_eq!(saved["preferences"]["mode"], "downloadAndAsk");
    assert_eq!(
        saved["preferences"]["checkIntervalHours"], 1,
        "intervals are clamped"
    );
    assert!(
        h.call("download_update", json!({})).is_err(),
        "nothing to download"
    );

    let launch = h.ok("launch_info", json!({}));
    assert_eq!(launch["launch"]["firstAfterUpdate"], false);
    assert_eq!(launch["encryption"]["enabled"], true);
    assert_eq!(launch["encryption"]["passwordRequired"], true);
}

/// Starts a real agent with a simulated MK.2 on its own runtime, without a
/// credential store.
fn start_agent(paths: &AgentPaths) -> (tokio::runtime::Runtime, MockBackend) {
    start_agent_with(paths, None)
}

/// As `start_agent`, with an in-memory credential store standing in for
/// the Keychain (no real credentials are touched).
fn start_agent_with(
    paths: &AgentPaths,
    keys: Option<MemoryKeyStore>,
) -> (tokio::runtime::Runtime, MockBackend) {
    let (runtime, mock, _) = launch_agent(paths, keys);
    (runtime, mock)
}

/// As `start_agent_with`, also giving in-process access to the agent (e.g.
/// to press a work session key).
fn launch_agent(
    paths: &AgentPaths,
    keys: Option<MemoryKeyStore>,
) -> (
    tokio::runtime::Runtime,
    MockBackend,
    deckpilot_agent::AgentHandle,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mock = MockBackend::new();
    mock.plug(&models::stream_deck_mk2(), "TEST-0001");
    let options = AgentOptions {
        paths: paths.clone(),
        backend: Box::new(mock.clone()),
        mock: Some(mock.clone()),
        rescan_interval: std::time::Duration::from_millis(50),
        autostart: None,
        open_desktop: None,
        open_window: None,
        key_store: keys.map(|keys| -> Arc<dyn deckpilot_keystore::KeyStore> { Arc::new(keys) }),
        auto_lock_check: std::time::Duration::from_millis(50),
        cancel_grace: std::time::Duration::from_secs(10),
        clock: deckpilot_agent::system_clock(),
        screen_lock: None,
        notifier: None,
    };
    paths.create_dirs().unwrap();
    let lock = InstanceLock::acquire(&paths.lock_file).unwrap();
    let handle = runtime.block_on(async {
        let agent = Agent::start(options, lock).unwrap();
        let handle = agent.handle();
        tokio::spawn(agent.run(std::future::pending()));
        handle
    });
    (runtime, mock, handle)
}

fn wait_for(h: &Harness, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let status = h.ok("agent_status", json!({}));
        if predicate(&status) {
            return status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "agent state not reached: {status}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn runtime_state_and_actions_go_through_the_agent() {
    let h = Harness::new();
    let profile = h.ok(
        "create_profile",
        json!({ "name": "Work", "description": "" }),
    );
    let profile_id = profile["id"].as_str().unwrap().to_owned();
    for key in ["SHOP", "CMR"] {
        h.ok(
            "create_project",
            json!({ "profileId": profile_id, "project": { "key": key, "name": "" } }),
        );
    }
    let action = h.ok(
        "create_action",
        json!({
            "profileId": profile_id,
            "action": {
                "name": "Announce",
                "actionType": "general.log",
                "parameters": { "message": { "type": "text", "value": "on {project.key}" } }
            }
        }),
    );

    let (_runtime, mock) = start_agent(&h.paths);
    h.unlock();
    let status = wait_for(&h, |s| {
        s["running"] == true && s["status"]["devices"][0]["state"] == "connected"
    });
    assert_eq!(status["status"]["backend"], "mock");
    assert_eq!(status["status"]["activeProfile"]["name"], "Work");
    assert_eq!(status["status"]["devices"][0]["layout"]["type"], "missing");

    // Generating a layout is picked up by the agent without a restart.
    h.ok(
        "create_layout",
        json!({ "profileId": profile_id, "modelId": "elgato.streamdeck.mk2", "name": "", "generate": true }),
    );
    wait_for(&h, |s| {
        s["status"]["devices"][0]["layout"]["type"] == "compatible"
    });

    let next = h.ok(
        "cycle_project",
        json!({ "profileId": profile_id, "direction": "next" }),
    );
    let result = h.ok(
        "run_action",
        json!({ "profileId": profile_id, "actionId": action["id"] }),
    );
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["message"], "on CMR");
    // The agent persisted the selection; the overview reads it from storage.
    let overview = h.ok("get_overview", json!({}));
    assert_eq!(overview["activeProject"]["id"], next);

    h.ok("set_brightness", json!({ "percent": 40 }));
    let device = deckpilot_device::DeviceId {
        model: deckpilot_device::DeviceModelId::new(models::STREAM_DECK_MK2_ID),
        serial: "TEST-0001".into(),
    };
    wait_for(&h, |_| mock.brightness(&device) == Some(40));
    assert!(h.call("set_brightness", json!({ "percent": 200 })).is_err());

    h.ok("stop_agent", json!({}));
    wait_for(&h, |s| s["running"] == false);
}

#[test]
fn fresh_installation_has_an_empty_action_library() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    assert_eq!(view["profile"]["actions"], json!([]));
    assert_eq!(view["profile"]["workflows"], json!([]));
    assert_eq!(view["profile"]["projects"], json!([]));
    assert_eq!(view["profile"]["layouts"], json!([]));
    // Modules provide types, not instances.
    let modules = h.ok("list_modules", json!({}));
    let ids: Vec<_> = modules
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        ids,
        [
            "docker",
            "general",
            "git",
            "github",
            "gitlab",
            "http",
            "jira",
            "notifications",
            "projects",
            "system",
            "work-session",
            "workspace"
        ]
    );
    let module = |id: &str| {
        modules
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(module("projects")["pages"][0]["id"], "projects");
    // Modules with risky actions start with them switched off.
    assert_eq!(
        module("system")["configValues"]["allowCommands"]["value"],
        false
    );
    let types: Vec<_> = module("projects")["actionTypes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        types,
        [
            "projects.cycle",
            "projects.select",
            "projects.current",
            "projects.repository",
            "projects.documentation",
            "projects.resolve-resource",
            "projects.next",
            "projects.previous",
            "projects.active",
        ]
    );
    // Replaced types are marked, so the app does not offer them for new
    // actions; the flag is left out for every other type.
    let deprecated: Vec<_> = module("projects")["actionTypes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["deprecated"] == true)
        .map(|t| t["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        deprecated,
        ["projects.next", "projects.previous", "projects.active"]
    );
    assert!(
        module("projects")["actionTypes"][0]
            .get("deprecated")
            .is_none()
    );
    assert_eq!(h.ok("migration_notes", json!({})), json!([]));
}

#[test]
fn disabled_modules_hide_types_but_keep_saved_actions() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let next = h.ok(
        "create_action",
        json!({
            "profileId": profile_id,
            "action": { "name": "Next", "actionType": "projects.next", "parameters": {} }
        }),
    );

    let modules = h.ok(
        "set_module_enabled",
        json!({ "moduleId": "projects", "enabled": false }),
    );
    let projects = modules
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "projects")
        .unwrap();
    assert_eq!(projects["enabled"], false);
    let types = h.ok("list_action_types", json!({}));
    assert!(
        types
            .as_array()
            .unwrap()
            .iter()
            .all(|t| !t["id"].as_str().unwrap().starts_with("projects."))
    );
    // New actions of the module and project changes are refused ...
    assert!(
        h.call(
            "create_action",
            json!({
                "profileId": profile_id,
                "action": { "name": "Again", "actionType": "projects.next", "parameters": {} }
            }),
        )
        .unwrap_err()
        .as_str()
        .unwrap()
        .contains("disabled")
    );
    assert!(
        h.call(
            "create_project",
            json!({ "profileId": profile_id, "project": { "key": "SHOP", "name": "" } }),
        )
        .is_err()
    );
    // ... but the saved action is still there and the state survives restarts.
    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    assert_eq!(view["profile"]["actions"][0]["id"], next["id"]);
    let reopened = AppState::open(h.paths.clone()).unwrap();
    let key = h.window.state::<AppState>().key().unwrap();
    reopened.attach(key, false).unwrap();
    assert!(
        !reopened
            .modules()
            .is_enabled(&deckpilot_modules::projects::module_id())
    );

    h.ok(
        "set_module_enabled",
        json!({ "moduleId": "projects", "enabled": true }),
    );
    let invalid = h.call(
        "set_module_config",
        json!({ "moduleId": "projects", "config": { "wrapAround": { "type": "text", "value": "x" } } }),
    );
    assert!(invalid.is_err());
    let configured = h.ok(
        "set_module_config",
        json!({ "moduleId": "projects", "config": { "wrapAround": { "type": "boolean", "value": false } } }),
    );
    let projects = configured
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "projects")
        .unwrap();
    assert_eq!(projects["configValues"]["wrapAround"]["value"], false);
}

#[test]
fn projects_stay_in_their_profile() {
    let h = Harness::new();
    let work = new_profile(&h, "Work");
    let home = new_profile(&h, "Home");
    let shop = h.ok(
        "create_project",
        json!({ "profileId": work, "project": { "key": "SHOP", "name": "Shop", "resources": { "repository": "shop.repository" } } }),
    );
    let shop_id = shop["id"].as_str().unwrap();

    // Another profile cannot edit, delete, select or reference it.
    let edit = h.call(
        "update_project",
        json!({ "profileId": home, "projectId": shop_id, "project": { "key": "SHOP", "name": "Mine" } }),
    );
    assert!(edit.is_err());
    assert!(
        h.call(
            "delete_project",
            json!({ "profileId": home, "projectId": shop_id })
        )
        .is_err()
    );
    assert!(
        h.call(
            "set_default_project",
            json!({ "profileId": home, "projectId": shop_id })
        )
        .is_err()
    );
    let select = h.call(
        "create_action",
        json!({
            "profileId": home,
            "action": {
                "name": "Shop",
                "actionType": "projects.select",
                "parameters": { "project_id": { "type": "project", "value": shop_id } }
            }
        }),
    );
    assert!(
        select
            .unwrap_err()
            .as_str()
            .unwrap()
            .contains("does not belong to this profile")
    );

    // Copying is explicit and creates an independent project.
    let copy = h.ok(
        "copy_project",
        json!({ "profileId": work, "projectId": shop_id, "targetProfileId": home }),
    );
    assert_ne!(copy["id"], shop["id"]);
    assert_eq!(copy["resources"], shop["resources"]);
    assert!(
        h.call(
            "copy_project",
            json!({ "profileId": work, "projectId": shop_id, "targetProfileId": home }),
        )
        .is_err(),
        "the key is taken now"
    );
    h.ok(
        "update_project",
        json!({ "profileId": home, "projectId": copy["id"], "project": { "key": "SHOP", "name": "Home shop" } }),
    );
    let work_view = h.ok("get_profile", json!({ "profileId": work }));
    assert_eq!(work_view["profile"]["projects"][0]["name"], "Shop");
    let home_view = h.ok("get_profile", json!({ "profileId": home }));
    assert_eq!(
        home_view["profile"]["projects"].as_array().unwrap().len(),
        1
    );
}

/// The real commands against a running agent: generic project actions with
/// structured results, explicit project copies and module enable/disable.
#[allow(clippy::too_many_lines)]
#[test]
fn modules_and_project_copies_work_with_the_running_agent() {
    let h = Harness::new();
    let work = new_profile(&h, "Work");
    let home = new_profile(&h, "Home");
    let mut projects = Vec::new();
    for key in ["SHOP", "CMR"] {
        let project = h.ok(
            "create_project",
            json!({
                "profileId": work,
                "project": { "key": key, "name": "", "resources": { "repository": format!("{}.repository", key.to_lowercase()) } }
            }),
        );
        projects.push(project["id"].as_str().unwrap().to_owned());
    }
    h.ok(
        "set_resource_binding",
        json!({ "key": "shop.repository", "kind": "directory", "value": "/src/shop" }),
    );
    let action = |name: &str, action_type: &str, parameters: Value| {
        h.ok(
            "create_action",
            json!({
                "profileId": work,
                "action": { "name": name, "actionType": action_type, "parameters": parameters }
            }),
        )["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let repository = action("Repository", "projects.repository", json!({}));
    let select_cmr = action(
        "CMR",
        "projects.select",
        json!({ "project_id": { "type": "project", "value": projects[1] } }),
    );
    let current = action("Current", "projects.select", json!({ "project_id": null }));

    let (_runtime, _mock) = start_agent(&h.paths);
    h.unlock();
    wait_for(&h, |s| s["running"] == true);
    let run = |id: &str| h.ok("run_action", json!({ "profileId": work, "actionId": id }));

    // Generic types with structured results; the current project is the default.
    let resolved = run(&repository);
    assert_eq!(resolved["status"], "succeeded", "{resolved}");
    assert_eq!(resolved["data"]["value"], "/src/shop");
    assert_eq!(resolved["data"]["projectKey"], "SHOP");
    assert_eq!(resolved["data"]["kind"], "directory");
    let reported = run(&current);
    assert_eq!(reported["data"]["projectKey"], "SHOP");
    assert!(reported.get("effects").is_none(), "{reported}");
    run(&select_cmr);
    wait_for(&h, |s| s["status"]["activeProject"]["key"] == "CMR");
    // CMR's repository is not bound on this machine.
    let missing = run(&repository);
    assert_eq!(missing["status"], "failed");
    assert!(missing["message"].as_str().unwrap().contains("not bound"));

    // An explicit copy is an independent project of the other profile.
    let copy = h.ok(
        "copy_project",
        json!({ "profileId": work, "projectId": projects[0], "targetProfileId": home }),
    );
    h.ok("set_active_profile", json!({ "profileId": home }));
    let status = wait_for(&h, |s| s["status"]["activeProfile"]["name"] == "Home");
    assert_eq!(status["status"]["activeProject"]["id"], copy["id"]);
    // Projects of the inactive profile cannot be selected.
    assert!(
        h.call(
            "select_project",
            json!({ "profileId": home, "projectId": projects[1] }),
        )
        .is_err()
    );
    h.ok("set_active_profile", json!({ "profileId": work }));
    wait_for(&h, |s| s["status"]["activeProject"]["key"] == "CMR");

    // Disabling the module stops its actions and project switching in the
    // agent, through every path, without touching the configuration.
    h.ok(
        "set_module_enabled",
        json!({ "moduleId": "projects", "enabled": false }),
    );
    let refused = run(&repository);
    assert_eq!(refused["status"], "failed");
    assert!(
        refused["message"].as_str().unwrap().contains("disabled"),
        "{refused}"
    );
    assert!(
        h.call(
            "cycle_project",
            json!({ "profileId": work, "direction": "next" })
        )
        .is_err()
    );
    assert!(
        h.call(
            "select_project",
            json!({ "profileId": work, "projectId": projects[0] })
        )
        .is_err()
    );
    assert!(
        h.call("check_resources", json!({ "profileId": work }))
            .is_err()
    );
    let view = h.ok("get_profile", json!({ "profileId": work }));
    assert_eq!(view["profile"]["actions"].as_array().unwrap().len(), 3);
    wait_for(&h, |s| s["status"]["activeProject"]["key"] == "CMR");

    // Enabled again with wrap-around off: the agent's switching follows the
    // module setting, just like the actions.
    h.ok(
        "set_module_enabled",
        json!({ "moduleId": "projects", "enabled": true }),
    );
    h.ok(
        "set_module_config",
        json!({ "moduleId": "projects", "config": { "wrapAround": { "type": "boolean", "value": false } } }),
    );
    let at_end = h.call(
        "cycle_project",
        json!({ "profileId": work, "direction": "next" }),
    );
    assert!(
        at_end
            .unwrap_err()
            .as_str()
            .unwrap()
            .contains("last project"),
        "the agent applies wrapAround"
    );
    assert_eq!(run(&select_cmr)["status"], "succeeded");

    h.ok("stop_agent", json!({}));
    wait_for(&h, |s| s["running"] == false);
}

#[test]
fn the_configuration_is_locked_without_the_password() {
    let h = Harness::new();
    let status = h.ok("security_status", json!({}));
    assert_eq!(status["database"], "encrypted");
    assert_eq!(status["unlocked"], true);
    let profile = new_profile(&h, "Work");

    h.ok("lock", json!({}));
    assert_eq!(h.ok("security_status", json!({}))["unlocked"], false);
    assert_eq!(
        h.error("list_profiles", json!({})),
        "the configuration is locked"
    );
    assert_eq!(
        h.error("unlock", json!({ "password": "not the password" })),
        "the password is not correct"
    );
    h.unlock();
    assert_eq!(
        h.ok("list_profiles", json!({}))["profiles"][0]["id"],
        json!(profile)
    );
    // The database file holds no readable configuration.
    let bytes = std::fs::read(&h.paths.database).unwrap();
    assert!(!bytes.windows(15).any(|w| w == b"SQLite format 3"));
    // Short passwords and mismatched confirmations are refused.
    let other = Harness::without_setup(Harness::temp_paths());
    assert!(
        other
            .error(
                "setup_password",
                json!({ "input": { "password": "short", "confirmation": "short" } })
            )
            .contains("at least 8")
    );
    assert!(
        other
            .error(
                "setup_password",
                json!({ "input": { "password": "long enough", "confirmation": "long enougH" } })
            )
            .contains("do not match")
    );
}

#[test]
fn changing_the_password_keeps_the_agent_and_data() {
    let h = Harness::new();
    let profile = new_profile(&h, "Work");
    let (_runtime, _mock) = start_agent(&h.paths);
    h.unlock();
    wait_for(&h, |s| s["status"]["security"] == "unlocked");

    assert_eq!(
        h.error(
            "change_password",
            json!({ "input": { "current": "wrong", "password": "brand new pw", "confirmation": "brand new pw" } }),
        ),
        "the password is not correct"
    );
    h.ok(
        "change_password",
        json!({ "input": { "current": PASSWORD, "password": "brand new pw", "confirmation": "brand new pw" } }),
    );
    // The agent was locked during the swap and is unlocked again.
    let status = wait_for(&h, |s| s["status"]["security"] == "unlocked");
    assert_eq!(status["status"]["activeProfile"]["id"], json!(profile));
    h.ok("lock", json!({}));
    wait_for(&h, |s| s["status"]["security"] == "locked");
    assert!(h.call("unlock", json!({ "password": PASSWORD })).is_err());
    h.ok("unlock", json!({ "password": "brand new pw" }));
    wait_for(&h, |s| s["status"]["security"] == "unlocked");
    assert_eq!(
        h.ok("list_profiles", json!({}))["profiles"][0]["id"],
        json!(profile)
    );
    h.ok("stop_agent", json!({}));
    wait_for(&h, |s| s["running"] == false);
}

#[test]
fn a_restarted_app_reuses_the_unlocked_agent_without_asking() {
    let h = Harness::new();
    new_profile(&h, "Work");
    let (_runtime, _mock) = start_agent(&h.paths);
    h.unlock();
    wait_for(&h, |s| s["status"]["security"] == "unlocked");

    // The window is closed and opened again: a new app process.
    let restarted = Harness::without_setup(h.paths.clone());
    let status = restarted.ok("security_status", json!({}));
    assert_eq!(status["unlocked"], true, "{status}");
    assert_eq!(
        restarted.ok("list_profiles", json!({}))["profiles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    h.ok("stop_agent", json!({}));
    wait_for(&h, |s| s["running"] == false);

    // With the agent stopped, a new app process must ask.
    let cold = Harness::without_setup(h.paths.clone());
    assert_eq!(cold.ok("security_status", json!({}))["unlocked"], false);
}

#[test]
fn the_password_is_confirmed_once_after_an_update() {
    let h = Harness::new();
    new_profile(&h, "Work");
    // The previous version confirmed the password; this is the first start
    // of the new version, unlocked with the key of the running agent.
    {
        use deckpilot_storage::ConfigStore as _;
        let state = h.window.state::<AppState>();
        let store = state.store().unwrap();
        store
            .set_setting("security.confirmedVersion", "0.0.1")
            .unwrap();
        // The default asks only for sensitive operations.
        store
            .set_setting("security.reauthentication", "afterUpdate")
            .unwrap();
    }
    let state = h.window.state::<AppState>();
    let key = state.key().unwrap();
    state.detach();
    state.attach(key.clone(), false).unwrap();

    let status = h.ok("security_status", json!({}));
    assert_eq!(status["confirmationRequired"], true);
    assert_eq!(status["unlocked"], false);
    assert!(h.error("list_profiles", json!({})).contains("confirm"));
    assert!(
        h.call("confirm_identity", json!({ "password": "wrong" }))
            .is_err()
    );
    h.ok("confirm_identity", json!({ "password": PASSWORD }));
    assert_eq!(
        h.ok("list_profiles", json!({}))["profiles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Later starts of the same version do not ask again.
    state.detach();
    state.attach(key, false).unwrap();
    assert_eq!(
        h.ok("security_status", json!({}))["confirmationRequired"],
        false
    );
}

#[test]
fn legacy_databases_are_encrypted_on_request() {
    let paths = Harness::temp_paths();
    paths.create_dirs().unwrap();
    let profile = {
        use deckpilot_storage::{ConfigStore, Encryption, SqliteStore};
        let store = SqliteStore::open(&paths.database, &Encryption::None).unwrap();
        let profile = deckpilot_core::Profile::new("From 0.3");
        store.save_profile(&profile).unwrap();
        profile
    };
    let h = Harness::without_setup(paths);
    let status = h.ok("security_status", json!({}));
    assert_eq!(status["database"], "plaintext");
    assert_eq!(
        h.error("list_profiles", json!({})),
        "the configuration is locked"
    );

    let status = h.ok(
        "encrypt_legacy_database",
        json!({ "input": { "password": PASSWORD, "confirmation": PASSWORD } }),
    );
    assert_eq!(status["database"], "encrypted");
    assert_eq!(status["unlocked"], true);
    assert_eq!(status["legacyCopies"].as_array().unwrap().len(), 1);
    assert_eq!(
        h.ok("list_profiles", json!({}))["profiles"][0]["id"],
        json!(profile.id)
    );
    // Sensitive: needs a fresh authentication.
    assert_eq!(
        h.error("remove_unencrypted_copies", json!({})),
        "authentication required"
    );
    h.ok("authenticate", json!({ "password": PASSWORD }));
    let status = h.ok("remove_unencrypted_copies", json!({}));
    assert_eq!(status["legacyCopies"], json!([]));
}

/// The export file reveals nothing of its content: neither the plaintext
/// header nor the (base64-decoded) ciphertext contains the serialized
/// profile, and machine values are not even in the decrypted payload.
///
/// The needles are long, structured strings taken from the plaintext, so
/// a random match is practically impossible (searching for a short word
/// such as "Work" in base64 text matches by chance now and then).
fn assert_export_hides_its_content(bytes: &[u8], password: &str) {
    use base64::Engine as _;
    let contains = |haystack: &[u8], needle: &str| {
        haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
    };

    let envelope: Value = serde_json::from_slice(bytes).unwrap();
    let mut fields: Vec<&str> = envelope
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(fields, ["ciphertext", "header"]);
    let header = envelope["header"].as_str().unwrap();
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(envelope["ciphertext"].as_str().unwrap())
        .unwrap();

    let payload = deckpilot_portability::payload::open(
        bytes,
        &deckpilot_storage::crypto::Password::new(password),
    )
    .unwrap();
    let plaintext = serde_json::to_string(&payload).unwrap();
    let hidden = [r#""name":"Work""#, r#""key":"SHOP""#, "shop.repository"];
    for needle in hidden {
        // Meaningful: the payload holds exactly this text.
        assert!(plaintext.contains(needle), "{needle} is not in the payload");
        assert!(!header.contains(needle), "the header shows {needle}");
        assert!(
            !contains(&ciphertext, needle),
            "the ciphertext shows {needle}"
        );
        assert!(!contains(bytes, needle), "the file shows {needle}");
    }
    // Machine values are never exported at all.
    let machine = "/private/machine/path";
    assert!(!plaintext.contains(machine));
    assert!(!contains(bytes, machine));
}

#[test]
fn configuration_round_trips_through_an_encrypted_export() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    h.ok(
        "create_project",
        json!({ "profileId": profile_id, "project": { "key": "SHOP", "name": "", "resources": { "repository": "shop.repository" } } }),
    );
    h.ok(
        "set_resource_binding",
        json!({ "key": "shop.repository", "kind": "directory", "value": "/private/machine/path" }),
    );
    let file = h.paths.data_dir.join("work.deckpilot");
    let path = file.display().to_string();
    h.ok("authenticate", json!({ "password": PASSWORD }));
    let result = h.ok(
        "export_configuration",
        json!({ "request": { "path": path, "password": "export secret", "confirmation": "export secret", "profiles": null, "includeModuleSettings": true } }),
    );
    assert_eq!(result["profiles"], 1);
    let bytes = std::fs::read(&file).unwrap();
    assert_export_hides_its_content(&bytes, "export secret");

    assert!(
        h.error(
            "preview_import",
            json!({ "path": path, "password": "wrong" })
        )
        .contains("password is wrong")
    );
    // A modified file is rejected the same way.
    let mut tampered = bytes.clone();
    let middle = tampered.len() - 10;
    tampered[middle] = if tampered[middle] == b'A' { b'B' } else { b'A' };
    let tampered_path = h.paths.data_dir.join("tampered.deckpilot");
    std::fs::write(&tampered_path, &tampered).unwrap();
    assert!(
        h.call(
            "preview_import",
            json!({ "path": tampered_path.display().to_string(), "password": "export secret" })
        )
        .is_err()
    );

    // Importing into the same installation: a conflict, kept as a copy.
    let preview = h.ok(
        "preview_import",
        json!({ "path": path, "password": "export secret" }),
    );
    assert_eq!(
        preview["preview"]["profiles"][0]["conflict"]["type"],
        "sameId"
    );
    assert_eq!(preview["preview"]["missingResources"], json!([]));
    let summary = h.ok(
        "apply_import",
        json!({ "token": preview["token"], "options": { "profiles": [profile_id], "conflicts": { profile_id.clone(): "keepBoth" } } }),
    );
    assert_eq!(summary["renamed"][0], "Work (imported)");
    let profiles = h.ok("list_profiles", json!({}));
    assert_eq!(profiles["profiles"].as_array().unwrap().len(), 2);
    // The token is used up.
    assert!(
        h.call(
            "apply_import",
            json!({ "token": preview["token"], "options": { "profiles": [profile_id] } })
        )
        .is_err()
    );
}

#[test]
fn native_windows_are_opened_by_identifier_and_reused() {
    use crate::windows::{self, WindowRegistry};
    let h = Harness::new();
    let app = h.window.app_handle().clone();
    let count = || app.webview_windows().len();
    let before = count();

    h.ok("open_window", json!({ "id": "security.unlock" }));
    let label = windows::label_for("security.unlock");
    let prompt = app.get_webview_window(&label).expect("window created");
    assert_eq!(count(), before + 1);
    // A second request focuses the same window instead of opening another.
    h.ok("open_window", json!({ "id": "security.unlock" }));
    assert_eq!(count(), before + 1);

    // Unknown windows and windows that need the configuration while it is
    // locked are refused; nothing but an identifier can be requested.
    assert!(
        h.error("open_window", json!({ "id": "logs.viewer" }))
            .contains("no window")
    );
    assert!(
        h.error("open_window", json!({ "id": "../index.html" }))
            .contains("no window")
    );
    h.ok("lock", json!({}));
    assert!(
        h.error(
            "open_window",
            json!({ "id": "workflows.execution-summary" })
        )
        .contains("locked")
    );
    h.unlock();
    h.ok(
        "open_window",
        json!({ "id": "workflows.execution-summary" }),
    );

    // Sizing: within the compact limits, shown after the first fit.
    let size = windows::fit(&prompt, (390.0, 280.0), (390.0, 280.0)).unwrap();
    assert_eq!(size, (390.0, 280.0));
    let size = windows::fit(&prompt, (100.0, 2000.0), (390.0, 280.0)).unwrap();
    assert_eq!(size.0, 360.0);
    assert!(size.1 <= 760.0, "clamped: {size:?}");
    // The main window is not a sized prompt.
    assert!(
        h.error(
            "fit_window",
            json!({ "width": 300.0, "height": 200.0, "viewportWidth": 300.0, "viewportHeight": 200.0 })
        )
            .contains("no window")
    );
    assert!(
        h.call(
            "fit_window",
            json!({ "width": -1.0, "height": 200.0, "viewportWidth": 300.0, "viewportHeight": 200.0 })
        )
            .is_err()
    );
    assert_eq!(
        h.ok("current_window", json!({})),
        Value::Null,
        "the main window has no window spec"
    );

    // Closing removes the window from the registry (cleanup).
    let registry = app.state::<WindowRegistry>();
    assert_eq!(registry.open_labels().len(), 2);
    windows::closed(&app, &label, true);
    assert!(registry.spec(&label).is_none());
    assert_eq!(registry.open_labels().len(), 1);
}

// System-managed keys and sensitive operations. The agent's credential store
// is in memory and Touch ID is scripted; nothing real is touched.

fn status(h: &Harness) -> Value {
    h.ok("security_status", json!({}))
}

/// Waits until the agent opened the configuration with the system key and
/// the app attached to it.
fn wait_unlocked(h: &Harness) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let view = status(h);
        if view["unlocked"] == true {
            return view;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "not unlocked: {view}, agent: {}",
            h.ok("agent_status", json!({}))
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn agent_security(h: &Harness) -> Value {
    h.ok("agent_status", json!({}))["status"].clone()
}

#[test]
fn a_fresh_install_opens_without_a_password() {
    let paths = Harness::temp_paths();
    let keys = MemoryKeyStore::new();
    let (_runtime, _mock) = start_agent_with(&paths, Some(keys.clone()));
    let h = Harness::without_setup(paths);
    // The agent created the database with a key in the credential store;
    // the app opens it with the key from the agent.
    let view = wait_unlocked(&h);
    assert_eq!(view["protection"], "systemManaged");
    assert_eq!(view["database"], "encrypted");
    assert_eq!(view["verification"], "confirmationOnly");
    assert_eq!(view["policy"]["autoLock"], "never");
    assert_eq!(keys.ids().len(), 1);
    new_profile(&h, "Work");
    let launch = h.ok("launch_info", json!({}));
    assert_eq!(launch["encryption"]["passwordRequired"], false);

    // A second start of the app (the agent keeps running): no prompt.
    let again = Harness::without_setup(h.paths.clone());
    let view = status(&again);
    assert_eq!(view["unlocked"], true);
    assert_eq!(view["confirmationRequired"], false);
    assert_eq!(
        again.ok("list_profiles", json!({}))["profiles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn sensitive_operations_need_a_fresh_authentication() {
    let paths = Harness::temp_paths();
    let keys = MemoryKeyStore::new();
    let (_runtime, _mock) = start_agent_with(&paths, Some(keys));
    let touch_id = Arc::new(Scripted::new(Some(Method::TouchId), false));
    let h = Harness::with_presence(paths, touch_id.clone());
    wait_unlocked(&h);
    assert_eq!(status(&h)["verification"], "touchId");
    let export = || {
        let path = h.paths.data_dir.join("all.deckpilot").display().to_string();
        h.call(
            "export_configuration",
            json!({ "request": { "path": path, "password": "export secret", "confirmation": "export secret", "profiles": null, "includeModuleSettings": true } }),
        )
    };
    assert_eq!(export().unwrap_err(), "authentication required");
    // A failed system check grants nothing.
    assert!(h.call("authenticate", json!({})).is_err());
    assert_eq!(export().unwrap_err(), "authentication required");
    touch_id
        .succeed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    h.ok("authenticate", json!({}));
    export().unwrap();
    // One authentication allows one operation.
    assert_eq!(export().unwrap_err(), "authentication required");
    assert_eq!(
        touch_id.prompts.load(std::sync::atomic::Ordering::SeqCst),
        2
    );

    // Ordinary work never asks.
    let profile = new_profile(&h, "Work");
    h.ok("set_active_profile", json!({ "profileId": profile }));
    assert_eq!(
        touch_id.prompts.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}

#[test]
fn switching_between_password_and_system_key() {
    let paths = Harness::temp_paths();
    let keys = MemoryKeyStore::new();
    let h = Harness::new_on(paths);
    let (_runtime, _mock) = start_agent_with(&h.paths, Some(keys.clone()));
    h.unlock();
    let profile = new_profile(&h, "Work");
    assert_eq!(status(&h)["protection"], "password");
    assert_eq!(status(&h)["verification"], "password");

    // Sensitive: the current password is required first.
    assert_eq!(
        h.error("use_system_key", json!({})),
        "authentication required"
    );
    assert!(
        h.call("authenticate", json!({ "password": "wrong password" }))
            .is_err()
    );
    h.ok("authenticate", json!({ "password": PASSWORD }));
    let view = h.ok("use_system_key", json!({}));
    assert_eq!(view["protection"], "systemManaged");
    assert_eq!(view["unlocked"], true);
    assert_eq!(keys.ids().len(), 1);
    assert_eq!(agent_security(&h)["protection"], "systemManaged");
    assert!(
        h.error("unlock", json!({ "password": PASSWORD }))
            .contains("system key")
    );
    assert_eq!(
        h.ok("list_profiles", json!({}))["profiles"][0]["id"],
        json!(profile)
    );

    // Back to a password. Without system authentication on this computer,
    // the explicit confirmation is the authentication.
    assert_eq!(
        h.error(
            "use_password",
            json!({ "input": { "password": "another password", "confirmation": "another password" } })
        ),
        "authentication required"
    );
    h.ok("authenticate", json!({}));
    let view = h.ok(
        "use_password",
        json!({ "input": { "password": "another password", "confirmation": "another password" } }),
    );
    assert_eq!(view["protection"], "password");
    h.ok("lock", json!({}));
    assert!(h.call("unlock", json!({ "password": PASSWORD })).is_err());
    h.ok("unlock", json!({ "password": "another password" }));
    assert_eq!(
        h.ok("list_profiles", json!({}))["profiles"][0]["id"],
        json!(profile)
    );
    // The earlier system key is kept, not deleted.
    assert_eq!(keys.ids().len(), 1);
}

#[test]
fn a_locked_system_key_configuration_unlocks_with_the_system_check() {
    let paths = Harness::temp_paths();
    let keys = MemoryKeyStore::new();
    let (_runtime, _mock) = start_agent_with(&paths, Some(keys));
    let touch_id = Arc::new(Scripted::new(Some(Method::TouchId), true));
    let h = Harness::with_presence(paths, touch_id.clone());
    wait_unlocked(&h);
    let view = h.ok("lock", json!({}));
    assert_eq!(view["unlocked"], false);
    assert_eq!(view["lockReason"], "requested");
    // The app restarting meanwhile does not get the key.
    assert_eq!(status(&h)["unlocked"], false);

    touch_id
        .succeed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(h.call("system_unlock", json!({})).is_err());
    assert_eq!(agent_security(&h)["security"], "locked");
    touch_id
        .succeed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let view = h.ok("system_unlock", json!({}));
    assert_eq!(view["unlocked"], true);
    assert_eq!(
        touch_id.prompts.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}

#[test]
fn policies_need_a_way_to_verify_the_user() {
    let paths = Harness::temp_paths();
    let (_runtime, _mock) = start_agent_with(&paths, Some(MemoryKeyStore::new()));
    let h = Harness::without_setup(paths);
    wait_unlocked(&h);
    // System key, no system authentication: locking would be pointless.
    let idle =
        json!({ "policy": { "autoLock": "idle15Minutes", "reauthentication": "sensitiveOnly" } });
    assert!(
        h.error("set_security_policy", idle.clone())
            .contains("does not offer")
    );

    // With a password, the user can be verified.
    let h = Harness::new();
    let (_runtime, _mock) = start_agent(&h.paths);
    h.unlock();
    assert_eq!(
        h.error("set_security_policy", idle.clone()),
        "authentication required"
    );
    h.ok("authenticate", json!({ "password": PASSWORD }));
    let view = h.ok("set_security_policy", idle);
    assert_eq!(view["policy"]["autoLock"], "idle15Minutes");

    // Every launch: the next start asks again.
    h.ok("authenticate", json!({ "password": PASSWORD }));
    h.ok(
        "set_security_policy",
        json!({ "policy": { "autoLock": "idle15Minutes", "reauthentication": "everyLaunch" } }),
    );
    let state = h.window.state::<AppState>();
    let key = state.key().unwrap();
    state.detach();
    state.attach(key, false).unwrap();
    assert_eq!(status(&h)["confirmationRequired"], true);
    h.ok("confirm_identity", json!({ "password": PASSWORD }));
    assert_eq!(status(&h)["unlocked"], true);
}

#[test]
fn key_store_names_match_the_credential_store() {
    assert_eq!(
        crate::commands::security::key_store_name(),
        deckpilot_keystore::platform_store_name()
    );
}

#[test]
fn an_update_does_not_ask_for_anything_with_a_system_key() {
    let paths = Harness::temp_paths();
    let (_runtime, _mock) = start_agent_with(&paths, Some(MemoryKeyStore::new()));
    let h = Harness::without_setup(paths);
    wait_unlocked(&h);
    new_profile(&h, "Work");
    // The previous version was confirmed and recorded its launch.
    {
        use deckpilot_storage::ConfigStore as _;
        let state = h.window.state::<AppState>();
        let store = state.store().unwrap();
        store
            .set_setting("security.confirmedVersion", "0.3.0")
            .unwrap();
        store.record_launch("desktop", "0.3.0").unwrap();
    }
    // First start of the new version: the app gets the key from the agent.
    let again = Harness::without_setup(h.paths.clone());
    let view = status(&again);
    assert_eq!(view["unlocked"], true, "{view}");
    assert_eq!(view["confirmationRequired"], false);
    let launch = again.ok("launch_info", json!({}));
    assert_eq!(launch["launch"]["firstAfterUpdate"], true);
    assert_eq!(launch["encryption"]["passwordRequired"], false);
}

#[test]
fn a_missing_database_needs_an_explicit_decision() {
    let h = Harness::new();
    new_profile(&h, "Work");
    let state = h.window.state::<AppState>();
    state.detach();
    for suffix in ["", "-wal", "-shm"] {
        let mut path = h.paths.database.clone().into_os_string();
        path.push(suffix);
        let _ = std::fs::remove_file(path);
    }
    let view = status(&h);
    assert_eq!(view["database"], "missing");
    assert_eq!(view["databaseMissing"], true);
    // Neither a password setup nor an old key creates a new database.
    assert!(
        h.error(
            "setup_password",
            json!({ "input": { "password": PASSWORD, "confirmation": PASSWORD } })
        )
        .contains("missing")
    );
    assert!(!h.paths.database.exists());

    // Starting over moves the key file aside; then setup works again.
    h.ok("start_new_configuration", json!({}));
    let view = status(&h);
    assert_eq!(view["databaseMissing"], false);
    let aside = std::fs::read_dir(&h.paths.data_dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("deckpilot.vault.orphaned-")
        });
    assert!(aside, "the old key file is kept");
    h.ok(
        "setup_password",
        json!({ "input": { "password": PASSWORD, "confirmation": PASSWORD } }),
    );
    assert_eq!(h.ok("list_profiles", json!({}))["profiles"], json!([]));
}

#[test]
fn the_screen_lock_option_needs_detection() {
    let h = Harness::new();
    let (_runtime, _mock) = start_agent(&h.paths);
    h.unlock();
    assert_eq!(status(&h)["screenLockDetection"], false);
    h.ok("authenticate", json!({ "password": PASSWORD }));
    assert!(
        h.error(
            "set_security_policy",
            json!({ "policy": { "autoLock": "screenLock", "reauthentication": "sensitiveOnly" } })
        )
        .contains("cannot detect the screen lock")
    );
}

#[test]
fn every_deliberate_lock_needs_authentication_to_undo() {
    use crate::commands::security::lock_needs_authentication;
    use deckpilot_ipc::LockReason;
    for reason in [
        LockReason::Requested,
        LockReason::Idle,
        LockReason::Sleep,
        LockReason::ScreenLocked,
    ] {
        assert!(lock_needs_authentication(Some(reason)), "{reason:?}");
    }
    for reason in [LockReason::Startup, LockReason::KeyUnavailable] {
        assert!(!lock_needs_authentication(Some(reason)), "{reason:?}");
    }
    assert!(!lock_needs_authentication(None));
}

// Export and import through the real commands, between two installations.

/// A profile with projects, an action, a workflow and a layout with
/// bindings, written as the configuration studio would.
fn rich_profile(h: &Harness, name: &str) -> deckpilot_core::Profile {
    use deckpilot_core::{DeviceLayout, Profile};
    use deckpilot_storage::ConfigStore as _;
    let id = new_profile(h, name);
    h.ok(
        "create_project",
        json!({ "profileId": id, "project": { "key": "SHOP", "name": "Shop", "resources": { "repository": "shop.repository" } } }),
    );
    log_action(h, &id, "Announce", "{project.key}");
    let state = h.window.state::<AppState>();
    let store = state.store().unwrap();
    let mut profile: Profile = store.load_profile(id.parse().unwrap()).unwrap().unwrap();
    let action = profile.actions[0].id;
    profile
        .workflows
        .push(deckpilot_core::Workflow::new("Start").step(action, deckpilot_core::OnFailure::Stop));
    let layout = DeviceLayout::generate("Main", &models::stream_deck_mk2(), &profile);
    let layout_id = layout.id;
    assert!(
        !layout.pages[0].bindings.is_empty(),
        "the layout has bindings"
    );
    profile.layouts.push(layout);
    profile.activate_layout(layout_id).unwrap();
    store.save_profile(&profile).unwrap();
    profile
}

fn export_to(h: &Harness, file: &std::path::Path, password: Option<&str>) -> Value {
    h.ok("authenticate", json!({ "password": password }));
    h.ok(
        "export_configuration",
        json!({ "request": { "path": file.display().to_string(), "password": "export secret", "confirmation": "export secret", "profiles": null, "includeModuleSettings": true, "includeAppSettings": true } }),
    )
}

fn database_is_encrypted(paths: &AgentPaths) -> bool {
    let header = std::fs::read(&paths.database).unwrap();
    !header.starts_with(b"SQLite format 3")
}

#[test]
fn a_system_key_installation_moves_to_a_password_installation() {
    use deckpilot_storage::ConfigStore as _;

    // Source: system-managed key, Touch ID.
    let source_paths = Harness::temp_paths();
    let source_keys = MemoryKeyStore::new();
    let (_source_runtime, _mock) = start_agent_with(&source_paths, Some(source_keys.clone()));
    let source = Harness::with_presence(
        source_paths,
        Arc::new(Scripted::new(Some(Method::TouchId), true)),
    );
    wait_unlocked(&source);
    let profile = rich_profile(&source, "Work");
    source.ok(
        "set_resource_binding",
        json!({ "key": "shop.repository", "kind": "directory", "value": "/source/machine/shop" }),
    );
    source.ok(
        "set_update_preferences",
        json!({ "preferences": { "mode": "downloadAndAsk", "automaticChecks": false, "checkIntervalHours": 48, "lastCheck": null } }),
    );
    let file = source.paths.data_dir.join("work.deckpilot");
    let result = export_to(&source, &file, None);
    assert_eq!(result["profiles"], 1);
    let bytes = std::fs::read(&file).unwrap();
    for secret in [
        &b"Work"[..],
        b"SHOP",
        b"/source/machine/shop",
        b"downloadAndAsk",
    ] {
        assert!(!bytes.windows(secret.len()).any(|w| w == secret));
    }
    assert_eq!(source_keys.ids().len(), 1);

    // Target: password protection, its own binding for the same resource.
    let target = Harness::new();
    target.ok(
        "set_resource_binding",
        json!({ "key": "shop.repository", "kind": "directory", "value": "/target/machine/shop" }),
    );
    assert!(
        target
            .error(
                "preview_import",
                json!({ "path": file.display().to_string(), "password": "wrong" })
            )
            .contains("password is wrong")
    );
    let preview = target.ok(
        "preview_import",
        json!({ "path": file.display().to_string(), "password": "export secret" }),
    );
    assert_eq!(
        preview["preview"]["settings"]["updates"]["mode"],
        "downloadAndAsk"
    );
    assert_eq!(preview["preview"]["missingResources"], json!([]));
    let summary = target.ok(
        "apply_import",
        json!({ "token": preview["token"], "options": { "profiles": [profile.id.to_string()], "moduleSettings": true, "appSettings": true } }),
    );
    assert_eq!(summary["settings"], json!(["updates"]));

    // Everything arrived as it was: projects, actions, workflows, layouts
    // and their bindings, with the same identifiers.
    let state = target.window.state::<AppState>();
    let imported = state
        .store()
        .unwrap()
        .load_profile(profile.id)
        .unwrap()
        .unwrap();
    assert_eq!(imported.projects, profile.projects);
    assert_eq!(imported.actions, profile.actions);
    assert_eq!(imported.workflows, profile.workflows);
    assert_eq!(imported.layouts, profile.layouts);
    assert_eq!(imported.active_layouts, profile.active_layouts);
    // This computer's binding is kept; the import is not activated.
    let machine = target.ok("get_machine_config", json!({}));
    assert_eq!(machine["bindings"][0]["value"], "/target/machine/shop");
    assert_eq!(
        target.ok("list_profiles", json!({}))["activeProfileId"],
        Value::Null
    );
    let updates = target.ok("update_state", json!({}));
    assert_eq!(updates["preferences"]["mode"], "downloadAndAsk");
    assert_eq!(updates["preferences"]["checkIntervalHours"], 48);
    // Still encrypted with the target's own protection; no plaintext or
    // temporary files were left.
    assert!(database_is_encrypted(&target.paths));
    assert_eq!(status(&target)["protection"], "password");
    let leftovers: Vec<_> = std::fs::read_dir(&target.paths.data_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp") || name.contains("unencrypted"))
        .collect();
    assert_eq!(leftovers, Vec::<String>::new());

    // And back: a password installation exports as well.
    let back = target.paths.data_dir.join("back.deckpilot");
    export_to(&target, &back, Some(PASSWORD));
    assert!(back.exists());
    assert_eq!(source_keys.ids().len(), 1, "no credential was touched");
}

#[test]
fn replacing_with_selected_profiles() {
    let source = Harness::new();
    let home = rich_profile(&source, "Home");
    let work = rich_profile(&source, "Work 2");
    let file = source.paths.data_dir.join("both.deckpilot");
    export_to(&source, &file, Some(PASSWORD));

    let target = Harness::new();
    let local = new_profile(&target, "Local");
    target.ok(
        "set_resource_binding",
        json!({ "key": "shop.repository", "kind": "directory", "value": "/target/shop" }),
    );
    let preview = target.ok(
        "preview_import",
        json!({ "path": file.display().to_string(), "password": "export secret" }),
    );
    assert_eq!(preview["preview"]["localProfiles"][0][0], json!(local));
    let summary = target.ok(
        "apply_import",
        json!({ "token": preview["token"], "options": { "mode": "replace", "profiles": [work.id.to_string()] } }),
    );
    assert_eq!(summary["removed"][0][1], "Local");
    let names: Vec<String> = target.ok("list_profiles", json!({}))["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["Work 2"]);
    assert!(!names.contains(&home.name));
    assert_eq!(
        target.ok("get_machine_config", json!({}))["bindings"][0]["value"],
        "/target/shop"
    );
}

#[test]
fn hostile_archives_are_rejected_without_changes() {
    let h = Harness::new();
    new_profile(&h, "Mine");
    let dir = &h.paths.data_dir;
    let oversized = dir.join("huge.deckpilot");
    std::fs::write(&oversized, vec![b' '; 33 * 1024 * 1024]).unwrap();
    assert!(
        h.error(
            "preview_import",
            json!({ "path": oversized.display().to_string(), "password": "x" })
        )
        .contains("too large")
    );
    let garbage = dir.join("garbage.deckpilot");
    std::fs::write(
        &garbage,
        br#"{"header":"{\"format\":\"deckpilot-configuration\"}","ciphertext":"AAAA"}"#,
    )
    .unwrap();
    assert!(
        h.call(
            "preview_import",
            json!({ "path": garbage.display().to_string(), "password": "x" })
        )
        .is_err()
    );
    // Applying without a preview, or with a made-up token, does nothing.
    assert!(
        h.call(
            "apply_import",
            json!({ "token": 12345, "options": { "profiles": [] } })
        )
        .is_err()
    );
    assert_eq!(
        h.ok("list_profiles", json!({}))["profiles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_failed_import_changes_nothing_and_can_be_retried() {
    let source = Harness::new();
    rich_profile(&source, "Imported");
    let file = source.paths.data_dir.join("one.deckpilot");
    export_to(&source, &file, Some(PASSWORD));

    let target = Harness::new();
    new_profile(&target, "Existing");
    let preview = |h: &Harness| {
        h.ok(
            "preview_import",
            json!({ "path": file.display().to_string(), "password": "export secret" }),
        )
    };
    let first = preview(&target);
    // Another connection holds the write lock: the import transaction cannot
    // be written and is rolled back as a whole.
    let key = target.window.state::<AppState>().key().unwrap();
    let blocker = deckpilot_storage::SqliteStore::open(
        &target.paths.database,
        &deckpilot_storage::Encryption::Key(key),
    )
    .unwrap();
    blocker
        .connection_for_tests()
        .execute_batch("BEGIN EXCLUSIVE;")
        .unwrap();
    assert!(
        target
            .call(
                "apply_import",
                json!({ "token": first["token"], "options": { "profiles": [first["preview"]["profiles"][0]["id"]], "moduleSettings": true } }),
            )
            .is_err()
    );
    blocker
        .connection_for_tests()
        .execute_batch("ROLLBACK;")
        .unwrap();
    drop(blocker);
    let names = |h: &Harness| -> Vec<String> {
        h.ok("list_profiles", json!({}))["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names(&target), ["Existing"]);
    // The file is unchanged and the import can simply be repeated.
    let second = preview(&target);
    target.ok(
        "apply_import",
        json!({ "token": second["token"], "options": { "profiles": [second["preview"]["profiles"][0]["id"]] } }),
    );
    assert_eq!(names(&target), ["Existing", "Imported"]);
}

#[test]
fn credentials_are_never_read_back_or_exported() {
    let source = Harness::new();
    new_profile(&source, "Work");
    let listed = source.ok(
        "set_credential",
        json!({ "id": "api.token", "description": "Staging API", "secret": "s3cret-value" }),
    );
    assert_eq!(listed[0]["id"], "api.token");
    assert_eq!(listed[0]["description"], "Staging API");
    assert!(!listed.to_string().contains("s3cret-value"));
    // A new description keeps the secret; an invalid id is refused.
    source.ok(
        "set_credential",
        json!({ "id": "api.token", "description": "Renamed", "secret": null }),
    );
    assert!(
        source
            .call(
                "set_credential",
                json!({ "id": "Bad Id", "description": "", "secret": "x" })
            )
            .is_err()
    );
    assert!(
        !source
            .ok("list_credentials", json!({}))
            .to_string()
            .contains("s3cret-value")
    );

    let file = source.paths.data_dir.join("credentials.deckpilot");
    export_to(&source, &file, Some(PASSWORD));
    let target = Harness::new();
    let preview = target.ok(
        "preview_import",
        json!({ "path": file.display().to_string(), "password": "export secret" }),
    );
    let profile = preview["preview"]["profiles"][0]["id"].clone();
    target.ok(
        "apply_import",
        json!({ "token": preview["token"], "options": { "profiles": [profile], "moduleSettings": true, "appSettings": true } }),
    );
    assert_eq!(target.ok("list_credentials", json!({})), json!([]));

    source.ok("delete_credential", json!({ "id": "api.token" }));
    assert_eq!(source.ok("list_credentials", json!({})), json!([]));
}

fn png_base64(width: u32, height: u32, color: [u8; 3]) -> String {
    use base64::Engine as _;
    let png = deckpilot_render::frame_png(&deckpilot_device::Frame::filled(width, height, color))
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(png)
}

#[test]
#[allow(clippy::too_many_lines)] // One flow from import to deletion.
fn icons_are_imported_shown_previewed_and_deleted() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let action = log_action(&h, &profile_id, "Hello", "hi");

    let icon = h.ok(
        "import_icon",
        json!({ "profileId": profile_id, "name": "Logo", "data": png_base64(600, 300, [200, 30, 30]) }),
    );
    assert_eq!(
        (icon["width"].clone(), icon["height"].clone()),
        (json!(256), json!(128))
    );
    assert!(
        icon["dataUrl"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    // The same image again is the same icon.
    let again = h.ok(
        "import_icon",
        json!({ "profileId": profile_id, "name": "Logo 2", "data": png_base64(600, 300, [200, 30, 30]) }),
    );
    assert_eq!(again["id"], icon["id"]);
    // Wrong types, damaged and oversized files are refused.
    for data in [
        "R0lGODlhAQABAAAAACw=".to_owned(),
        "not base64!".to_owned(),
        "A".repeat(3 * 1024 * 1024),
    ] {
        assert!(
            h.call(
                "import_icon",
                json!({ "profileId": profile_id, "name": "x", "data": data }),
            )
            .is_err()
        );
    }
    let icons = h.ok("list_icons", json!({ "profileId": profile_id }));
    assert_eq!(icons.as_array().unwrap().len(), 1);

    let mut layout = h.ok(
        "create_layout",
        json!({ "profileId": profile_id, "modelId": "elgato.streamdeck.mk2", "name": "Main", "generate": false }),
    );
    let page = layout["homePage"].clone();
    let binding = |appearance: Value| {
        json!([{
            "control": { "kind": "key", "index": 0 },
            "trigger": { "type": "press" },
            "target": { "type": "action", "action": action["id"] },
            "appearance": appearance,
        }])
    };
    // Invalid colours and unknown icons are refused.
    layout["pages"][0]["bindings"] = binding(json!({ "background": "red" }));
    assert!(
        h.call(
            "save_layout",
            json!({ "profileId": profile_id, "layout": layout })
        )
        .is_err()
    );
    layout["pages"][0]["bindings"] =
        binding(json!({ "icon": { "id": "00000000-0000-4000-8000-000000000000" } }));
    let error = h
        .call(
            "save_layout",
            json!({ "profileId": profile_id, "layout": layout }),
        )
        .unwrap_err();
    assert!(error.as_str().unwrap().contains("icon"), "{error}");

    layout["pages"][0]["bindings"] = binding(json!({
        "background": "#102030",
        "fontSize": "large",
        "titleAlign": "top",
        "icon": { "id": icon["id"], "fit": "crop" }
    }));
    let saved = h.ok(
        "save_layout",
        json!({ "profileId": profile_id, "layout": layout }),
    );
    assert_eq!(
        saved["pages"][0]["bindings"][0]["appearance"]["fontSize"],
        "large"
    );

    // The preview carries the appearance, and on request the key image.
    let args = |render: Value| json!({ "profileId": profile_id, "layout": saved, "pageId": page, "projectId": null, "render": render });
    let faces = h.ok("preview_layout", args(json!(true)));
    assert_eq!(faces[0]["face"]["appearance"]["background"], "#102030");
    let image = faces[0]["image"].as_str().unwrap();
    assert!(image.starts_with("data:image/png;base64,"));
    // Empty keys are images too; without `render` there are none.
    assert!(faces[1]["image"].is_string());
    let plain = h.ok("preview_layout", args(Value::Null));
    assert!(plain[0].get("image").is_none());

    // An icon in use stays.
    assert!(
        h.call(
            "delete_icon",
            json!({ "profileId": profile_id, "iconId": icon["id"] })
        )
        .is_err()
    );
    let mut cleared = saved.clone();
    cleared["pages"][0]["bindings"] = binding(json!({}));
    let saved = h.ok(
        "save_layout",
        json!({ "profileId": profile_id, "layout": cleared }),
    );
    // A default appearance is not stored.
    assert!(saved["pages"][0]["bindings"][0].get("appearance").is_none());
    h.ok(
        "delete_icon",
        json!({ "profileId": profile_id, "iconId": icon["id"] }),
    );
    let icons = h.ok("list_icons", json!({ "profileId": profile_id }));
    assert_eq!(icons, json!([]));
}

#[test]
#[allow(clippy::too_many_lines)] // One flow through the page commands.
fn pages_are_ordered_moved_and_copied() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let create = |name: &str| {
        h.ok(
            "create_layout",
            json!({ "profileId": profile_id, "modelId": "elgato.streamdeck.mk2", "name": name, "generate": false }),
        )
    };
    let mut main = create("Main");
    let second = create("Second");
    let home = main["homePage"].clone();
    let (tools, git) = (
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
    );
    // An old-style layout: folders only through openPage bindings, and Git
    // has no Back key.
    main["pages"][0]["bindings"] = json!([
        { "control": { "kind": "key", "index": 1 }, "trigger": { "type": "press" }, "target": { "type": "openPage", "page": tools } }
    ]);
    main["pages"].as_array_mut().unwrap().extend([
        json!({ "id": tools, "name": "Tools", "bindings": [
            { "control": { "kind": "key", "index": 0 }, "trigger": { "type": "press" }, "target": { "type": "back" } },
            { "control": { "kind": "key", "index": 1 }, "trigger": { "type": "press" }, "target": { "type": "openPage", "page": git } }
        ]}),
        json!({ "id": git, "name": "Git", "bindings": [] }),
    ]);
    let saved = h.ok(
        "save_layout",
        json!({ "profileId": profile_id, "layout": main }),
    );
    // Saved without edits: no parents are added.
    assert!(!saved.to_string().contains("parent"));
    assert!(saved.get("explicitTree").is_none());

    let tree = h.ok("page_tree", json!({ "layout": saved }));
    let node = |id: &str| {
        tree["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(node(tools)["parent"], home);
    assert_eq!(node(git)["parent"], tools);
    assert_eq!(node(git)["depth"], 2);
    assert_eq!(node(git)["stored"], false);
    assert_eq!(node(git)["automaticBack"]["index"], 0);
    assert_eq!(tree["warnings"].as_array().unwrap().len(), 1);

    let layout_id = saved["id"].clone();
    // Reordering needs every page once and stores the tree.
    assert!(
        h.call(
            "reorder_pages",
            json!({ "profileId": profile_id, "layoutId": layout_id, "order": [home, git] }),
        )
        .is_err()
    );
    let reordered = h.ok(
        "reorder_pages",
        json!({ "profileId": profile_id, "layoutId": layout_id, "order": [git, home, tools] }),
    );
    assert_eq!(reordered["pages"][0]["id"], git);
    assert_eq!(reordered["explicitTree"], true);
    assert_eq!(reordered["pages"][0]["parent"], tools);

    // A cycle is refused, also through save_layout.
    assert!(
        h.call(
            "move_page",
            json!({ "profileId": profile_id, "layoutId": layout_id, "pageId": tools, "parentId": git }),
        )
        .is_err()
    );
    let mut cyclic = reordered.clone();
    cyclic["pages"][2]["parent"] = json!(git);
    assert!(
        h.call(
            "save_layout",
            json!({ "profileId": profile_id, "layout": cyclic })
        )
        .is_err()
    );

    // Copy Tools (with Git) to the other layout.
    let copied = h.ok(
        "copy_page",
        json!({ "profileId": profile_id, "layoutId": layout_id, "pageId": tools, "targetLayoutId": second["id"] }),
    );
    assert_ne!(copied["page"], tools);
    let target = copied["layouts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == second["id"])
        .unwrap()
        .clone();
    assert_eq!(target["pages"].as_array().unwrap().len(), 3);

    // Move Tools within its layout to the top level, then to the other
    // layout: the key that opened it goes with it.
    let moved = h.ok(
        "move_page",
        json!({ "profileId": profile_id, "layoutId": layout_id, "pageId": tools }),
    );
    assert_eq!(moved["removedKeys"], 0);
    let moved = h.ok(
        "move_page",
        json!({ "profileId": profile_id, "layoutId": layout_id, "pageId": tools, "targetLayoutId": second["id"] }),
    );
    assert_eq!(moved["page"], tools);
    assert_eq!(moved["removedKeys"], 1);
    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    let layouts = view["profile"]["layouts"].as_array().unwrap();
    assert_eq!(layouts[0]["pages"].as_array().unwrap().len(), 1);
    assert_eq!(layouts[1]["pages"].as_array().unwrap().len(), 5);
    // The home page stays.
    assert!(
        h.call(
            "move_page",
            json!({ "profileId": profile_id, "layoutId": layout_id, "pageId": home, "targetLayoutId": second["id"] }),
        )
        .is_err()
    );
}

#[test]
fn restarting_the_agent_never_ends_running_workflows_silently() {
    let h = Harness::new();
    let profile = h.ok(
        "create_profile",
        json!({ "name": "Work", "description": "" }),
    );
    let profile_id = profile["id"].as_str().unwrap().to_owned();
    let wait = h.ok(
        "create_action",
        json!({
            "profileId": profile_id,
            "action": {
                "name": "Wait",
                "actionType": "general.wait",
                "parameters": { "seconds": { "type": "integer", "value": 60 } }
            }
        }),
    );
    let workflow = h.ok(
        "save_workflow",
        json!({ "profileId": profile_id, "workflow": {
            "id": deckpilot_core::WorkflowId::new(), "name": "Long", "steps": [],
            "graph": { "version": 1,
                "nodes": [
                    {"id": "start", "type": "trigger"},
                    {"id": "wait", "type": "action", "action": wait["id"]}
                ],
                "edges": [{"from": "start", "to": "wait"}] }
        } }),
    );
    let (_runtime, _mock) = start_agent(&h.paths);
    h.unlock();
    wait_for(&h, |s| {
        s["running"] == true && s["status"]["activeProfile"]["name"] == "Work"
    });
    let execution = h.ok(
        "start_workflow",
        json!({ "profileId": profile_id, "workflowId": workflow["id"] }),
    );
    let status = wait_for(&h, |s| s["status"]["runningExecutions"] == 1);
    assert_eq!(status["protocol"], deckpilot_ipc::PROTOCOL_VERSION);
    assert!(
        status["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("backgroundExecutions"))
    );

    // Without confirmation the restart is refused and the run goes on.
    let refused = h.error("restart_agent", json!({}));
    assert!(refused.contains("1 workflow is running"), "{refused}");
    wait_for(&h, |s| {
        s["running"] == true && s["status"]["runningExecutions"] == 1
    });

    h.ok(
        "cancel_execution",
        json!({ "executionId": execution["id"] }),
    );
    wait_for(&h, |s| s["status"]["runningExecutions"] == 0);
}

/// The work session commands (protocol 8) against a real agent: state,
/// settings, history, notices and the `agent-work-session` event. The
/// session is operated from the "key" (the agent's test entry point):
/// there is no command for it (D11).
#[allow(clippy::too_many_lines)] // One session, followed through.
#[test]
fn the_work_session_is_read_and_configured_through_the_agent() {
    use deckpilot_agent::{WorkSessionOperation, WorkSessionOutcome};
    use deckpilot_core::gesture::Gesture;
    use tauri::Listener;

    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    h.ok("set_active_profile", json!({ "profileId": profile_id }));
    let start = h.ok(
        "save_workflow",
        json!({ "profileId": profile_id, "workflow": {
            "id": deckpilot_core::WorkflowId::new(), "name": "Prepare", "steps": [],
            "graph": { "version": 1, "nodes": [{"id": "start", "type": "trigger"}], "edges": [] }
        } }),
    );

    // Without an agent the page is told to start it.
    let error = h.error("get_work_session", json!({}));
    assert!(error.contains("start the DeckPilot agent"), "{error}");

    let (runtime, _mock, agent) = launch_agent(&h.paths, None);
    h.unlock();
    wait_for(&h, |s| {
        s["running"] == true && s["status"]["activeProfile"]["name"] == "Work"
    });
    let status = h.ok("agent_status", json!({}));
    assert!(
        status["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("workSession"))
    );
    // The page appears: the backend reports the module and its page.
    let modules = h.ok("list_modules", json!({}));
    let module = modules
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "work-session")
        .unwrap()
        .clone();
    assert_eq!(module["enabled"], true);
    assert_eq!(module["actionTypes"], json!([]));
    assert_eq!(module["pages"][0]["id"], "work-session");

    let idle = h.ok("get_work_session", json!({}));
    assert_eq!(idle["state"], "idle");
    assert!(idle["asOfMs"].as_i64().unwrap() > 0);

    // Settings: defaults, then saved; a field the agent does not store is
    // refused rather than dropped.
    let defaults = h.ok(
        "get_work_session_settings",
        json!({ "profileId": profile_id }),
    );
    assert_eq!(
        defaults,
        json!({
            "profile": profile_id,
            "startWorkflow": null,
            "verificationWorkflow": null,
            "approvalTtlSeconds": 600,
            "reviewGapSeconds": 900,
            "elapsedFormat": "minutes",
        })
    );
    let settings = json!({
        "startWorkflow": start["id"],
        "verificationWorkflow": null,
        "approvalTtlSeconds": 300,
        "reviewGapSeconds": 600,
        "elapsedFormat": "seconds",
    });
    let mut with_timings = settings.clone();
    with_timings["doubleClickMs"] = json!(250);
    assert!(
        h.call(
            "save_work_session_settings",
            json!({ "profileId": profile_id, "settings": with_timings }),
        )
        .is_err()
    );
    let invalid = h.error(
        "save_work_session_settings",
        json!({ "profileId": profile_id, "settings": {
            "startWorkflow": null, "verificationWorkflow": start["id"],
            "approvalTtlSeconds": 300, "reviewGapSeconds": 600, "elapsedFormat": "minutes",
        } }),
    );
    assert!(invalid.contains("canFinish"), "{invalid}");
    let saved = h.ok(
        "save_work_session_settings",
        json!({ "profileId": profile_id, "settings": settings }),
    );
    assert_eq!(saved["startWorkflow"], start["id"]);
    assert_eq!(saved["elapsedFormat"], "seconds");
    assert!(saved["updatedAtMs"].as_i64().is_some());
    // What the page got back can be sent again as it is.
    h.ok(
        "save_work_session_settings",
        json!({ "profileId": profile_id, "settings": saved }),
    );
    assert_eq!(
        h.ok(
            "get_work_session_settings",
            json!({ "profileId": profile_id }),
        )["approvalTtlSeconds"],
        300
    );

    // The key starts a session; the app hears of it.
    let (sender, received) = std::sync::mpsc::channel::<Value>();
    let app = h.window.app_handle().clone();
    app.listen(crate::agent::WORK_SESSION_EVENT, move |event| {
        let _ = sender.send(serde_json::from_str(event.payload()).unwrap());
    });
    crate::agent::spawn_bridge(app, h.paths.clone());
    std::thread::sleep(std::time::Duration::from_millis(300));
    let started = runtime.block_on(agent.work_session_operation(
        WorkSessionOperation::Start,
        Gesture::Click,
        None,
    ));
    assert_eq!(started, Some(WorkSessionOutcome::Applied));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let working = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let snapshot = received.recv_timeout(left).expect("agent-work-session");
        if snapshot["state"] == "working" {
            break snapshot;
        }
    };
    let session = working["session"].as_str().unwrap().to_owned();
    let current = h.ok("get_work_session", json!({}));
    assert_eq!(current["state"], "working");
    assert_eq!(current["session"], session.as_str());

    let history = h.ok("list_work_sessions", json!({ "limit": 10 }));
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["session"], session.as_str());
    assert_eq!(history[0]["state"], "working");
    assert_eq!(history[0]["intervals"].as_array().unwrap().len(), 1);
    assert_eq!(
        h.ok(
            "list_work_sessions",
            json!({ "limit": 10, "before": history[0]["requestedAtMs"] }),
        ),
        json!([])
    );
    assert!(h.call("list_work_sessions", json!({ "limit": 0 })).is_err());
    let missing = h.error(
        "resolve_work_session_review",
        json!({ "session": session, "review": 1, "count": false }),
    );
    assert!(missing.contains("no such review"), "{missing}");
    let cleared = h.ok("clear_work_session_notices", json!({ "session": session }));
    assert_eq!(cleared["state"], "working");
    let cleared = h.ok("clear_work_session_notices", json!({ "session": null }));
    assert_eq!(cleared["state"], "working");

    // Work sessions cannot be turned off while one is active.
    let refused = h.error(
        "set_module_enabled",
        json!({ "moduleId": "work-session", "enabled": false }),
    );
    assert!(refused.contains("a work session is active"), "{refused}");

    h.ok("stop_agent", json!({}));
    wait_for(&h, |s| s["running"] == false);
}

// One scenario through every step of the safe deletion.
#[allow(clippy::too_many_lines)]
#[test]
fn referenced_actions_are_replaced_or_unbound_before_they_are_deleted() {
    let h = Harness::new();
    let profile_id = new_profile(&h, "Work");
    let hello = log_action(&h, &profile_id, "Hello", "hi");
    let other = log_action(&h, &profile_id, "Other", "ho");
    let ids = json!({ "profileId": profile_id, "actionId": hello["id"] });

    // Used on a key, in a step workflow and in a graph node of a workflow
    // with a trigger.
    let mut layout = h.ok(
        "create_layout",
        json!({ "profileId": profile_id, "modelId": "elgato.streamdeck.mk2", "name": "Main", "generate": false }),
    );
    layout["pages"][0]["bindings"] = json!([{
        "control": { "kind": "key", "index": 0 },
        "trigger": { "type": "press" },
        "target": { "type": "action", "action": hello["id"] }
    }]);
    let layout = h.ok(
        "save_layout",
        json!({ "profileId": profile_id, "layout": layout }),
    );
    h.ok(
        "save_workflow",
        json!({
            "profileId": profile_id,
            "workflow": {
                "id": "00000000-0000-4000-8000-000000000001",
                "name": "Steps",
                "steps": [{ "action": hello["id"], "onFailure": "stop" }]
            }
        }),
    );
    let policy = json!({ "onError": "stop", "retry": { "attempts": 0, "backoffMs": 0 } });
    h.ok(
        "save_workflow",
        json!({
            "profileId": profile_id,
            "workflow": {
                "id": "00000000-0000-4000-8000-000000000002",
                "name": "Morning",
                "steps": [],
                "triggers": [{ "type": "schedule", "schedule": { "every": "daily", "at": "09:00" } }],
                "graph": {
                    "version": 1,
                    "nodes": [
                        { "id": "start", "type": "trigger", "position": { "x": 0, "y": 0 } },
                        { "id": "say", "label": "Say hello", "type": "action", "action": hello["id"],
                          "policy": policy, "position": { "x": 200, "y": 0 } }
                    ],
                    "edges": [{ "from": "start", "port": "out", "to": "say" }]
                }
            }
        }),
    );

    let references = h.ok("action_references", ids.clone());
    let sites: Vec<&Value> = references
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["site"])
        .collect();
    let kinds: Vec<&str> = sites.iter().map(|s| s["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["binding", "step", "node", "trigger"]);
    assert_eq!(references[0]["ownerName"], "Main");
    assert_eq!(references[2]["detail"], "Say hello");
    let (binding, step, node, trigger) = (sites[0], sites[1], sites[2], sites[3]);

    // Deleting is refused while anything uses it, and changes nothing.
    let refused = h.error("delete_action", ids.clone());
    assert!(refused.contains("still used in 4 place(s)"), "{refused}");
    assert_eq!(h.ok("action_references", ids.clone()), references);

    // A graph node cannot be unbound; a trigger is never changed itself.
    let sites_of = |list: &[&Value]| json!({ "type": "sites", "sites": list });
    let error = h.error(
        "unbind_action_references",
        json!({ "profileId": profile_id, "actionId": hello["id"], "selection": sites_of(&[node]) }),
    );
    assert!(error.contains("cannot be unbound"), "{error}");
    let error = h.error(
        "replace_action_references",
        json!({ "profileId": profile_id, "actionId": hello["id"], "replacement": other["id"],
                "selection": sites_of(&[trigger]) }),
    );
    assert!(error.contains("trigger"), "{error}");
    assert_eq!(h.ok("action_references", ids.clone()), references);

    // Replace the step and the node, unbind the key.
    let change = h.ok(
        "replace_action_references",
        json!({ "profileId": profile_id, "actionId": hello["id"], "replacement": other["id"],
                "selection": sites_of(&[step, node]) }),
    );
    assert_eq!(change["changed"], json!([step, node]));
    // Same action type: nothing was cleared.
    assert_eq!(change["clearedSettings"], json!([]));
    let change = h.ok(
        "unbind_action_references",
        json!({ "profileId": profile_id, "actionId": hello["id"], "selection": sites_of(&[binding]) }),
    );
    assert_eq!(change["changed"], json!([binding]));

    assert_eq!(h.ok("action_references", ids.clone()), json!([]));
    h.ok("delete_action", ids);

    // The other action now has the step, the node and with it the trigger;
    // the key is empty.
    let moved = h.ok(
        "action_references",
        json!({ "profileId": profile_id, "actionId": other["id"] }),
    );
    let kinds: Vec<&str> = moved
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["site"]["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["step", "node", "trigger"]);
    let view = h.ok("get_profile", json!({ "profileId": profile_id }));
    let page = view["profile"]["layouts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == layout["id"])
        .unwrap()["pages"][0]
        .clone();
    assert_eq!(page["bindings"], json!([]));
}
