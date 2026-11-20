//! `deckpilot.workspace` end to end through the real agent, the real
//! sandboxed host and plugin API 1.9's real host operations (feature
//! `plugins`): its `resource` parameters and the active project's roles
//! become handles of the invocation (`resource.resolve`), `resource.open`
//! plans the editor's arguments in the core's launcher, `url.openParam`
//! opens the literal page the user typed, and every program the agent
//! would start goes to a file (`DECKPILOT_TEST_PLUGIN_LAUNCH`, feature
//! `test-hooks`): no editor, no browser. See `support/first_party.rs`.
//!
//! * `every_action_opens_bound_resources_through_the_launcher`: the
//!   permissions ask for consent, then each of the three actions runs as
//!   the user started it, and opens exactly the bindings it names;
//! * `nothing_opens_in_a_run_the_user_did_not_start`: the same actions
//!   from a scheduled workflow are refused and nothing is launched.
//!
//! The plugin is pinned as a `default`-class plugin: `bundled.json`
//! releases it as `migration`, which only the migration engine installs
//! (its routing from the legacy `workspace.*` ids is WP-C1).
#![cfg(feature = "plugins")]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use deckpilot_core::{
    MachineConfig, MachineResourceBinding, Os, ParameterValue, Profile, Project, ProjectKey,
    ResourceKey, ResourceKind, ResourceRef,
};
use deckpilot_ipc::plugins::{PermissionStatus, PermissionTier, PluginState};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Env, FirstParty};
use serde_json::json;

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.workspace",
    dir: "workspace",
    krate: "deckpilot-workspace",
    released: "migration",
};
const EDITOR: &str = "/usr/local/bin/code";

fn fixed(key: &str) -> ParameterValue {
    ParameterValue::Resource(ResourceRef::Fixed {
        key: ResourceKey::new(key).unwrap(),
    })
}

fn bind(machine: &mut MachineConfig, key: &str, kind: ResourceKind, value: &str) {
    machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new(key).unwrap(),
        kind,
        value: value.into(),
    });
}

/// A checkout of the shop on this computer, with one file.
fn checkout(env_dir: &Path) -> (PathBuf, PathBuf) {
    let repo = env_dir.join("src/shop");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let file = repo.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    (repo.canonicalize().unwrap(), file.canonicalize().unwrap())
}

struct Fixture {
    profile: Profile,
    project: deckpilot_core::ProjectId,
    machine: MachineConfig,
    file: PathBuf,
    repo: PathBuf,
    at_line: deckpilot_core::ActionInstanceId,
    repository: deckpilot_core::ActionInstanceId,
    pages: deckpilot_core::ActionInstanceId,
    docs: deckpilot_core::ActionInstanceId,
    whole: deckpilot_core::ActionInstanceId,
    scheduled: deckpilot_core::WorkflowId,
}

