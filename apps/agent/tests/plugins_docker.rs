//! `deckpilot.docker` end to end through the real agent, the real
//! sandboxed host and plugin API 1.9's real `tool.run` (feature
//! `plugins`): the plugin's `compose` and `environment` parameters and the
//! active project's compose role become handles of the invocation, and
//! every docker it asks for is the agent's own run of a **fake `docker`**
//! (a shell script first on the agent's `PATH` that prints the built-in
//! tests' recorded output and logs its arguments), with the agent's
//! `--context`, compose folder, file and project name. No engine, no
//! container. See `support/first_party.rs`.
//!
//! * [`every_action_runs_through_tool_run`]: the permissions ask for
//!   consent (`tools:docker` and `tools:docker:modify` high tier), then
//!   every action runs as the user started it, with the built-in's
//!   outputs and messages;
//! * [`changes_run_only_in_runs_the_user_started`]: Compose down from a
//!   scheduled workflow is refused (`not-user-initiated`) and docker never
//!   sees it;
//! * [`nothing_runs_without_consent`]: before the consent nothing runs;
//! * [`changing_containers_is_a_separate_grant_off_by_default`], where
//!   the plugin is enabled without `tools:docker:modify` (X3c, optional: only
//!   the Compose changes name it); the reads run, the changes are listed
//!   unavailable ("needs the permission") and never reach docker until the
//!   user grants it, and revoking it again stops only them;
//! * [`reads_need_no_resource`], where status, containers and container logs
//!   run without a Docker environment or a project, as with the built-in,
//!   in an empty folder of the agent's own.
//!
//! The plugin is pinned as a `default`-class plugin: `bundled.json`
//! releases it as `migration` (its routing from the legacy `docker.*` ids
//! is WP-C1). Unix only (the fake is a shell script).
#![cfg(all(feature = "plugins", unix))]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::path::{Path, PathBuf};

use deckpilot_core::{
    ActionInstanceId, MachineConfig, MachineResourceBinding, Os, ParameterValue, Profile, Project,
    ProjectKey, ResourceKey, ResourceKind, ResourceRef, WorkflowId,
};
use deckpilot_ipc::Request;
use deckpilot_ipc::plugins::{PermissionStatus, PermissionTier, PluginState};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Env, FirstParty};

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.docker",
    dir: "docker",
    krate: "deckpilot-docker",
    released: "migration",
};

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

/// The fake `docker` (the built-in tests' script): logs its arguments,
/// takes the host's `--context` and compose options off and answers like
/// an engine with Compose and two containers.
fn fake_docker(bin: &Path, log: &Path) {
    std::fs::create_dir_all(bin).unwrap();
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$@" >> '{log}'
echo '@@end@@' >> '{log}'
pwd -P >> '{log}.cwd'
ls -A >> '{log}.cwd'
ctx=default
if [ "$1" = "--context" ]; then ctx=$2; shift 2; fi
case "$1" in
  context) echo "$ctx" ;;
  version) echo '{{"Client":{{"Version":"29.1.0","Os":"linux"}},"Server":{{"Version":"29.1.0","Os":"linux"}}}}' ;;
  ps) echo '{{"Names":"web-1","Image":"nginx","State":"running","Status":"Up 1 hour","Labels":"token=abc"}}'
      echo '{{"Names":"job-1","Image":"busybox","State":"exited","Status":"Exited (0)"}}' ;;
  logs) echo 'out line'; echo 'err line' >&2 ;;
  compose)
    shift
    while [ $# -gt 0 ]; do
      case "$1" in
        --project-directory|-f) shift 2 ;;
        --project-name=*) shift ;;
        *) break ;;
      esac
    done
    case "$1" in
      version) echo 2.30.0 ;;
      ps) echo '{{"Name":"shop-web-1","Service":"web","State":"running","Status":"Up"}}'
          echo '{{"Name":"shop-db-1","Service":"db","State":"exited","Status":"Exited (1)"}}' ;;
      up) echo 'Container shop-web-1 Started' >&2 ;;
      down) echo 'Container shop-web-1 Removed' >&2 ;;
      restart) echo 'Container shop-web-1 Restarted' >&2 ;;
      logs) echo 'web  | hello' ;;
      *) echo "fake docker: unsupported compose $*" >&2; exit 1 ;;
    esac ;;
  *) echo "fake docker: unsupported $*" >&2; exit 1 ;;
esac
"#,
        log = log.display()
    );
    support::fakes::write_script(&bin.join("docker"), &script);
}

