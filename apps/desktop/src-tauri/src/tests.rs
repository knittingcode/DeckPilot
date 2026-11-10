//! Exercises the commands through Tauri's mock IPC runtime, using the same
//! argument names and JSON shapes as the frontend (`src/lib/api.ts`).

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use deckpilot_agent::{Agent, AgentOptions, InstanceLock};
use deckpilot_device::{mock::MockBackend, models};
use deckpilot_ipc::AgentPaths;

use crate::state::AppState;

/// Origin the bundled frontend is served from. Requests from any other origin
/// are treated as remote and rejected by Tauri's access control.
const APP_ORIGIN: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

struct Harness {
    window: WebviewWindow<tauri::test::MockRuntime>,
    paths: AgentPaths,
}

impl Harness {
    fn new() -> Self {
        let data_dir = std::env::temp_dir().join(format!(
            "deckpilot-desktop-test-{}",
            deckpilot_core::ProfileId::new()
        ));
        let paths = AgentPaths::in_dir(data_dir);
        let app = mock_builder()
            .manage(AppState::open(paths.clone()).expect("open state"))
            .manage(crate::updates::UpdateManager::default())
            .invoke_handler(crate::commands::handler())
            .build(mock_context(noop_assets()))
            .expect("build app");
        let window = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .expect("build window");
        Self { window, paths }
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
                "actionType": "projects.resolve-resource",
                "parameters": {
                    "resource": { "type": "resource", "value": { "type": "projectRole", "role": "repository" } }
                }
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
    assert_eq!(launch["encryption"]["passwordRequired"], false);
}

/// Starts a real agent with a simulated MK.2 on its own runtime.
fn start_agent(paths: &AgentPaths) -> (tokio::runtime::Runtime, MockBackend) {
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
    };
    let lock = InstanceLock::acquire(&paths.lock_file).unwrap();
    runtime.block_on(async {
        let agent = Agent::start(options, lock).unwrap();
        tokio::spawn(agent.run(std::future::pending()));
    });
    (runtime, mock)
}

fn wait_for(h: &Harness, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
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
    assert_eq!(ids, ["general", "projects"]);
    assert_eq!(modules[1]["pages"][0]["id"], "projects");
    assert_eq!(modules[1]["actionTypes"].as_array().unwrap().len(), 5);
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
    assert_eq!(modules[1]["enabled"], false);
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
    assert_eq!(configured[1]["configValues"]["wrapAround"]["value"], false);
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
                "parameters": { "project": { "type": "project", "value": shop_id } }
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