fn fixture(dir: &Path) -> Fixture {
    let (repo, file) = checkout(dir);
    let mut machine = MachineConfig::new(Os::current());
    bind(
        &mut machine,
        "shop.repo",
        ResourceKind::Directory,
        &repo.display().to_string(),
    );
    bind(
        &mut machine,
        "shop.main",
        ResourceKind::Other,
        &file.display().to_string(),
    );
    bind(
        &mut machine,
        "tools.code",
        ResourceKind::Application,
        EDITOR,
    );
    bind(
        &mut machine,
        "shop.docs",
        ResourceKind::Url,
        "https://docs.example.com/shop",
    );
    bind(
        &mut machine,
        "shop.ci",
        ResourceKind::Url,
        "https://ci.example.com/shop?branch=main",
    );

    let mut profile = Profile::new("Work");
    let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    for (role, key) in [
        ("repository", "shop.repo"),
        ("editor", "tools.code"),
        ("docs", "shop.docs"),
        ("ci", "shop.ci"),
    ] {
        shop.resources
            .insert(role.into(), ResourceKey::new(key).unwrap());
    }
    let project = shop.id;
    profile.projects.push(shop);
    let id = PLUGIN.id;
    let at_line = first_party::instance(
        &mut profile,
        id,
        "open-in-editor",
        &[
            ("application", fixed("tools.code")),
            ("location", fixed("shop.main")),
            ("editor", ParameterValue::Choice("vscode".into())),
            ("line", ParameterValue::Integer(12)),
            ("column", ParameterValue::Integer(3)),
        ],
    );
    let repository = first_party::instance(
        &mut profile,
        id,
        "open-in-editor",
        &[
            ("application", fixed("tools.code")),
            ("editor", ParameterValue::Choice("zed".into())),
        ],
    );
    let pages = first_party::instance(
        &mut profile,
        id,
        "open-urls",
        &[(
            "urls",
            ParameterValue::Text("https://status.example.com/shop".into()),
        )],
    );
    let docs = first_party::instance(
        &mut profile,
        id,
        "open-project",
        &[
            ("repositoryRole", ParameterValue::Text(String::new())),
            ("docsRole", ParameterValue::Text("docs".into())),
        ],
    );
    let whole = first_party::instance(
        &mut profile,
        id,
        "open-project",
        &[
            ("editor", ParameterValue::Choice("jetbrains".into())),
            ("docsRole", ParameterValue::Text(String::new())),
            ("urlRoles", ParameterValue::Text("missing".into())),
        ],
    );
    let scheduled = first_party::scheduled_workflow(&mut profile, "Every minute", pages);
    Fixture {
        profile,
        project,
        machine,
        file,
        repo,
        at_line,
        repository,
        pages,
        docs,
        whole,
        scheduled,
    }
}

