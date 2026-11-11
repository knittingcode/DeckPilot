//! Tests against real, temporary repositories. Nothing outside the test's
//! temporary folder is touched; the "remote" is a local bare repository,
//! so no network is used.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{
    ActionParameters, ExecutionContext, MachineConfig, MachineResourceBinding, Os, ParameterValue,
    Profile, Project, ProjectKey, ResourceKey, ResourceKind, ResourceRef,
};

use super::*;

struct Fixture {
    root: PathBuf,
    profile: Profile,
    other: Profile,
    machine: MachineConfig,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Runs git for the test's own setup, independent of the user's
/// configuration.
fn setup(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .expect("git is installed");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str, message: &str) {
    std::fs::write(dir.join(file), message).unwrap();
    setup(dir, &["add", "--", file]);
    setup(dir, &["commit", "-q", "-m", message]);
}

fn key(value: &str) -> ResourceKey {
    ResourceKey::new(value).unwrap()
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "deckpilot-git-test-{}",
        deckpilot_core::ProfileId::new()
    ));
    for dir in ["shop", "cms", "plain"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    setup(&root.join("shop"), &["init", "-q"]);
    setup(&root.join("cms"), &["init", "-q"]);
    let mut profile = Profile::new("Work");
    let mut other = Profile::new("Home");
    let mut machine = MachineConfig::new(Os::current());
    for (project_key, dir, home) in [
        ("SHOP", "shop", false),
        ("CMS", "cms", false),
        ("HOME", "plain", true),
    ] {
        let target = if home { &mut other } else { &mut profile };
        let resource = format!("{}.repository", project_key.to_lowercase());
        let mut project = Project::new(ProjectKey::new(project_key).unwrap(), project_key);
        project
            .resources
            .insert("repository".into(), key(&resource));
        target.projects.push(project);
        machine.set_binding(MachineResourceBinding {
            key: key(&resource),
            kind: ResourceKind::Directory,
            value: root.join(dir).display().to_string(),
        });
    }
    machine.set_binding(MachineResourceBinding {
        key: key("tools.editor"),
        kind: ResourceKind::Application,
        value: "Editor".into(),
    });
    Fixture {
        root,
        profile,
        other,
        machine,
    }
}

impl Fixture {
    fn shop(&self) -> PathBuf {
        self.root.join("shop")
    }

    /// The run's project is SHOP.
    fn context(&self) -> ExecutionContext<'_> {
        ExecutionContext::new(
            &self.profile,
            Some(&self.profile.projects[0]),
            &self.machine,
        )
    }
}

fn handler(kind: &str) -> Box<dyn ActionHandler> {
    handler_with(
        kind,
        crate::settings(&ActionParameters::new()),
        Arc::new(RecordingLauncher::new()),
    )
}

fn handler_with(
    kind: &str,
    settings: GitSettings,
    launcher: Arc<dyn Launcher>,
) -> Box<dyn ActionHandler> {
    handlers(&Arc::new(settings), &{ launcher })
        .into_iter()
        .find(|h| h.definition().id.as_str() == kind)
        .unwrap()
}

fn run(kind: &str, parameters: &ActionParameters, context: &ExecutionContext<'_>) -> ActionResult {
    let handler = handler(kind);
    let normalized = handler.definition().normalize(parameters).unwrap();
    let result = handler.execute(&normalized, context);
    // Git actions never change the run's or DeckPilot's project.
    assert_eq!(result.effects, []);
    result
}

fn none() -> ActionParameters {
    ActionParameters::new()
}

#[test]
fn every_action_type_is_namespaced_and_read_only_or_opening() {
    let ids: Vec<_> = handlers(
        &Arc::new(crate::settings(&none())),
        &(Arc::new(RecordingLauncher::new()) as Arc<dyn Launcher>),
    )
    .iter()
    .map(|h| {
        let d = h.definition();
        let expected = if d.id.as_str() == OPEN_REPOSITORY {
            ActionEffect::Opens
        } else {
            ActionEffect::ReadOnly
        };
        assert_eq!(d.node.effect, expected, "{}", d.id);
        assert!(d.node.outputs.iter().any(|o| o.key == "repository"));
        d.id.to_string()
    })
    .collect();
    assert_eq!(
        ids,
        [
            STATUS,
            BRANCH,
            CHANGES,
            LAST_COMMIT,
            REMOTE_STATUS,
            OPEN_REPOSITORY
        ]
    );
}

#[test]
fn a_repository_without_commits() {
    let fixture = fixture();
    let status = run(STATUS, &none(), &fixture.context());
    assert!(status.is_success(), "{}", status.message);
    assert_eq!(status.data["branch"], "main");
    assert_eq!(status.data["unborn"], "true");
    assert_eq!(status.data["detached"], "false");
    assert_eq!(status.data["clean"], "true");
    assert_eq!(status.data["projectKey"], "SHOP");
    assert!(!status.data.contains_key("head"));
    let last = run(LAST_COMMIT, &none(), &fixture.context());
    assert_eq!(last.code.as_deref(), Some(codes::NO_COMMITS));
}

