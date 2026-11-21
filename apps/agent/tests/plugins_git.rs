//! `deckpilot.git` end to end through the real agent, the real sandboxed
//! host and plugin API 1.9's real `tool.run` (feature `plugins`): the
//! plugin's `repository` parameters and the active project's repository
//! role become handles of the invocation, and every git it asks for is the
//! agent's own run of a **fake `git`** (a shell script first on the
//! agent's `PATH` that prints recorded output and logs its arguments and
//! whether it saw the SSH agent), with the agent's fixed prefix, its
//! environment and its work tree check (`rev-parse --show-toplevel
//! --absolute-git-dir`). No repository,
//! no network, no editor (`DECKPILOT_TEST_PLUGIN_LAUNCH`). See
//! `support/first_party.rs`.
//!
//! * [`every_action_runs_through_tool_run`]: the permissions ask for
//!   consent (`tools:git` and `tools:git:network` high tier), then every
//!   action runs as the user started it, with the built-in's outputs and
//!   messages, and git only ever sees what the host's policy allows;
//! * [`nothing_runs_without_consent`]: revoking `tools:git:network`
//!   refuses `ls-remote` and nothing else, revoking `tools:git` refuses
//!   every run before git starts.
//!
//! The plugin is pinned as a `default`-class plugin: `bundled.json`
//! releases it as `migration`, which only the migration engine installs
//! (its routing from the legacy `git.*` ids is WP-C1). Unix only (the fake
//! is a shell script).
#![cfg(all(feature = "plugins", unix))]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::path::{Path, PathBuf};

use deckpilot_core::{
    ActionInstanceId, MachineConfig, MachineResourceBinding, Os, ParameterValue, Profile, Project,
    ProjectKey, ResourceKey, ResourceKind, ResourceRef,
};
use deckpilot_ipc::Request;
use deckpilot_ipc::plugins::{PermissionStatus, PermissionTier, PluginState};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Env, FirstParty};

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.git",
    dir: "git",
    krate: "deckpilot-git",
    released: "migration",
};
const OID: &str = "1234567890abcdef1234567890abcdef12345678";
const REMOTE_OID: &str = "abcdefabcdefabcdefabcdefabcdefabcdefabcd";
const EDITOR: &str = "/usr/local/bin/code";
const SSH_SOCK: &str = "/tmp/deckpilot-test-ssh-agent.sock";

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

/// The fake `git`: logs `<args…> @@ssh=<SSH_AUTH_SOCK or none>@@` per run,
/// skips the host's prefix (`-C <dir>`, `-c <k=v>`, `--no-pager`…) and
/// answers like git in a repository on `main` tracking `origin/main`, one
/// commit ahead, with a staged, an unstaged and an untracked file. A
/// folder named `plain` is not a repository.
fn fake_git(bin: &Path, log: &Path) -> PathBuf {
    std::fs::create_dir_all(bin).unwrap();
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$@" >> '{log}'
printf '@@ssh=%s@@\n' "${{SSH_AUTH_SOCK:-none}}" >> '{log}'
while [ $# -gt 0 ]; do
  case "$1" in
    -C|-c) shift 2 ;;
    --no-pager|--no-optional-locks) shift ;;
    *) break ;;
  esac
done
here=$(pwd -P)
case "$here" in
  */plain) echo 'fatal: not a git repository (or any of the parent directories): .git' >&2; exit 128 ;;
esac
case "$*" in
  "rev-parse --show-toplevel") echo "$here" ;;
  "rev-parse --show-toplevel --absolute-git-dir") echo "$here"; echo "$here/.git" ;;
  "status --porcelain=v2 --branch -z --untracked-files=all")
    printf '%s\000' '# branch.oid {OID}' '# branch.head main' '# branch.upstream origin/main' '# branch.ab +1 -0' \
      '1 .M N... 100644 100644 100644 aaaa aaaa a.txt' '1 M. N... 100644 100644 100644 bbbb cccc b.txt' '? new file.txt' ;;
  "status --porcelain=v2 --branch -z --untracked-files=no")
    printf '%s\000' '# branch.oid {OID}' '# branch.head main' '# branch.upstream origin/main' '# branch.ab +1 -0' \
      '1 .M N... 100644 100644 100644 aaaa aaaa a.txt' '1 M. N... 100644 100644 100644 bbbb cccc b.txt' ;;
  "log -1 --no-show-signature --format=format:%H%x00%h%x00%an%x00%aI%x00%s")
    printf '%s\000%s\000%s\000%s\000%s' '{OID}' '1234567' 'Test' '2026-11-20T10:00:00+01:00' 'Add b' ;;
  "config --get branch.main.remote") echo origin ;;
  "config --get branch.main.merge") echo refs/heads/main ;;
  "remote") echo origin ;;
  "rev-parse --verify -q origin/main") echo '{OID}' ;;
  "ls-remote -- origin refs/heads/main") printf '%s\trefs/heads/main\n' '{REMOTE_OID}' ;;
  *) echo "fake git: unsupported $*" >&2; exit 1 ;;
