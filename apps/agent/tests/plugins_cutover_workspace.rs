//! The Workspace cut-over of the extraction (WP-C1,
//! `docs/marketplace/module-migration.md` §11 D4, §7 X13) end to end,
//! through the real agent, the real sandboxed host and
//! `plugins/first-party/workspace` built to wasm and bundled as the
//! release bundles it (class `migration`), on a configuration with stored
//! `workspace.*` keys. The migration waits for `resources:open` with the
//! legacy keys unavailable; after the user's consent the stored
//! `workspace.open-urls` key opens its page through the plugin
//! (`url.openParam`), and a stored line above the review's bound
//! (1,000,000, §7 X4) is named by the migration and refused with the
//! reason, never clamped.
#![cfg(feature = "plugins")]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActionTypeId, MachineConfig, MachineResourceBinding, Os,
    ParameterValue, Profile, ResourceKey, ResourceKind, ResourceRef,
};
use deckpilot_ipc::plugins::{
    ConsentDecision, MigrationPhase, MigrationRoute, ModuleMigrationInfo,
};
use deckpilot_ipc::{Request, Response};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Agent, Env, FirstParty, SETTLE};

const WORKSPACE: FirstParty = FirstParty {
    id: "deckpilot.workspace",
    dir: "workspace",
    krate: "deckpilot-workspace",
    released: "migration",
};

/// A legacy key, as a 0.9.x app stored it.
fn legacy(
    profile: &mut Profile,
    action: &str,
    parameters: &[(&str, ParameterValue)],
) -> ActionInstanceId {
    let id = ActionInstanceId::new();
    profile.actions.push(deckpilot_core::ActionInstance {
        id,
        name: action.into(),
        action_type: ActionTypeId::new(action).unwrap(),
        parameters: parameters
            .iter()
            .fold(ActionParameters::new(), |p, (k, v)| p.with(*k, v.clone())),
        description: String::new(),
        icon: None,
        label: None,
    });
    id
}

async fn migration(agent: &Agent, module: &str) -> ModuleMigrationInfo {
    match agent.ok(Request::ListModuleMigrations).await {
        Response::ModuleMigrations { migrations } => migrations
            .into_iter()
            .find(|m| m.module == module)
            .unwrap_or_else(|| panic!("no migration of {module}\n{}", agent.log())),
        other => panic!("unexpected {other:?}"),
    }
}

async fn wait_for(
    agent: &Agent,
    module: &str,
    done: impl Fn(&ModuleMigrationInfo) -> bool,
) -> ModuleMigrationInfo {
    let deadline = Instant::now() + SETTLE;
    loop {
        let info = migration(agent, module).await;
        if done(&info) {
            return info;
        }
        assert!(Instant::now() < deadline, "{info:?}\n{}", agent.log());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A stored key, run as the app's `runAction` does: whether it
/// succeeded, its message, or the agent's refusal.
async fn run(agent: &Agent, action: ActionInstanceId) -> (bool, String) {
    match agent.request(Request::RunAction { action }).await {
        Ok(Response::ActionResult { result }) => (
            result.status == deckpilot_core::ActionStatus::Succeeded,
            result.message,
        ),
        Ok(other) => panic!("unexpected {other:?}"),
        Err(error) => (false, error.to_string()),
    }
}

/// Waits for the consent, checks `key` is unavailable meanwhile, gives
/// the consent and waits for the cut-over.
async fn consent(agent: &Agent, module: &str, high: &str, key: ActionInstanceId) {
    let info = wait_for(agent, module, |m| {
        m.phase == MigrationPhase::AwaitingConsent
    })
    .await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    let pending: Vec<String> = info
        .consent
        .iter()
        .filter(|item| item.decision == ConsentDecision::Pending)
        .map(|item| item.key.clone())
        .collect();
    assert!(pending.iter().any(|k| k == high), "{pending:?}");
    let (ok, message) = run(agent, key).await;
    assert!(
        !ok && message.contains("waits for your consent"),
        "{message}"
    );
    match agent
        .ok(Request::DecideModuleMigration {
            module: module.into(),
            approve: pending,
            decline: Vec::new(),
        })
        .await
    {
        Response::ModuleMigration { .. } => {}
        other => panic!("unexpected {other:?}"),
    }
    wait_for(agent, module, |m| {
        m.phase == MigrationPhase::CutOver && m.route == MigrationRoute::Plugin
    })
    .await;
}

/// `key` once it runs (the routing follows the cut-over shortly).
async fn ran(agent: &Agent, key: ActionInstanceId) -> String {
    let deadline = Instant::now() + SETTLE;
    loop {
        let (ok, message) = run(agent, key).await;
        if ok {
            return message;
        }
        assert!(
            Instant::now() < deadline,
            "the key did not run: {message}\n{}",
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_workspace_user_moves_to_the_plugin_after_consent() {
    let dir = support::TempDir::new("plugins-cutover-workspace-fixture");
    let repo = dir.path().join("src/shop");
    std::fs::create_dir_all(&repo).unwrap();
    let file = repo.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let mut machine = MachineConfig::new(Os::current());
    for (key, kind, value) in [
        (
            "shop.main",
            ResourceKind::Other,
            file.canonicalize().unwrap().display().to_string(),
        ),
        (
            "tools.code",
            ResourceKind::Application,
            "/usr/local/bin/code".into(),
        ),
    ] {
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new(key).unwrap(),
            kind,
            value,
        });
    }
    let fixed = |key: &str| {
        ParameterValue::Resource(ResourceRef::Fixed {
            key: ResourceKey::new(key).unwrap(),
        })
    };
    let mut profile = Profile::new("Work");
    let pages = legacy(
        &mut profile,
        "workspace.open-urls",
        &[(
            "urls",
            ParameterValue::Text("https://status.example.com/shop".into()),
        )],
    );
    let far = legacy(
        &mut profile,
        "workspace.open-in-editor",
        &[
            ("application", fixed("tools.code")),
            ("location", fixed("shop.main")),
            ("editor", ParameterValue::Choice("vscode".into())),
            ("line", ParameterValue::Integer(5_000_000)),
        ],
    );
    let env = Env::new(
        "plugins-cutover-workspace",
        WORKSPACE,
        BundleClass::Migration,
        Some((&profile, None)),
        Some(&machine),
    );
    let agent = env.start().await;
    consent(&agent, "workspace", "resources:open", pages).await;
    let info = migration(&agent, "workspace").await;
    assert!(
        info.reason
            .as_deref()
            .is_some_and(|r| r.contains("sets `line` above 1000000")),
        "{info:?}"
    );
    assert_eq!(ran(&agent, pages).await, "opened 1 pages");
    let opened = std::fs::read_to_string(env.opened_file()).unwrap_or_default()
        + &std::fs::read_to_string(env.launch_file()).unwrap_or_default();
    assert!(
        opened.contains("https://status.example.com/shop"),
        "{opened}"
    );
    // Above the review's bound: refused with the reason (the plugin's
    // range), never clamped; nothing opened.
    let (ok, message) = run(&agent, far).await;
    assert!(!ok, "{message}");
    assert!(
        message.contains("`line`") && message.contains("out of range"),
        "{message}"
    );
    assert!(
        !std::fs::read_to_string(env.launch_file())
            .unwrap_or_default()
            .contains("main.rs"),
        "nothing was launched for it"
    );
    agent.stop().await;
}