#[test]
fn status_changes_and_the_last_commit() {
    let fixture = fixture();
    let shop = fixture.shop();
    commit(&shop, "a.txt", "first");
    commit(&shop, "b.txt", "Add b");
    std::fs::write(shop.join("a.txt"), "changed").unwrap();
    std::fs::write(shop.join("b.txt"), "staged").unwrap();
    setup(&shop, &["add", "b.txt"]);
    std::fs::write(shop.join("new file.txt"), "untracked").unwrap();
    let index = std::fs::read(shop.join(".git/index")).unwrap();

    let status = run(STATUS, &none(), &fixture.context());
    assert!(status.is_success(), "{}", status.message);
    assert_eq!(status.data["clean"], "false");
    assert_eq!(
        (
            status.data["staged"].as_str(),
            status.data["unstaged"].as_str(),
            status.data["untracked"].as_str()
        ),
        ("1", "1", "1")
    );
    assert_eq!(status.data["head"].len(), 40);

    let changes = run(
        CHANGES,
        &none().with("limit", ParameterValue::Integer(2)),
        &fixture.context(),
    );
    assert_eq!(changes.data["changed"], "3");
    assert_eq!(changes.data["files"].lines().count(), 2);
    assert_eq!(changes.data["truncated"], "true");
    let all = run(CHANGES, &none(), &fixture.context());
    assert!(
        all.data["files"].contains("?? new file.txt"),
        "{}",
        all.data["files"]
    );

    let last = run(LAST_COMMIT, &none(), &fixture.context());
    assert_eq!(last.data["subject"], "Add b");
    assert_eq!(last.data["author"], "Test");
    assert_eq!(last.data["hash"], status.data["head"]);
    assert!(last.data["date"].contains('T'));

    // Reading did not write to the repository.
    assert_eq!(std::fs::read(shop.join(".git/index")).unwrap(), index);
    assert!(!shop.join(".git/index.lock").exists());
}

#[test]
fn a_detached_head() {
    let fixture = fixture();
    let shop = fixture.shop();
    commit(&shop, "a.txt", "first");
    let head = setup(&shop, &["rev-parse", "HEAD"]);
    setup(&shop, &["checkout", "-q", "--detach", &head]);
    let branch = run(BRANCH, &none(), &fixture.context());
    assert!(branch.is_success(), "{}", branch.message);
    assert_eq!(branch.data["detached"], "true");
    assert_eq!(branch.data["head"], head);
    assert!(!branch.data.contains_key("branch"));
    let remote = run(REMOTE_STATUS, &none(), &fixture.context());
    assert_eq!(remote.data["hasUpstream"], "false");
}

#[test]
fn repositories_of_explicit_projects_and_resources() {
    let fixture = fixture();
    let cms = fixture.profile.projects[1].id;
    commit(&fixture.root.join("cms"), "c.txt", "cms commit");
    let explicit = run(
        LAST_COMMIT,
        &none().with(repo::PROJECT, ParameterValue::Project(cms)),
        &fixture.context(),
    );
    assert_eq!(explicit.data["subject"], "cms commit");
    assert_eq!(explicit.data["projectKey"], "CMS");

    let by_resource = run(
        BRANCH,
        &none().with(
            repo::REPOSITORY,
            ParameterValue::Resource(ResourceRef::Fixed {
                key: key("cms.repository"),
            }),
        ),
        &fixture.context(),
    );
    assert!(by_resource.is_success(), "{}", by_resource.message);
    assert!(!by_resource.data.contains_key("projectKey"));

    // A project of another profile is refused when saving and when running.
    let foreign = fixture.other.projects[0].id;
    let parameters = none().with(repo::PROJECT, ParameterValue::Project(foreign));
    assert_eq!(
        handler(STATUS).check(&parameters, &fixture.profile),
        ["the project does not belong to this profile"]
    );
    let refused = run(STATUS, &parameters, &fixture.context());
    assert_eq!(refused.code.as_deref(), Some(codes::PROJECT_NOT_IN_PROFILE));

    // Without a project there is nothing to fall back to.
    let no_project = ExecutionContext::new(&fixture.profile, None, &fixture.machine);
    assert_eq!(
        run(STATUS, &none(), &no_project).code.as_deref(),
        Some("no-active-project")
    );
}

