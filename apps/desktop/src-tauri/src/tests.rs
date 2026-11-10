//! Exercises the commands through Tauri's mock IPC runtime, using the same
//! argument names and JSON shapes as the frontend (`src/lib/api.ts`).

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::state::AppState;

struct Harness {
    window: WebviewWindow<tauri::test::MockRuntime>,
    data_dir: std::path::PathBuf,
}

impl Harness {
    fn new() -> Self {
        let data_dir = std::env::temp_dir().join(format!(
            "deckpilot-desktop-test-{}",
            deckpilot_core::ProfileId::new()
        ));
        let app = mock_builder()
            .manage(AppState::open(&data_dir).expect("open state"))
            .invoke_handler(crate::commands::handler())
            .build(mock_context(noop_assets()))
            .expect("build app");
        let window = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .expect("build window");
        Self { window, data_dir }
    }

    fn call(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
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
        let _ = std::fs::remove_dir_all(&self.data_dir);
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

    h.ok(
        "select_project",
        json!({ "profileId": profile_id, "projectId": project_ids[0] }),
    );
    let result = h.ok(
        "run_action",
        json!({ "profileId": profile_id, "actionId": action_id }),
    );
    assert_eq!(result["status"], "succeeded", "{result}");
    assert!(result["message"].as_str().unwrap().ends_with("/src/shop"));

    // CMR has no binding on this machine: the action fails without side effects.
    h.ok(
        "select_project",
        json!({ "profileId": profile_id, "projectId": project_ids[1] }),
    );
    let result = h.ok(
        "run_action",
        json!({ "profileId": profile_id, "actionId": action_id }),
    );
    assert_eq!(result["status"], "failed");

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
    assert_eq!(models[0]["support"], "descriptorOnly");
}
