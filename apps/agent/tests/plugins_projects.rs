//! `deckpilot.projects` end to end through the real agent, the real
//! sandboxed host and plugin API 1.9's project context (feature
//! `plugins`; see `support/first_party.rs`):
//!
//! * `the_plugin_switches_and_reads_the_active_project`: seeded waiting
//!   for consent (`projectContext:select` is high tier), its views listed;
//!   once enabled every action runs against the agent's own
//!   `ActiveProjectContext`. `cycle`, `next`, `previous` and `select`
//!   switch the project as the app does, and `projectChanged` names the
//!   plugin; `wrapAround` is read when a cycle runs; the reading actions
//!   and the resource actions report what the core holds;
//! * `switching_is_refused_in_a_run_the_user_did_not_start`: a
//!   scheduled workflow's `cycle` is refused and nothing switches;
//! * `host_variables_and_views_exist_only_while_the_plugin_is_enabled`:
//!   `current.id`, `.key` and `.name` are the host's values, the project in
//!   the context, while the plugin is enabled and not before or after;
//!   the `projects` view is listed with the plugin's state, which the app
//!   shows only while enabled.
//!
//! The plugin is pinned as a `default`-class plugin: `bundled.json`
//! releases it as `migration` (its routing from `projects.*` is WP-C1).
#![cfg(feature = "plugins")]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::time::Duration;

use deckpilot_core::variables::{ReferenceStatus, VariableValue};
use deckpilot_core::{
    MachineConfig, MachineResourceBinding, Os, ParameterValue, Profile, Project, ProjectId,
    ProjectKey, ResourceKey, ResourceKind,
};
use deckpilot_ipc::plugins::{PermissionStatus, PermissionTier, PluginState};
use deckpilot_ipc::{AgentEvent, Request, Response};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Agent, Env, FirstParty};
use serde_json::json;

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.projects",
    dir: "projects",
    krate: "deckpilot-projects",
    released: "migration",
};
/// The host's switch rate (X6): a second switch within 500 ms is
/// throttled.
const SWITCH_GAP: Duration = Duration::from_millis(600);
const VARIABLES: [&str; 3] = [
    "plugins.deckpilot.projects.current.id",
    "plugins.deckpilot.projects.current.key",
    "plugins.deckpilot.projects.current.name",
];

struct Fixture {
    profile: Profile,
    ids: Vec<ProjectId>,
    machine: MachineConfig,
    repo: std::path::PathBuf,
    select_cms: deckpilot_core::ActionInstanceId,
    repository_of_cmr: deckpilot_core::ActionInstanceId,
    repository: deckpilot_core::ActionInstanceId,
    documentation: deckpilot_core::ActionInstanceId,
    ci: deckpilot_core::ActionInstanceId,
    scheduled: deckpilot_core::WorkflowId,
}

fn fixture(dir: &std::path::Path) -> Fixture {
    let repo = dir.join("src/shop");
    std::fs::create_dir_all(&repo).unwrap();
    let repo = repo.canonicalize().unwrap();
    let cmr = dir.join("src/cmr");
    std::fs::create_dir_all(&cmr).unwrap();
    let mut machine = MachineConfig::new(Os::current());
    for (key, kind, value) in [
        (
            "shop.repo",
            ResourceKind::Directory,
            repo.display().to_string(),
        ),
        (
            "cmr.repo",
            ResourceKind::Directory,
            cmr.canonicalize().unwrap().display().to_string(),
        ),
        (
            "shop.docs",
            ResourceKind::Url,
            "https://docs.example.com/shop".into(),
        ),
    ] {
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new(key).unwrap(),
            kind,
            value,
        });
    }
    let mut profile = Profile::new("Work");
    for (key, name) in [("SHOP", "Shop"), ("CMR", "Customer"), ("CMS", "Content")] {
        profile
            .projects
            .push(Project::new(ProjectKey::new(key).unwrap(), name));
    }
    let res = &mut profile.projects[0].resources;
    res.insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
    res.insert("docs".into(), ResourceKey::new("shop.docs").unwrap());
    profile.projects[1]
        .resources
        .insert("repository".into(), ResourceKey::new("cmr.repo").unwrap());
    let ids: Vec<ProjectId> = profile.projects.iter().map(|p| p.id).collect();
    let select_cms = first_party::instance(
        &mut profile,
        PLUGIN.id,
        "select",
        &[("projectId", ParameterValue::Project(ids[2]))],
    );
    let repository_of_cmr = first_party::instance(
        &mut profile,
        PLUGIN.id,
        "repository",
        &[("projectId", ParameterValue::Project(ids[1]))],
    );
    let repository = first_party::instance(&mut profile, PLUGIN.id, "repository", &[]);
    let documentation = first_party::instance(&mut profile, PLUGIN.id, "documentation", &[]);
    let ci = first_party::instance(
        &mut profile,
        PLUGIN.id,
        "resolve-resource",
        &[("resource", ParameterValue::Text("ci".into()))],
    );
    let cycle = first_party::instance(&mut profile, PLUGIN.id, "cycle", &[]);
    let scheduled = first_party::scheduled_workflow(&mut profile, "Every minute", cycle);
    Fixture {
        profile,
        ids,
        machine,
        repo,
        select_cms,
        repository_of_cmr,
        repository,
        documentation,
        ci,
        scheduled,
    }
}

