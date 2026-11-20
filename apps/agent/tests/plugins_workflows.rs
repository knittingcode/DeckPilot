//! `deckpilot.workflows` end to end through the real agent and host
//! (feature `plugins`; see `support/first_party.rs`): a manifest-only
//! plugin, `default` class as `bundled.json` releases it.
//!
//! * `a_fresh_install_gets_the_workflows_screen`: seeded and enabled at
//!   the first start with its one low-tier permission, no actions, and the
//!   `workflows` view and its navigation entry listed, translated;
//! * `workflows_run_without_the_plugin`: the engine stays in the core:
//!   with the plugin disabled its view is no longer listed (host views
//!   only while enabled), and a workflow still runs.
#![cfg(feature = "plugins")]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use deckpilot_core::{ExecutionState, Profile};
use deckpilot_ipc::plugins::{PermissionStatus, PermissionTier, PluginState};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Env, FirstParty};

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.workflows",
    dir: "workflows",
    krate: "deckpilot-workflows",
    released: "default",
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fresh_install_gets_the_workflows_screen() {
    let env = Env::new(
        "plugins-workflows",
        PLUGIN,
        BundleClass::Default,
        None,
        None,
    );
    let agent = env.start().await;
    let summary = agent.wait_state(PluginState::Enabled).await;
    assert!(summary.actions.is_empty(), "{summary:?}");
    assert_eq!(summary.name, "Workflows");
    let views: Vec<(String, String, String)> = summary
        .navigation
        .iter()
        .map(|e| (e.id.clone(), e.view.kind.clone(), e.label.clone()))
        .collect();
    assert_eq!(
        views,
        [(
            "workflows".to_owned(),
            "workflows".to_owned(),
            "Workflows".to_owned()
        )]
    );
    let details = agent.details().await;
    let permissions: Vec<(String, PermissionTier, PermissionStatus)> = details
        .permissions
        .iter()
        .map(|p| (p.key.clone(), p.tier, p.status))
        .collect();
    assert_eq!(
        permissions,
        [(
            "hostViews".to_owned(),
            PermissionTier::Low,
            PermissionStatus::Granted
        )]
    );
    let justification = details.permissions[0].justification.as_deref().unwrap();
    assert!(!justification.starts_with("t:"), "{justification}");
    // Nothing to invoke.
    let refused = agent
        .request(deckpilot_ipc::Request::InvokePluginAction {
            plugin: PLUGIN.id.into(),
            action: "run".into(),
            params: std::collections::BTreeMap::default(),
        })
        .await;
    assert!(
        !matches!(
            refused,
            Ok(deckpilot_ipc::Response::PluginActionResult {
                outcome: deckpilot_ipc::plugins::PluginActionOutcome::Ok { .. }
            })
        ),
        "{refused:?}"
    );
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflows_run_without_the_plugin() {
    let mut profile = Profile::new("Work");
    let workflow = support::waiting_workflow("Short", 10);
    let id = workflow.id;
    profile.workflows.push(workflow);
    let env = Env::new(
        "plugins-workflows-off",
        PLUGIN,
        BundleClass::Default,
        Some((&profile, None)),
        None,
    );
    let agent = env.start().await;
    agent.wait_state(PluginState::Enabled).await;
    agent.disable().await;
    let summary = agent.summary().await.unwrap();
    assert_eq!(summary.state, PluginState::Disabled);
    // The app's own Workflows screen is listed only while the plugin is
    // enabled: disabled, it has no entry (plugin API 1.9, `hostViews`).
    assert_eq!(summary.navigation, Vec::new(), "{summary:?}");
    let record = agent.start_workflow(id).await;
    assert_eq!(
        record.summary.state,
        ExecutionState::Succeeded,
        "{record:#?}"
    );
    // A restart keeps the user's decision.
    agent.stop().await;
    let agent = env.start().await;
    let summary = agent.seeded().await;
    assert_eq!(summary.state, PluginState::Disabled, "{}", agent.log());
    agent.enable().await;
    let summary = agent.wait_state(PluginState::Enabled).await;
    assert_eq!(summary.navigation.len(), 1, "{summary:?}");
    agent.stop().await;
}