#[test]
fn missing_repositories_and_git() {
    let mut fixture = fixture();
    let mut project = Project::new(ProjectKey::new("NONE").unwrap(), "None");
    project
        .resources
        .insert("repository".into(), key("none.repository"));
    fixture.profile.projects.push(project);
    fixture.machine.set_binding(MachineResourceBinding {
        key: key("plain.repository"),
        kind: ResourceKind::Directory,
        value: fixture.root.join("plain").display().to_string(),
    });
    let plain = none().with(
        repo::REPOSITORY,
        ParameterValue::Resource(ResourceRef::Fixed {
            key: key("plain.repository"),
        }),
    );
    let context = fixture.context();
    assert_eq!(
        run(STATUS, &plain, &context).code.as_deref(),
        Some(codes::NOT_A_REPOSITORY)
    );
    let unbound = none().with(
        repo::PROJECT,
        ParameterValue::Project(fixture.profile.projects[2].id),
    );
    assert_eq!(
        run(STATUS, &unbound, &context).code.as_deref(),
        Some("resource-unbound")
    );

    let missing_git = handler_with(
        STATUS,
        crate::settings(&none().with(
            crate::EXECUTABLE,
            ParameterValue::Text("/nonexistent/deckpilot/git".into()),
        )),
        Arc::new(RecordingLauncher::new()),
    );
    let normalized = missing_git.definition().normalize(&none()).unwrap();
    let result = missing_git.execute(&normalized, &context);
    assert_eq!(result.code.as_deref(), Some(codes::GIT_NOT_FOUND));
}

/// SHOP with `origin`, a local bare repository, tracking `main`.
fn with_remote(fixture: &Fixture) -> PathBuf {
    let shop = fixture.shop();
    let remote = fixture.root.join("remote.git");
    std::fs::create_dir_all(&remote).unwrap();
    setup(&remote, &["init", "-q", "--bare"]);
    commit(&shop, "a.txt", "first");
    setup(
        &shop,
        &["remote", "add", "origin", &remote.display().to_string()],
    );
    setup(&shop, &["push", "-q", "-u", "origin", "main"]);
    remote
}

#[test]
fn remote_status_from_what_was_last_fetched() {
    let fixture = fixture();
    let shop = fixture.shop();
    let no_upstream = run(REMOTE_STATUS, &none(), &fixture.context());
    assert_eq!(no_upstream.data["hasUpstream"], "false");

    with_remote(&fixture);
    commit(&shop, "b.txt", "local only");
    let cached = run(REMOTE_STATUS, &none(), &fixture.context());
    assert!(cached.is_success(), "{}", cached.message);
    assert_eq!(cached.data["source"], "cached");
    assert_eq!(cached.data["upstream"], "origin/main");
    assert_eq!(cached.data["remote"], "origin");
    assert_eq!(
        (
            cached.data["ahead"].as_str(),
            cached.data["behind"].as_str()
        ),
        ("1", "0")
    );
    assert!(!cached.data.contains_key("remoteHead"));
}

#[test]
fn refreshing_asks_the_remote_without_changing_the_repository() {
    let fixture = fixture();
    let shop = fixture.shop();
    let remote = with_remote(&fixture);
    let refresh = none().with("refresh", ParameterValue::Boolean(true));

    let fresh = run(REMOTE_STATUS, &refresh, &fixture.context());
    assert!(fresh.is_success(), "{}", fresh.message);
    assert_eq!(fresh.data["source"], "network");
    assert_eq!(fresh.data["stale"], "false");

    // Someone else pushes.
    let other = fixture.root.join("other");
    setup(
        &fixture.root,
        &["clone", "-q", &remote.display().to_string(), "other"],
    );
    commit(&other, "c.txt", "from elsewhere");
    setup(&other, &["push", "-q", "origin", "main"]);

    let tracking = setup(&shop, &["rev-parse", "origin/main"]);
    let moved = run(REMOTE_STATUS, &refresh, &fixture.context());
    assert_eq!(moved.data["stale"], "true", "{}", moved.message);
    assert_ne!(moved.data["remoteHead"], tracking);
    assert_eq!(moved.data["trackingHead"], tracking);
    // ls-remote does not fetch: the tracking branch is unchanged.
    assert_eq!(setup(&shop, &["rev-parse", "origin/main"]), tracking);

    // A remote that is no longer configured.
    setup(&shop, &["config", "branch.main.remote", "gone"]);
    let missing = run(REMOTE_STATUS, &refresh, &fixture.context());
    assert_eq!(
        missing.code.as_deref(),
        Some(codes::NO_REMOTE),
        "{}",
        missing.message
    );
}

#[test]
fn opening_a_repository() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let open = handler_with(OPEN_REPOSITORY, crate::settings(&none()), launcher.clone());
    let parameters = none().with(
        "application",
        ParameterValue::Resource(ResourceRef::Fixed {
            key: key("tools.editor"),
        }),
    );
    let normalized = open.definition().normalize(&parameters).unwrap();
    let result = open.execute(&normalized, &fixture.context());
    assert!(result.is_success(), "{}", result.message);
    let plans = launcher.plans();
    assert_eq!(plans.len(), 1);
    assert!(
        plans[0]
            .args
            .iter()
            .chain([&plans[0].program])
            .any(|a| a.contains("shop"))
    );
}
