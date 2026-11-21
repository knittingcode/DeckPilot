//! The Git cut-over of the extraction (WP-C1, `docs/marketplace/
//! module-migration.md` §11 D4, §7 X13) end to end, through the real
//! agent, the real sandboxed host and `plugins/first-party/git` built to
//! wasm and bundled as the release bundles it (class `migration`, pinned
//! with `DECKPILOT_TEST_BUNDLED_PINS`), on a configuration that uses the
//! built-in `git` module: its `modules` row (with the executable) and a
//! stored `git.status` key. Every git is a **fake `git`** that logs its
//! arguments (Unix only).
//!
//! * the migration installs the plugin, backs up `deckpilot.db` once
//!   (`backups/deckpilot-pre-plugin-extraction-*.db`, encrypted) and waits
//!   for the high-tier consent: the stored key is unavailable with the
//!   reason, nothing runs, and the profile is byte for byte as stored;
//! * the user's consent (`decideModuleMigration`) cuts it over: the same
//!   stored key runs through the plugin and the host's `tool.run` (its
//!   work tree check, which the built-in never runs, proves which ran),
//!   with the built-in's executable setting held by the host;
//! * a restart changes nothing and writes no second backup.
#![cfg(all(feature = "plugins", unix))]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActionTypeId, MachineConfig, MachineResourceBinding,
    ModuleId, ModuleSettings, Os, ParameterValue, Profile, Project, ProjectKey, ResourceKey,
    ResourceKind,
};
use deckpilot_ipc::plugins::{
    ConsentDecision, MigrationPhase, MigrationRoute, ModuleMigrationInfo,
};
use deckpilot_ipc::{Request, Response};
use deckpilot_plugin_bundled::BundleClass;
use deckpilot_storage::ConfigStore;
use first_party::{Agent, Env, FirstParty, SETTLE};

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.git",
    dir: "git",
    krate: "deckpilot-git",
    released: "migration",
};
const OID: &str = "1234567890abcdef1234567890abcdef12345678";

/// A fake `git` that logs its arguments and answers `status` and the
/// host's work tree check like git in a clean repository on `main`.
fn fake_git(bin: &Path, log: &Path) -> PathBuf {
    std::fs::create_dir_all(bin).unwrap();
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$@" '@@' >> '{log}'
while [ $# -gt 0 ]; do
  case "$1" in
    -C|-c) shift 2 ;;
    --no-pager|--no-optional-locks) shift ;;
    *) break ;;
  esac
done
here=$(pwd -P)
case "$*" in
  "rev-parse --show-toplevel") echo "$here" ;;
  "rev-parse --show-toplevel --absolute-git-dir") echo "$here"; echo "$here/.git" ;;
  status*) printf '%s\000' '# branch.oid {OID}' '# branch.head main' ;;
  *) echo "fake git: unsupported $*" >&2; exit 1 ;;
esac
"#,
        log = log.display()
    );
    let path = bin.join("git");
    std::fs::write(&path, script).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// Each logged run's arguments.
fn runs(log: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut out = Vec::new();
    let mut args = Vec::new();
    for line in text.lines() {
        if line == "@@" {
            out.push(std::mem::take(&mut args));
        } else {
            args.push(line.to_owned());
        }
    }
    out
}

/// The backups the agent wrote before the extraction.
fn backups(env: &Env) -> Vec<String> {
    let dir = env.paths.data_dir.join("backups");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("deckpilot-pre-plugin-extraction-"))
        .collect();
    names.sort();
    names
}

async fn migration(agent: &Agent) -> ModuleMigrationInfo {
    match agent.ok(Request::ListModuleMigrations).await {
        Response::ModuleMigrations { migrations } => migrations
            .into_iter()
            .find(|m| m.module == "git")
            .unwrap_or_else(|| panic!("no migration of git\n{}", agent.log())),
        other => panic!("unexpected {other:?}"),
    }
}