esac
"#,
        log = log.display()
    );
    let path = bin.join("git");
    support::fakes::write_script(&path, &script);
    path
}

/// The runs the fake logged: each run's arguments and the SSH agent it
/// saw.
fn runs(log: &Path) -> Vec<(Vec<String>, String)> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut out = Vec::new();
    let mut args = Vec::new();
    for line in text.lines() {
        if let Some(ssh) = line
            .strip_prefix("@@ssh=")
            .and_then(|l| l.strip_suffix("@@"))
        {
            out.push((std::mem::take(&mut args), ssh.to_owned()));
        } else {
            args.push(line.to_owned());
        }
    }
    out
}

/// The subcommand and what follows, after the host's prefix.
fn subcommand(args: &[String]) -> Vec<String> {
    let mut rest = args;
    loop {
        match rest.first().map(String::as_str) {
            Some("-C" | "-c") => rest = &rest[2..],
            Some("--no-pager" | "--no-optional-locks") => rest = &rest[1..],
            _ => return rest.to_vec(),
        }
    }
}

struct Fixture {
    profile: Profile,
    project: deckpilot_core::ProjectId,
    machine: MachineConfig,
    repo: PathBuf,
    bin: PathBuf,
    log: PathBuf,
    status: ActionInstanceId,
    branch: ActionInstanceId,
    changes: ActionInstanceId,
    last_commit: ActionInstanceId,
    cached: ActionInstanceId,
    refreshed: ActionInstanceId,
    open: ActionInstanceId,
    plain: ActionInstanceId,
}

fn fixture(dir: &Path) -> Fixture {
    let repo = dir.join("src/shop");
    let plain = dir.join("src/plain");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(&plain).unwrap();
    let repo = repo.canonicalize().unwrap();
    let bin = dir.join("bin");
    let log = dir.join("git.log");
    fake_git(&bin, &log);
    let mut machine = MachineConfig::new(Os::current());
    bind(
        &mut machine,
        "shop.repo",
        ResourceKind::Directory,
        &repo.display().to_string(),
    );
    bind(
        &mut machine,
        "shop.plain",
        ResourceKind::Directory,
        &plain.canonicalize().unwrap().display().to_string(),
    );
    bind(
        &mut machine,
        "tools.editor",
        ResourceKind::Application,
        EDITOR,
    );

    let mut profile = Profile::new("Work");
    let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    shop.resources
        .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
    let project = shop.id;
    profile.projects.push(shop);
    let id = PLUGIN.id;
    let mut add = |action: &str, params: &[(&str, ParameterValue)]| {
        first_party::instance(&mut profile, id, action, params)
    };
    let status = add("status", &[]);
    let branch = add("branch", &[("repository", fixed("shop.repo"))]);
    let changes = add("changes", &[("limit", ParameterValue::Integer(2))]);
    let last_commit = add("last-commit", &[]);
    let cached = add("remote-status", &[]);
    let refreshed = add(
        "remote-status",
        &[("refresh", ParameterValue::Boolean(true))],
    );
    let open = add("open-repository", &[("application", fixed("tools.editor"))]);
    let plain = add("status", &[("repository", fixed("shop.plain"))]);
    Fixture {
        profile,
        project,
        machine,
        repo,
        bin,
        log,
        status,
        branch,
        changes,
        last_commit,
        cached,
        refreshed,
        open,
        plain,
    }
}

