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
                "actionType": "deckpilot.resource.resolve",
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
            "action": { "name": "Broken", "actionType": "deckpilot.core.log", "parameters": {} }
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
            .any(|t| t["id"] == "deckpilot.core.log")
    );
    let models = h.ok("list_device_models", json!({}));
    assert_eq!(models[0]["id"], "elgato.streamdeck.mk2");
    assert_eq!(models[0]["support"], "unverified");
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
                "actionType": "deckpilot.core.log",
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
        "create_default_layout",
        json!({ "profileId": profile_id, "modelId": "elgato.streamdeck.mk2" }),
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