async fn wait_for(
    agent: &Agent,
    what: &str,
    done: impl Fn(&ModuleMigrationInfo) -> bool,
) -> ModuleMigrationInfo {
    let deadline = Instant::now() + SETTLE;
    loop {
        let info = migration(agent).await;
        if done(&info) {
            return info;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: {info:?}\n{}",
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The stored key, run as the app's `runAction` does: its message, or the
/// agent's refusal.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one path, step by step
async fn a_used_git_module_waits_for_consent_then_runs_through_the_plugin() {
    let dir = support::TempDir::new("plugins-cutover-git-fixture");
    let repo = dir.path().join("src/shop");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let repo = repo.canonicalize().unwrap();
    let log = dir.path().join("git.log");
    let git = fake_git(&dir.path().join("bin"), &log);

    let mut machine = MachineConfig::new(Os::current());
    machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new("shop.repo").unwrap(),
        kind: ResourceKind::Directory,
        value: repo.display().to_string(),
    });
    let mut profile = Profile::new("Work");
    let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    shop.resources
        .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
    let project = shop.id;
    profile.projects.push(shop);
    // The legacy key, as an alpha-era app stored it.
    let status = ActionInstanceId::new();
    profile.actions.push(deckpilot_core::ActionInstance {
        id: status,
        name: "Status".into(),
        action_type: ActionTypeId::new("git.status").unwrap(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    });
    let env = Env::new(
        "plugins-cutover-git",
        PLUGIN,
        BundleClass::Migration,
        Some((&profile, Some(project))),
        Some(&machine),
    )
    .with_projects(BundleClass::Migration);
    // The built-in `git` saved with the user's executable.
    {
        let store = support::open_store(&env.paths, &env.key);
        store
            .set_module_state(
                &ModuleId::new("git").unwrap(),
                &ModuleSettings {
                    enabled: true,
                    config: ActionParameters::new().with(
                        "gitExecutable",
                        ParameterValue::Text(git.display().to_string()),
                    ),
                },
            )
            .unwrap();
    }
    let stored = || {
        let store = support::open_store(&env.paths, &env.key);
        serde_json::to_string(&store.load_profile(profile.id).unwrap().unwrap()).unwrap()
    };
    let before = stored();

    // ---- waiting for consent: unavailable, nothing runs, one backup.
    let agent = env.start().await;
    let info = wait_for(&agent, "awaiting consent", |m| {
        m.phase == MigrationPhase::AwaitingConsent
    })
    .await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    assert!(
        info.route_reason
            .as_deref()
            .is_some_and(|r| r.contains("waits for your consent")),
        "{info:?}"
    );
    let pending: Vec<String> = info
        .consent
        .iter()
        .filter(|item| item.decision == ConsentDecision::Pending)
        .map(|item| item.key.clone())
        .collect();
    for key in ["tools:git", "tools:git:network", "resources:open"] {
        assert!(pending.iter().any(|k| k == key), "{key}: {pending:?}");
    }
    let (ok, message) = run(&agent, status).await;
    assert!(
        !ok && message.contains("waits for your consent"),
        "{message}"
    );
    assert_eq!(runs(&log), Vec::<Vec<String>>::new(), "nothing ran");
    let first = backups(&env);
    assert_eq!(first.len(), 1, "{first:?}\n{}", agent.log());

    // ---- the user's consent: the stored key runs through the plugin.
    match agent
        .ok(Request::DecideModuleMigration {
            module: "git".into(),
            approve: pending,
            decline: Vec::new(),
        })
        .await
    {
        Response::ModuleMigration { .. } => {}
        other => panic!("unexpected {other:?}"),
    }
    wait_for(&agent, "cut over", |m| {
        m.phase == MigrationPhase::CutOver && m.route == MigrationRoute::Plugin
    })
    .await;
    let deadline = Instant::now() + SETTLE;
    let message = loop {
        let (ok, message) = run(&agent, status).await;
        if ok {
            break message;
        }
        assert!(
            Instant::now() < deadline,
            "the key did not run: {message}\n{}",
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(message.starts_with("main"), "{message}");
    let ran = runs(&log);
    assert!(
        ran.iter().any(|args| args.ends_with(&[
            "rev-parse".to_owned(),
            "--show-toplevel".to_owned(),
            "--absolute-git-dir".to_owned()
        ])),
        "the host's work tree check ran, so the plugin did: {ran:?}"
    );
    assert!(
        ran.iter()
            .any(|args| args.iter().any(|a| a == &repo.display().to_string())),
        "{ran:?}"
    );
    agent.stop().await;

    // ---- a restart: still cut over, no second backup, nothing rewritten.
    let agent = env.start().await;
    wait_for(&agent, "still cut over", |m| {
        m.phase == MigrationPhase::CutOver && m.route == MigrationRoute::Plugin
    })
    .await;
    agent.stop().await;
    assert_eq!(backups(&env), first);
    assert_eq!(stored(), before, "the profile is byte for byte as stored");
}