fn calls(log: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .split("@@end@@\n")
        .filter(|call| !call.is_empty())
        .map(|call| call.lines().map(str::to_owned).collect())
        .collect()
}

struct Fixture {
    profile: Profile,
    project: deckpilot_core::ProjectId,
    machine: MachineConfig,
    shop: PathBuf,
    bin: PathBuf,
    log: PathBuf,
    status: ActionInstanceId,
    /// Status without a Docker environment.
    plain_status: ActionInstanceId,
    /// Container logs without a Docker environment.
    plain_logs: ActionInstanceId,
    containers: ActionInstanceId,
    compose_status: ActionInstanceId,
    up: ActionInstanceId,
    down: ActionInstanceId,
    restart: ActionInstanceId,
    container_logs: ActionInstanceId,
    service_logs: ActionInstanceId,
    no_file: ActionInstanceId,
    scheduled: WorkflowId,
}

fn fixture(dir: &Path) -> Fixture {
    let shop = dir.join("src/shop");
    let empty = dir.join("src/empty");
    std::fs::create_dir_all(&shop).unwrap();
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(shop.join("compose.yaml"), "name: shop\nservices: {}\n").unwrap();
    let shop = shop.canonicalize().unwrap();
    let bin = dir.join("bin");
    let log = dir.join("docker.log");
    fake_docker(&bin, &log);
    let mut machine = MachineConfig::new(Os::current());
    bind(
        &mut machine,
        "shop.compose",
        ResourceKind::Directory,
        &shop.display().to_string(),
    );
    bind(
        &mut machine,
        "empty.compose",
        ResourceKind::Directory,
        &empty.canonicalize().unwrap().display().to_string(),
    );
    bind(
        &mut machine,
        "remote.docker",
        ResourceKind::DockerEnvironment,
        "remote-prod",
    );

    let mut profile = Profile::new("Work");
    let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    project
        .resources
        .insert("compose".into(), ResourceKey::new("shop.compose").unwrap());
    let project_id = project.id;
    profile.projects.push(project);
    let id = PLUGIN.id;
    let mut add = |action: &str, params: &[(&str, ParameterValue)]| {
        first_party::instance(&mut profile, id, action, params)
    };
    let status = add("status", &[("environment", fixed("remote.docker"))]);
    let plain_status = add("status", &[]);
    let plain_logs = add(
        "logs",
        &[("container", ParameterValue::Text("web-1".into()))],
    );
    let containers = add(
        "containers",
        &[
            ("all", ParameterValue::Boolean(true)),
            ("name", ParameterValue::Text("web".into())),
        ],
    );
    let compose_status = add("compose-status", &[]);
    let up = add(
        "compose-up",
        &[("services", ParameterValue::Text("web\n\ndb\n".into()))],
    );
    let down = add(
        "compose-down",
        &[("removeOrphans", ParameterValue::Boolean(true))],
    );
    let restart = add("compose-restart", &[]);
    let container_logs = add(
        "logs",
        &[
            ("container", ParameterValue::Text("web-1".into())),
            ("lines", ParameterValue::Integer(20)),
            ("since", ParameterValue::Text("10m".into())),
            ("environment", fixed("remote.docker")),
        ],
    );
    let service_logs = add("logs", &[("service", ParameterValue::Text("web".into()))]);
    let no_file = add("compose-status", &[("compose", fixed("empty.compose"))]);
    let scheduled = first_party::scheduled_workflow(&mut profile, "Every minute", down);
    Fixture {
        profile,
        project: project_id,
        machine,
        shop,
        bin,
        log,
        status,
        plain_status,
        plain_logs,
        containers,
        compose_status,
        up,
        down,
        restart,
        container_logs,
        service_logs,
        no_file,
        scheduled,
    }
}

fn env(name: &str, f: &Fixture) -> Env {
    let mut env = Env::new(
        name,
        PLUGIN,
        BundleClass::Default,
        Some((&f.profile, Some(f.project))),
        Some(&f.machine),
    );
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![f.bin.clone()];
    dirs.extend(std::env::split_paths(&path));
    env.extra_env
        .push(("PATH".into(), std::env::join_paths(dirs).unwrap()));
    env
}