/// The page a launch plan opens in the default browser (`open <url>` on
/// macOS, `xdg-open <url>` on Linux, `rundll32 url.dll,… <url>` on
/// Windows): its last argument.
fn page(plan: &(String, Vec<String>)) -> &str {
    plan.1.last().map_or("", String::as_str)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one path, step by step
async fn every_action_opens_bound_resources_through_the_launcher() {
    let dir = support::TempDir::new("plugins-workspace-fixture");
    let f = fixture(dir.path());
    let env = Env::new(
        "plugins-workspace",
        PLUGIN,
        BundleClass::Default,
        Some((&f.profile, Some(f.project))),
        Some(&f.machine),
    );
    let agent = env.start().await;

    // ---- seeded, waiting for consent: `resources.open` is high tier.
    let summary = agent.seeded().await;
    assert_ne!(summary.state, PluginState::Enabled, "{}", agent.log());
    assert_eq!(summary.actions.len(), 3);
    let details = agent.details().await;
    let tiers: Vec<(String, PermissionTier, PermissionStatus)> = details
        .permissions
        .iter()
        .map(|p| (p.key.clone(), p.tier, p.status))
        .collect();
    assert!(
        tiers
            .iter()
            .any(|(key, tier, status)| key == "resources:open"
                && *tier == PermissionTier::High
                && *status != PermissionStatus::Granted),
        "{tiers:?}"
    );
    assert!(
        tiers
            .iter()
            .any(|(key, tier, _)| key == "openUrl:from-params" && *tier == PermissionTier::Medium),
        "{tiers:?}"
    );
    // The consent texts are translated, never the `t:` keys.
    for permission in &details.permissions {
        let text = permission.justification.as_deref().unwrap_or_default();
        assert!(
            !text.is_empty() && !text.starts_with("t:"),
            "{permission:?}"
        );
    }
    assert!(!summary.name.starts_with("t:"), "{}", summary.name);
    agent.enable().await;

    // ---- open-in-editor: a bound file at a line, in the bound editor,
    // with the vscode plan: the editor's own command-line tool, `--goto`.
    let opened = agent.run_action(f.at_line).await;
    assert!(opened.ok, "{opened:?}\n{}", agent.log());
    assert_eq!(opened.message, "opened shop.main in tools.code");
    assert_eq!(opened.data["path"], "shop.main");
    assert_eq!(opened.data["application"], "tools.code");
    assert_eq!(opened.data["located"], "true");
    let launched = env.launched();
    assert_eq!(launched.len(), 1, "{launched:?}");
    assert_eq!(launched[0].0, EDITOR);
    assert_eq!(
        launched[0].1,
        ["--goto", &format!("{}:12:3", f.file.display())]
    );
    // Never a path in what the plugin answered.
    assert!(
        !opened
            .data
            .values()
            .any(|v| v.contains(&*f.repo.to_string_lossy())),
        "{opened:?}"
    );

    // ---- open-in-editor without a folder: the active project's
    // repository, no line.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let opened = agent.run_action(f.repository).await;
    assert!(opened.ok, "{opened:?}");
    assert_eq!(opened.data["path"], "shop.repo");
    assert_eq!(opened.data["located"], "false");
    let launched = env.launched();
    assert_eq!(launched.len(), 2, "{launched:?}");
    assert!(
        launched[1].1.iter().any(|a| Path::new(a) == f.repo),
        "{launched:?}"
    );

    // ---- open-urls: the literal page the user typed, in the browser.
    let opened = agent.run_action(f.pages).await;
    assert!(opened.ok, "{opened:?}");
    assert_eq!(opened.message, "opened 1 pages");
    assert_eq!(opened.data["opened"], "1");
    assert_eq!(opened.data["urls"], "https://status.example.com/shop");
    let launched = env.launched();
    assert_eq!(page(&launched[2]), "https://status.example.com/shop");

    // A page that is not http(s) is refused by the plugin before any
    // open; the "Try it" run cannot name a resource, so it is refused too
    // when it needs one.
    let refused = agent
        .invoke("open-urls", &json!({"urls": "file:///etc/passwd"}))
        .await;
    assert_eq!(refused.code.as_deref(), Some("invalid-url"), "{refused:?}");
    let refused = agent.invoke("open-in-editor", &json!({})).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("invalid-parameters"),
        "{refused:?}"
    );
    assert_eq!(env.launched().len(), 3);

    // ---- open-project: the docs role, a bound address, in the browser.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let opened = agent.run_action(f.docs).await;
    assert!(opened.ok, "{opened:?}");
    assert_eq!(opened.message, "opened docs of SHOP");
    assert_eq!(opened.data["projectKey"], "SHOP");
    assert_eq!(opened.data["openedRoles"], "docs");
    assert_eq!(opened.data["missingCount"], "0");
    let launched = env.launched();
    assert_eq!(page(&launched[3]), "https://docs.example.com/shop");

    // ---- open-project: the repository in the project's editor (the
    // jetbrains plan opens a folder without a line), a role the project
    // does not define reported.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let opened = agent.run_action(f.whole).await;
    assert!(opened.ok, "{opened:?}");
    assert_eq!(opened.data["openedRoles"], "repository");
    assert_eq!(opened.data["missingCount"], "1");
    assert!(
        opened.data["missing"].starts_with("missing: "),
        "{opened:?}"
    );
    let launched = env.launched();
    assert_eq!(launched.len(), 5, "{launched:?}");
    assert!(
        launched[4].1.iter().any(|a| Path::new(a) == f.repo),
        "{launched:?}"
    );
    // Nothing went through `url.open`.
    env.assert_no_page_opened();
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_opens_in_a_run_the_user_did_not_start() {
    let dir = support::TempDir::new("plugins-workspace-schedule-fixture");
    let f = fixture(dir.path());
    let env = Env::new(
        "plugins-workspace-schedule",
        PLUGIN,
        BundleClass::Default,
        Some((&f.profile, Some(f.project))),
        Some(&f.machine),
    );
    let agent = env.start().await;
    agent.seeded().await;
    agent.enable().await;
    // The agent's clock moves past the workflow's first due time: its
    // schedule runs `open-urls`, which may not open a page.
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
    assert_eq!(env.launched(), Vec::<(String, Vec<String>)>::new());
    env.assert_no_page_opened();
    agent.stop().await;
}