fn env(name: &str, f: &Fixture) -> Env {
    let mut env = Env::new(
        name,
        PLUGIN,
        BundleClass::Default,
        Some((&f.profile, Some(f.project))),
        Some(&f.machine),
    )
    .with_projects(BundleClass::Default);
    // The fake first on the agent's own `PATH`: the host runs the user's
    // `git` from there (the executable is never the plugin's).
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![f.bin.clone()];
    dirs.extend(std::env::split_paths(&path));
    env.extra_env
        .push(("PATH".into(), std::env::join_paths(dirs).unwrap()));
    env.extra_env
        .push(("SSH_AUTH_SOCK".into(), SSH_SOCK.into()));
    env
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one path, step by step
async fn every_action_runs_through_tool_run() {
    let dir = support::TempDir::new("plugins-git-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-git", &f);
    let agent = env.start().await;

    // ---- seeded, waiting for consent: running git is high tier.
    let summary = agent.seeded().await;
    assert_ne!(summary.state, PluginState::Enabled, "{}", agent.log());
    assert_eq!(summary.actions.len(), 6);
    let details = agent.details().await;
    for key in ["tools:git", "tools:git:network", "resources:open"] {
        assert!(
            details.permissions.iter().any(|p| p.key == key
                && p.tier == PermissionTier::High
                && p.status != PermissionStatus::Granted),
            "{key}: {:?}",
            details.permissions
        );
    }
    for permission in &details.permissions {
        let text = permission.justification.as_deref().unwrap_or_default();
        assert!(
            !text.is_empty() && !text.starts_with("t:"),
            "{permission:?}"
        );
    }
    // Nothing ran before the consent.
    assert_eq!(runs(&f.log).len(), 0);
    agent.enable().await;

    // ---- status: the active project's repository role.
    let status = agent.run_action(f.status).await;
    assert!(status.ok, "{status:?}\n{}", agent.log());
    assert_eq!(status.message, "main; 3 changed; 1 ahead, 0 behind");
    assert_eq!(status.data["branch"], "main");
    assert_eq!(status.data["head"], OID);
    assert_eq!(status.data["upstream"], "origin/main");
    assert_eq!(status.data["clean"], "false");
    assert_eq!(
        (
            status.data["staged"].as_str(),
            status.data["unstaged"].as_str(),
            status.data["untracked"].as_str(),
            status.data["conflicts"].as_str()
        ),
        ("1", "1", "1", "0")
    );
    assert_eq!(
        (
            status.data["ahead"].as_str(),
            status.data["behind"].as_str()
        ),
        ("1", "0")
    );
    assert_eq!(status.data["repository"], f.repo.display().to_string());
    assert_eq!(status.data["projectKey"], "SHOP");

    // ---- branch: a fixed repository resource, no project key.
    let branch = agent.run_action(f.branch).await;
    assert!(branch.ok, "{branch:?}");
    assert_eq!(branch.message, "main");
    assert_eq!(branch.data["detached"], "false");
    assert!(!branch.data.contains_key("projectKey"), "{branch:?}");

    // ---- changes, limited to 2.
    let changes = agent.run_action(f.changes).await;
    assert!(changes.ok, "{changes:?}");
    assert_eq!(changes.message, "3 changed paths");
    assert_eq!(changes.data["files"], ".M a.txt\nM. b.txt");
    assert_eq!(changes.data["truncated"], "true");

    // ---- last commit.
    let last = agent.run_action(f.last_commit).await;
    assert!(last.ok, "{last:?}");
    assert_eq!(last.message, "1234567 Add b");
    assert_eq!(last.data["hash"], OID);
    assert_eq!(last.data["author"], "Test");
    assert_eq!(last.data["date"], "2026-11-20T10:00:00+01:00");

    // ---- remote status, as last fetched: no network.
    let cached = agent.run_action(f.cached).await;
    assert!(cached.ok, "{cached:?}");
    assert_eq!(
        cached.message,
        "main → origin/main: 1 ahead, 0 behind as last fetched"
    );
    assert_eq!(cached.data["source"], "cached");
    assert_eq!(cached.data["remote"], "origin");
    assert_eq!(cached.data["trackingHead"], OID);
    assert!(!cached.data.contains_key("remoteHead"));
    assert!(
        !runs(&f.log)
            .iter()
            .any(|(a, _)| a.contains(&"ls-remote".to_owned()))
    );

    // ---- remote status, asking the remote: the only network run, and
    // the only one that sees the SSH agent.
    let refreshed = agent.run_action(f.refreshed).await;
    assert!(refreshed.ok, "{refreshed:?}\n{}", agent.log());
    assert_eq!(refreshed.data["source"], "network");
    assert_eq!(refreshed.data["remoteHead"], REMOTE_OID);
    assert_eq!(refreshed.data["stale"], "true");
    assert!(refreshed.message.ends_with("; the remote has moved since"));

    // ---- open repository: in the bound editor, through the launcher.
    let opened = agent.run_action(f.open).await;
    assert!(opened.ok, "{opened:?}\n{}", agent.log());
    assert_eq!(opened.message, format!("opened {}", f.repo.display()));
    let launched = env.launched();
    assert_eq!(launched.len(), 1, "{launched:?}");
    assert!(
        launched[0].0 == EDITOR || launched[0].1.iter().any(|a| a == EDITOR),
        "{launched:?}"
    );
    assert!(
        launched[0].1.iter().any(|a| Path::new(a) == f.repo),
        "{launched:?}"
    );

    // ---- a folder that is not a repository: the agent's check refuses
    // it before any of the plugin's runs.
    let plain = agent.run_action(f.plain).await;
    assert_eq!(plain.code.as_deref(), Some("not-a-repository"), "{plain:?}");
    assert!(
        plain.message.ends_with("is not a Git repository"),
        "{plain:?}"
    );

    // ---- what git saw: the host's prefix, never an option of the
    // plugin's before the subcommand, the policy's subcommands only, and
    // the SSH agent only for `ls-remote`.
    let seen = runs(&f.log);
    assert_ne!(seen.len(), 0);
    for (args, ssh) in &seen {
        assert_eq!(args[0], "-C", "{args:?}");
        assert!(args.iter().any(|a| a == "--no-optional-locks"), "{args:?}");
        assert!(args.iter().any(|a| a == "core.fsmonitor=false"), "{args:?}");
        let sub = subcommand(args);
        assert!(
            [
                "status",
                "log",
                "config",
                "remote",
                "rev-parse",
                "ls-remote"
            ]
            .contains(&sub[0].as_str()),
            "{args:?}"
        );
        let network = sub[0] == "ls-remote";
        assert_eq!(ssh == SSH_SOCK, network, "{args:?} saw {ssh}");
    }
    assert_eq!(
        seen.iter()
            .filter(|(a, _)| subcommand(a)[0] == "ls-remote")
            .count(),
        1
    );
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_runs_without_consent() {
    let dir = support::TempDir::new("plugins-git-consent-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-git-consent", &f);
    let agent = env.start().await;
    agent.seeded().await;
    // Not enabled yet: nothing runs.
    let refused = agent.run_action(f.status).await;
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(runs(&f.log).len(), 0, "git ran before the consent");
    let consent = agent.enable().await;
    assert!(
        consent.iter().any(|k| k == "tools:git:network"),
        "{consent:?}"
    );
    assert!(agent.run_action(f.cached).await.ok);

    // Each sub-grant is consented to on its own; while one is revoked the
    // agent lists every action of the plugin as unavailable, and git
    // never starts.
    for permission in ["tools:git:network", "tools:git"] {
        agent
            .ok(Request::RevokePluginPermission {
                plugin: PLUGIN.id.into(),
                permission: permission.into(),
            })
            .await;
        let before = runs(&f.log).len();
        for action in [f.status, f.refreshed] {
            let refused = agent.run_action(action).await;
            assert!(!refused.ok, "{refused:?}");
            assert!(
                refused
                    .message
                    .contains(&format!("`{permission}` is not granted")),
                "{refused:?}"
            );
        }
        assert_eq!(runs(&f.log).len(), before, "git ran without `{permission}`");
        agent
            .ok(Request::GrantPluginPermissions {
                plugin: PLUGIN.id.into(),
                permissions: vec![permission.into()],
            })
            .await;
        let again = agent.run_action(f.refreshed).await;
        assert!(again.ok, "{again:?}\n{}", agent.log());
    }
    agent.stop().await;
}
