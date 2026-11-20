//! The Projects cut-over of the extraction (WP-C1,
//! `docs/marketplace/module-migration.md` §11 D4, §7 X13) end to end,
//! through the real agent, the real sandboxed host and
//! `plugins/first-party/projects` built to wasm and bundled as the release
//! bundles it (class `migration`), on a configuration that uses the
//! built-in: a profile with projects and stored `projects.*` keys. The
//! migration waits for `projectContext:select` with the legacy keys
//! unavailable; after the user's consent the same stored
//! `projects.current` and `projects.select` keys run through the plugin,
//! and `select` switches the active project as the built-in did. The
//! Workspace cut-over is `plugins_cutover_workspace.rs` (one first-party
//! plugin per test binary, `support/first_party.rs`).
#![cfg(feature = "plugins")]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActionTypeId, ParameterValue, Profile, Project, ProjectId,
    ProjectKey,
};
use deckpilot_ipc::plugins::{
    ConsentDecision, MigrationPhase, MigrationRoute, ModuleMigrationInfo,
};
use deckpilot_ipc::{Request, Response};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Agent, Env, FirstParty, SETTLE};

const PROJECTS: FirstParty = FirstParty {
    id: "deckpilot.projects",
    dir: "projects",
    krate: "deckpilot-projects",
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

/// Waits until `project` is the active project (the switch persists and
/// the configuration reloads after the key ran).
async fn wait_active(agent: &Agent, project: ProjectId) {
    let deadline = Instant::now() + SETTLE;
    loop {
        let seen = match agent.request(Request::GetActiveProject).await {
            Ok(Response::ActiveProject { project }) => project.map(|p| p.id),
            _ => None,
        };
        if seen == Some(project) {
            return;
        }
        assert!(Instant::now() < deadline, "{seen:?}\n{}", agent.log());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_projects_user_moves_to_the_plugin_after_consent() {
    let mut profile = Profile::new("Work");
    for (key, name) in [("SHOP", "Shop"), ("CMS", "Content")] {
        profile
            .projects
            .push(Project::new(ProjectKey::new(key).unwrap(), name));
    }
    let ids: Vec<ProjectId> = profile.projects.iter().map(|p| p.id).collect();
    let current = legacy(&mut profile, "projects.current", &[]);
    let select = legacy(
        &mut profile,
        "projects.select",
        &[("project_id", ParameterValue::Project(ids[1]))],
    );
    let env = Env::new(
        "plugins-cutover-projects",
        PROJECTS,
        BundleClass::Migration,
        Some((&profile, Some(ids[0]))),
        None,
    );
    let agent = env.start().await;
    consent(&agent, "projects", "projectContext:select", current).await;
    assert_eq!(ran(&agent, current).await, "SHOP (Shop)");
    let message = ran(&agent, select).await;
    assert_eq!(message, "selected CMS");
    wait_active(&agent, ids[1]).await;
    agent.stop().await;
}