fn env(name: &str, f: &Fixture) -> Env {
    Env::new(
        name,
        PLUGIN,
        BundleClass::Default,
        Some((&f.profile, Some(f.ids[0]))),
        Some(&f.machine),
    )
}

async fn active(agent: &Agent) -> Option<ProjectId> {
    match agent.ok(Request::GetActiveProject).await {
        Response::ActiveProject { project } => project.map(|p| p.id),
        other => panic!("unexpected {other:?}"),
    }
}

/// The next `projectChanged`: the project and the plugin that made the
/// change.
async fn project_changed(
    events: &mut deckpilot_ipc::AgentClient,
) -> (Option<ProjectId>, Option<String>) {
    let deadline = tokio::time::Instant::now() + first_party::SETTLE;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("projectChanged in time")
            .unwrap();
        if let AgentEvent::ProjectChanged {
            project, plugin, ..
        } = event
        {
            return (project, plugin);
        }
    }
}

fn string(value: &str) -> ReferenceStatus {
    ReferenceStatus::Resolved {
        value: VariableValue::String(value.into()),
        source: deckpilot_core::variables::ValueSource::System,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one path, step by step
async fn the_plugin_switches_and_reads_the_active_project() {
    let dir = support::TempDir::new("plugins-projects-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-projects", &f);
    let agent = env.start().await;

    // ---- seeded, waiting for consent to switch projects.
    let summary = agent.seeded().await;
    assert_ne!(summary.state, PluginState::Enabled, "{}", agent.log());
    assert_eq!(summary.actions.len(), 9);
    let details = agent.details().await;
    let select = details
        .permissions
        .iter()
        .find(|p| p.key == "projectContext:select")
        .unwrap_or_else(|| panic!("{:?}", details.permissions));
    assert_eq!(select.tier, PermissionTier::High);
    assert_ne!(select.status, PermissionStatus::Granted);
    for permission in &details.permissions {
        let text = permission.justification.as_deref().unwrap_or_default();
        assert!(
            !text.is_empty() && !text.starts_with("t:"),
            "{permission:?}"
        );
    }
    agent.enable().await;

    // ---- cycle from the app's "Try it": the agent's active project moves
    // on, `projectChanged` names the plugin.
    let mut events = agent.events().await;
    let cycled = agent.invoke("cycle", &json!({"direction": "next"})).await;
    assert!(cycled.ok, "{cycled:?}\n{}", agent.log());
    assert_eq!(cycled.message, "switched to CMR");
    assert_eq!(cycled.data["projectKey"], "CMR");
    assert_eq!(cycled.data["projectId"], f.ids[1].to_string());
    assert_eq!(cycled.data["projectName"], "Customer");
    assert_eq!(
        project_changed(&mut events).await,
        (Some(f.ids[1]), Some(PLUGIN.id.to_owned()))
    );
    assert_eq!(active(&agent).await, Some(f.ids[1]));

    // ---- the deprecated next and previous are cycle.
    tokio::time::sleep(SWITCH_GAP).await;
    let next = agent.invoke("next", &json!({})).await;
    assert_eq!(next.data["projectKey"], "CMS", "{next:?}");
    assert_eq!(
        project_changed(&mut events).await,
        (Some(f.ids[2]), Some(PLUGIN.id.to_owned()))
    );
    tokio::time::sleep(SWITCH_GAP).await;
    let wrapped = agent.invoke("next", &json!({})).await;
    assert_eq!(wrapped.data["projectKey"], "SHOP", "wraps: {wrapped:?}");
    project_changed(&mut events).await;
    tokio::time::sleep(SWITCH_GAP).await;
    let previous = agent.invoke("previous", &json!({})).await;
    assert_eq!(previous.data["projectKey"], "CMS", "{previous:?}");
    project_changed(&mut events).await;

    // ---- wrapAround off: the ends are reported, nothing switches.
    agent
        .ok(Request::SetPluginSettings {
            plugin: PLUGIN.id.into(),
            values: serde_json::from_value(json!({"wrapAround": false})).unwrap(),
        })
        .await;
    tokio::time::sleep(SWITCH_GAP).await;
    let at_end = agent.invoke("cycle", &json!({"direction": "next"})).await;
    assert_eq!(
        at_end.code.as_deref(),
        Some("at-last-project"),
        "{at_end:?}"
    );
    assert_eq!(active(&agent).await, Some(f.ids[2]));

    // ---- select through the engine, with the project parameter.
    tokio::time::sleep(SWITCH_GAP).await;
    agent.ok(Request::SelectProject { project: f.ids[0] }).await;
    project_changed(&mut events).await;
    let selected = agent.run_action(f.select_cms).await;
    assert!(selected.ok, "{selected:?}");
    assert_eq!(selected.message, "selected CMS");
    assert_eq!(
        project_changed(&mut events).await,
        (Some(f.ids[2]), Some(PLUGIN.id.to_owned()))
    );
    tokio::time::sleep(SWITCH_GAP).await;
    let unknown = agent
        .invoke(
            "select",
            &json!({"projectId": ProjectId::new().to_string()}),
        )
        .await;
    assert_eq!(
        unknown.code.as_deref(),
        Some("project-not-in-profile"),
        "{unknown:?}"
    );
    let reported = agent.invoke("select", &json!({})).await;
    assert_eq!(reported.message, "CMS is active");

    // ---- the reading actions.
    for action in ["current", "active"] {
        let current = agent.invoke(action, &json!({})).await;
        assert!(current.ok, "{current:?}");
        assert_eq!(current.message, "CMS (Content)");
        assert_eq!(current.data["projectKey"], "CMS");
    }
    tokio::time::sleep(SWITCH_GAP).await;
    agent.ok(Request::SelectProject { project: f.ids[0] }).await;
    // The resource actions run through the engine, which resolves the
    // invocation's resources (the app's "Try it" has none).
    let repository = agent.run_action(f.repository).await;
    assert!(repository.ok, "{repository:?}\n{}", agent.log());
    assert_eq!(
        repository.message,
        format!("SHOP repository: {}", f.repo.display())
    );
    assert_eq!(repository.data["resourceKey"], "shop.repo");
    assert_eq!(repository.data["kind"], "directory");
    let docs = agent.run_action(f.documentation).await;
    assert!(docs.ok, "{docs:?}");
    assert_eq!(docs.data["kind"], "url");
    assert_eq!(docs.data["resourceKey"], "shop.docs");
    let named = agent.run_action(f.repository_of_cmr).await;
    assert!(named.ok, "{named:?}");
    assert_eq!(named.data["projectKey"], "CMR");
    assert_eq!(named.data["resourceKey"], "cmr.repo");
    let missing = agent.run_action(f.ci).await;
    assert_eq!(
        missing.code.as_deref(),
        Some("resource-role-missing"),
        "{missing:?}"
    );
    assert_eq!(
        active(&agent).await,
        Some(f.ids[0]),
        "reading never switches"
    );
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_is_refused_in_a_run_the_user_did_not_start() {
    let dir = support::TempDir::new("plugins-projects-schedule-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-projects-schedule", &f);
    let agent = env.start().await;
    agent.seeded().await;
    agent.enable().await;
    // The scheduler first sees the workflow on one of its 5 s ticks with
    // the clock as it is; then the clock moves past its first due time.
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    std::fs::write(env.clock_offset(), "61").unwrap();
    let record = agent.first_run(f.scheduled).await;
    let run = first_party::node(&record, "run");
    let error = run.error.as_ref().expect("the node failed");
    assert_eq!(
        error.action_code.as_deref(),
        Some("not-permitted"),
        "{record:#?}"
    );
    assert!(error.message.contains("the user started"), "{record:#?}");
    assert_eq!(active(&agent).await, Some(f.ids[0]));
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_variables_and_views_exist_only_while_the_plugin_is_enabled() {
    let dir = support::TempDir::new("plugins-projects-variables-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-projects-variables", &f);
    let agent = env.start().await;
    let profile = f.profile.id;

    // ---- seeded, not enabled: no values; the view is listed with the
    // plugin's state (the app shows plugin views of enabled plugins only).
    let summary = agent.seeded().await;
    assert_ne!(summary.state, PluginState::Enabled);
    let views: Vec<(String, String)> = summary
        .navigation
        .iter()
        .map(|e| (e.view.kind.clone(), e.label.clone()))
        .collect();
    assert_eq!(views, [("projects".to_owned(), "Projects".to_owned())]);
    let before = agent.preview(profile, &VARIABLES).await;
    assert!(
        before
            .iter()
            .all(|s| !matches!(s, ReferenceStatus::Resolved { .. })),
        "{before:?}"
    );

    // ---- enabled: the host's values, the active project.
    agent.enable().await;
    let id = f.ids[0].to_string();
    assert_eq!(
        agent.preview(profile, &VARIABLES).await,
        [string(&id), string("SHOP"), string("Shop")]
    );
    agent.ok(Request::SelectProject { project: f.ids[1] }).await;
    assert_eq!(
        agent.preview(profile, &VARIABLES[1..2]).await,
        [string("CMR")]
    );
    let summary = agent.wait_state(PluginState::Enabled).await;
    assert_eq!(summary.navigation.len(), 1);

    // ---- disabled: gone, with the view's plugin.
    agent.disable().await;
    let after = agent.preview(profile, &VARIABLES).await;
    assert!(
        after
            .iter()
            .all(|s| !matches!(s, ReferenceStatus::Resolved { .. })),
        "{after:?}"
    );
    // ---- enabled again: back.
    agent.enable().await;
    assert_eq!(
        agent.preview(profile, &VARIABLES[1..2]).await,
        [string("CMR")]
    );
    agent.stop().await;
}