/// The compose arguments the host puts before the subcommand.
fn compose_prefix(shop: &Path) -> Vec<String> {
    vec![
        "compose".into(),
        "--project-directory".into(),
        shop.display().to_string(),
        "-f".into(),
        shop.join("compose.yaml").display().to_string(),
        "--project-name=shop".into(),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one path, step by step
async fn every_action_runs_through_tool_run() {
    let dir = support::TempDir::new("plugins-docker-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-docker", &f);
    let agent = env.start().await;

    // ---- seeded, waiting for consent: running docker is high tier, and
    // changing containers a separate high-tier grant (optional).
    let summary = agent.seeded().await;
    assert_ne!(summary.state, PluginState::Enabled, "{}", agent.log());
    assert_eq!(summary.actions.len(), 7);
    let details = agent.details().await;
    for key in ["tools:docker", "tools:docker:modify"] {
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
    assert_eq!(calls(&f.log).len(), 0);
    agent.enable().await;

    // ---- status in a bound Docker environment: its context.
    let status = agent.run_action(f.status).await;
    assert!(status.ok, "{status:?}\n{}", agent.log());
    assert_eq!(
        status.message,
        "Docker 29.1.0 in context remote-prod, Compose 2.30.0"
    );
    assert_eq!(status.data["available"], "true");
    assert_eq!(status.data["context"], "remote-prod");
    assert_eq!(status.data["serverVersion"], "29.1.0");
    assert_eq!(status.data["composeVersion"], "2.30.0");
    for call in calls(&f.log) {
        assert_eq!(call[..2], ["--context", "remote-prod"], "{call:?}");
    }

    // ---- containers, without their labels; Docker's own context.
    let listed = agent.run_action(f.containers).await;
    assert!(listed.ok, "{listed:?}\n{}", agent.log());
    assert_eq!(
        listed.message,
        "2 containers, 1 running, in context default"
    );
    assert_eq!(listed.data["count"], "2");
    assert_eq!(listed.data["running"], "1");
    assert_eq!(listed.data["names"], "web-1,job-1");
    assert!(!listed.data["containers"].contains("token"));
    let call = calls(&f.log).pop().unwrap();
    assert_eq!(
        call,
        [
            "ps",
            "--no-trunc",
            "--format={{json .}}",
            "--all",
            "--filter=name=web"
        ]
    );

    // ---- compose status: the project's compose role, the folder's file
    // and project name from the host.
    let compose = agent.run_action(f.compose_status).await;
    assert!(compose.ok, "{compose:?}\n{}", agent.log());
    assert_eq!(
        compose.message,
        "1 of 2 containers running in context default"
    );
    assert_eq!(compose.data["services"], "2");
    assert_eq!(compose.data["allRunning"], "false");
    assert_eq!(compose.data["projectKey"], "SHOP");
    let mut expected = compose_prefix(&f.shop);
    expected.extend(["ps".into(), "--all".into(), "--format=json".into()]);
    assert_eq!(calls(&f.log).pop().unwrap(), expected);

    // ---- the changes, as the user started them.
    let up = agent.run_action(f.up).await;
    assert!(up.ok, "{up:?}\n{}", agent.log());
    assert_eq!(up.message, "web, db started in context default");
    assert_eq!(up.data["output"], "Container shop-web-1 Started");
    let mut expected = compose_prefix(&f.shop);
    expected.extend(["up", "--detach", "--", "web", "db"].map(String::from));
    assert!(calls(&f.log).contains(&expected), "{:?}", calls(&f.log));

    let down = agent.run_action(f.down).await;
    assert!(down.ok, "{down:?}");
    assert_eq!(
        down.message,
        "the project stopped and removed (volumes kept) in context default"
    );
    let mut expected = compose_prefix(&f.shop);
    expected.extend(["down", "--remove-orphans"].map(String::from));
    assert!(calls(&f.log).contains(&expected), "{:?}", calls(&f.log));

    let restart = agent.run_action(f.restart).await;
    assert!(restart.ok, "{restart:?}");
    assert_eq!(restart.message, "the project restarted in context default");

    // ---- logs of a container (in the bound environment) and of a
    // Compose service.
    let logs = agent.run_action(f.container_logs).await;
    assert!(logs.ok, "{logs:?}");
    assert_eq!(logs.data["logs"], "out line\nerr line");
    assert_eq!(logs.data["lineCount"], "2");
    assert_eq!(logs.data["truncated"], "false");
    assert_eq!(
        calls(&f.log).pop().unwrap(),
        [
            "--context",
            "remote-prod",
            "logs",
            "--tail=20",
            "--since=10m",
            "--",
            "web-1"
        ]
    );
    let service = agent.run_action(f.service_logs).await;
    assert!(service.ok, "{service:?}");
    assert_eq!(service.data["logs"], "web  | hello");
    assert_eq!(service.data["projectKey"], "SHOP");

    // ---- a bound folder without a compose file: refused by the host.
    let before = calls(&f.log).len();
    let refused = agent.run_action(f.no_file).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("compose-file-not-found"),
        "{refused:?}"
    );
    assert!(
        calls(&f.log)[before..].iter().all(|c| c[0] != "compose"),
        "a compose run without a file"
    );

    // Never a destructive flag, never a plugin-chosen file or context.
    for call in calls(&f.log) {
        for forbidden in ["-v", "--volumes", "--rmi", "--follow"] {
            assert!(!call.iter().any(|a| a == forbidden), "{call:?}");
        }
    }
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changes_run_only_in_runs_the_user_started() {
    let dir = support::TempDir::new("plugins-docker-schedule-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-docker-schedule", &f);
    let agent = env.start().await;
    agent.seeded().await;
    agent.enable().await;
    // The agent's clock moves past the workflow's next due time (again
    // and again, in case a configuration reload restarted the schedule
    // after a jump): its schedule runs Compose down, which may not change
    // anything.
    let mut ahead = 0;
    let deadline = std::time::Instant::now() + first_party::SETTLE;
    let execution = loop {
        ahead += 61;
        std::fs::write(env.clock_offset(), ahead.to_string()).unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let deckpilot_ipc::Response::Executions { executions } = agent
            .ok(Request::ListExecutions {
                workflow: Some(f.scheduled),
                limit: None,
            })
            .await
        else {
            panic!("executions")
        };
        if let Some(run) = executions.last() {
            break run.id;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the schedule did not fire\n{}",
            agent.log()
        );
    };
    let record = agent.finished(execution).await;
    let run = first_party::node(&record, "run");
    let error = run.error.as_ref().expect("the node failed");
    assert_eq!(
        error.action_code.as_deref(),
        Some("not-user-initiated"),
        "{record:#?}"
    );
    assert!(error.message.contains("the user started"), "{record:#?}");
    assert!(
        !calls(&f.log).iter().any(|c| c.iter().any(|a| a == "down")),
        "docker saw the change: {:?}",
        calls(&f.log)
    );
    // The same action, as the user starts it.
    let down = agent.run_action(f.down).await;
    assert!(down.ok, "{down:?}");
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_runs_without_consent() {
    let dir = support::TempDir::new("plugins-docker-consent-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-docker-consent", &f);
    let agent = env.start().await;
    agent.seeded().await;
    let refused = agent.run_action(f.status).await;
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(calls(&f.log).len(), 0, "docker ran before the consent");
    agent.enable().await;
    assert!(agent.run_action(f.status).await.ok);
    // Revoking the read grant stops every action.
    agent
        .ok(Request::RevokePluginPermission {
            plugin: PLUGIN.id.into(),
            permission: "tools:docker".into(),
        })
        .await;
    let before = calls(&f.log).len();
    for action in [f.up, f.status] {
        let refused = agent.run_action(action).await;
        assert!(!refused.ok, "{refused:?}");
        assert!(
            refused.message.contains("`tools:docker` is not granted"),
            "{refused:?}"
        );
    }
    assert_eq!(calls(&f.log).len(), before);
    agent.stop().await;
}

/// The availability of the plugin's actions, by action id.
async fn availability(agent: &first_party::Agent) -> Vec<(String, Option<String>)> {
    agent
        .details()
        .await
        .summary
        .actions
        .iter()
        .map(|a| {
            assert_eq!(a.available, a.unavailable_reason.is_none(), "{a:?}");
            (a.action.clone(), a.unavailable_reason.clone())
        })
        .collect()
}

const CHANGES: [&str; 3] = ["compose-up", "compose-down", "compose-restart"];

/// X3c: changing containers is a separate grant, off by default. The
/// plugin is enabled without it; only the Compose changes wait for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changing_containers_is_a_separate_grant_off_by_default() {
    let dir = support::TempDir::new("plugins-docker-optional-fixture");
    let f = fixture(dir.path());
    let env = env("plugins-docker-optional", &f);
    let agent = env.start().await;
    agent.seeded().await;
    let details = agent.details().await;
    let modify = details
        .permissions
        .iter()
        .find(|p| p.key == "tools:docker:modify")
        .expect("the modify sub-grant");
    assert!(modify.optional, "{modify:?}");
    assert_eq!(modify.tier, PermissionTier::High);
    assert_eq!(modify.actions, CHANGES);
    assert!(
        details
            .permissions
            .iter()
            .filter(|p| p.key != "tools:docker:modify")
            .all(|p| !p.optional),
        "{:?}",
        details.permissions
    );

    // ---- enabled with the required grants only.
    let consent = agent.enable_required().await;
    assert!(!consent.iter().any(|k| k == "tools:docker:modify"));
    let needs = "needs the permission `tools:docker:modify`";
    for (action, reason) in availability(&agent).await {
        if CHANGES.contains(&action.as_str()) {
            let reason = reason.unwrap_or_default();
            assert!(reason.contains(needs), "{action}: {reason}");
        } else {
            assert_eq!(reason, None, "{action}");
        }
    }
    // The reads run; the changes never reach docker.
    let status = agent.run_action(f.status).await;
    assert!(
        status.ok,
        "{status:?}
{}",
        agent.log()
    );
    assert!(agent.run_action(f.compose_status).await.ok);
    let before = calls(&f.log).len();
    for change in [f.up, f.down, f.restart] {
        let refused = agent.run_action(change).await;
        assert!(!refused.ok, "{refused:?}");
        assert!(refused.message.contains(needs), "{refused:?}");
    }
    assert_eq!(calls(&f.log).len(), before, "docker saw a change");

    // ---- granted: the changes run.
    agent
        .ok(Request::GrantPluginPermissions {
            plugin: PLUGIN.id.into(),
            permissions: vec!["tools:docker:modify".into()],
        })
        .await;
    assert!(
        availability(&agent).await.iter().all(|(_, r)| r.is_none()),
        "{:?}",
        availability(&agent).await
    );
    let up = agent.run_action(f.up).await;
    assert!(
        up.ok,
        "{up:?}
{}",
        agent.log()
    );

    // ---- revoked again: only the changes stop.
    agent
        .ok(Request::RevokePluginPermission {
            plugin: PLUGIN.id.into(),
            permission: "tools:docker:modify".into(),
        })
        .await;
    assert_eq!(
        agent.summary().await.map(|s| s.state),
        Some(PluginState::Enabled)
    );
    for (action, reason) in availability(&agent).await {
        assert_eq!(
            reason.is_some(),
            CHANGES.contains(&action.as_str()),
            "{action}: {reason:?}"
        );
    }
    assert!(agent.run_action(f.containers).await.ok);
    let before = calls(&f.log).len();
    let refused = agent.run_action(f.down).await;
    assert!(refused.message.contains(needs), "{refused:?}");
    assert_eq!(calls(&f.log).len(), before);
    agent.stop().await;
}

/// Status, containers and container logs read no folder: without a Docker
/// environment and without a project they run, as the built-in did, in
/// an empty folder of the agent's own; Compose still needs its project.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_need_no_resource() {
    let dir = support::TempDir::new("plugins-docker-unbound-fixture");
    let mut f = fixture(dir.path());
    // No project is active.
    f.profile.projects.clear();
    let env = Env::new(
        "plugins-docker-unbound",
        PLUGIN,
        BundleClass::Default,
        Some((&f.profile, None)),
        Some(&f.machine),
    );
    let mut env = env;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![f.bin.clone()];
    dirs.extend(std::env::split_paths(&path));
    env.extra_env
        .push(("PATH".into(), std::env::join_paths(dirs).unwrap()));
    let agent = env.start().await;
    agent.seeded().await;
    agent.enable_required().await;
    let status = agent.run_action(f.plain_status).await;
    assert!(
        status.ok,
        "{status:?}
{}",
        agent.log()
    );
    assert_eq!(status.data["context"], "default");
    assert!(agent.run_action(f.containers).await.ok);
    let logs = agent.run_action(f.plain_logs).await;
    assert!(logs.ok, "{logs:?}");
    assert_eq!(
        calls(&f.log).pop().unwrap(),
        ["logs", "--tail=100", "--", "web-1"]
    );
    // Every run was in one empty folder the agent made, never a folder of
    // the user's.
    let cwd = std::fs::read_to_string(f.log.with_extension("log.cwd")).unwrap();
    let folders: std::collections::BTreeSet<&str> = cwd.lines().collect();
    assert_eq!(folders.len(), 1, "{cwd}");
    let folder = folders.first().unwrap();
    assert!(
        Path::new(folder)
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("deckpilot-empty-")),
        "{folder}"
    );
    assert!(
        !folder.starts_with(&*dir.path().to_string_lossy()),
        "{folder}"
    );
    // Compose needs its project.
    let refused = agent.run_action(f.compose_status).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("no-active-project"),
        "{refused:?}"
    );
    agent.stop().await;
}
