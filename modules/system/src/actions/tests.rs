use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{
    ActionParameters, CancellationToken, Capability, ExecutionContext, ExecutionControl,
    MachineConfig, MachineResourceBinding, Os, ParameterValue, Profile, Progress, Project,
    ProjectKey, ResourceKey, ResourceKind, ResourceRef,
};

use super::*;

struct Fixture {
    profile: Profile,
    machine: MachineConfig,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn key(value: &str) -> ResourceKey {
    ResourceKey::new(value).unwrap()
}

fn fixed(value: &str) -> ParameterValue {
    ParameterValue::Resource(ResourceRef::Fixed { key: key(value) })
}

fn role(value: &str) -> ParameterValue {
    ParameterValue::Resource(ResourceRef::ProjectRole { role: value.into() })
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.into())
}

fn shell() -> String {
    if cfg!(windows) { "cmd" } else { "sh" }.into()
}

/// A script argument list for the platform's shell.
fn script(unix: &str, windows: &str) -> String {
    if cfg!(windows) {
        format!("/C\n{windows}")
    } else {
        format!("-c\n{unix}")
    }
}

fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-system-test-{}",
        deckpilot_core::ProfileId::new()
    ));
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs/README.md"), "hi").unwrap();
    let mut profile = Profile::new("Work");
    let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    project
        .resources
        .insert("repository".into(), key("shop.repository"));
    profile.projects.push(project);
    let mut machine = MachineConfig::new(Os::current());
    for (k, kind, value) in [
        (
            "shop.repository",
            ResourceKind::Directory,
            dir.display().to_string(),
        ),
        ("tools.shell", ResourceKind::Other, shell()),
        (
            "tools.editor",
            ResourceKind::Application,
            "Editor".to_owned(),
        ),
        (
            "tools.missing-app",
            ResourceKind::Application,
            dir.join("Missing.app").display().to_string(),
        ),
    ] {
        machine.set_binding(MachineResourceBinding {
            key: key(k),
            kind,
            value,
        });
    }
    Fixture {
        profile,
        machine,
        dir,
    }
}

impl Fixture {
    fn context(&self) -> ExecutionContext<'_> {
        ExecutionContext::new(
            &self.profile,
            Some(&self.profile.projects[0]),
            &self.machine,
        )
    }
}

fn run(
    handler: &dyn ActionHandler,
    parameters: impl Into<ActionParameters>,
    context: &ExecutionContext<'_>,
) -> ActionResult {
    let parameters = parameters.into();
    let normalized = handler.definition().normalize(&parameters).unwrap();
    handler.execute(&normalized, context)
}

fn code(result: &ActionResult) -> Option<&str> {
    result.code.as_deref()
}

#[test]
fn every_action_type_declares_its_node_contract() {
    let launcher: Arc<dyn Launcher> = Arc::new(RecordingLauncher::new());
    let handlers = handlers(&launcher, false);
    let ids: Vec<_> = handlers
        .iter()
        .map(|h| h.definition().id.to_string())
        .collect();
    assert_eq!(
        ids,
        [
            OPEN_APP,
            OPEN_URL,
            OPEN_FILE,
            OPEN_DIRECTORY,
            RUN_COMMAND,
            PROCESS_STATUS
        ]
    );
    for handler in &handlers {
        let definition = handler.definition();
        assert!(!definition.node.outputs.is_empty(), "{}", definition.id);
        assert!(definition.node.icon.is_some());
        let expected = match definition.id.as_str() {
            RUN_COMMAND => ActionEffect::Modifies,
            PROCESS_STATUS => ActionEffect::ReadOnly,
            _ => ActionEffect::Opens,
        };
        assert_eq!(definition.node.effect, expected, "{}", definition.id);
    }
    let command = RunCommand::new(true);
    let spec = &command.definition().node;
    assert!(spec.cancellable && !spec.retry_safe && !spec.idempotent);
    let sensitive: Vec<_> = spec
        .outputs
        .iter()
        .filter(|o| o.sensitive)
        .map(|o| o.key.as_str())
        .collect();
    assert_eq!(sensitive, ["stdout", "stderr"]);
}

#[test]
fn urls_open_through_the_platform_launcher() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = OpenUrl::new(launcher.clone());
    let result = run(
        &handler,
        ActionParameters::new().with("url", text("https://example.com/x")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["url"], "https://example.com/x");
    let plans = launcher.plans();
    assert_eq!(plans.len(), 1);
    assert!(plans[0].args.contains(&"https://example.com/x".to_owned()));

    let refused = run(
        &handler,
        ActionParameters::new().with("url", text("file:///etc/passwd")),
        &fixture.context(),
    );
    assert_eq!(code(&refused), Some(crate::codes::INVALID_URL));
    assert_eq!(launcher.plans().len(), 1, "nothing else was opened");
}

#[test]
fn opening_requires_the_capability() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let granted = BTreeSet::from([Capability::MachineResources]);
    let context = fixture.context().restricted(&granted);
    let result = run(
        &OpenUrl::new(launcher.clone()),
        ActionParameters::new().with("url", text("https://example.com")),
        &context,
    );
    assert_eq!(code(&result), Some("not-permitted"));
    assert_eq!(launcher.plans(), []);
}

#[test]
fn applications_come_from_resources() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = OpenApp::new(launcher.clone());
    let result = run(
        &handler,
        ActionParameters::new()
            .with("application", fixed("tools.editor"))
            .with("arguments", text("--new-window\nmy file.txt")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    let plan = &launcher.plans()[0];
    assert!(
        plan.args.ends_with(&["my file.txt".to_owned()])
            || plan.args.contains(&"my file.txt".to_owned())
    );

    let unbound = run(
        &handler,
        ActionParameters::new().with("application", fixed("tools.unbound")),
        &fixture.context(),
    );
    assert_eq!(code(&unbound), Some("resource-unbound"));
    let missing = run(
        &handler,
        ActionParameters::new().with("application", fixed("tools.missing-app")),
        &fixture.context(),
    );
    assert_eq!(code(&missing), Some(crate::codes::NOT_FOUND));
    assert_eq!(launcher.plans().len(), 1);

    let failing = OpenApp::new(Arc::new(RecordingLauncher::failing()));
    let result = run(
        &failing,
        ActionParameters::new().with("application", fixed("tools.editor")),
        &fixture.context(),
    );
    assert_eq!(code(&result), Some("program-not-found"));
}

#[test]
fn files_and_folders_stay_inside_their_resource() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let folder = OpenDirectory::new(launcher.clone());
    let file = OpenFile::new(launcher.clone());

    let opened = run(
        &folder,
        ActionParameters::new()
            .with("location", role("repository"))
            .with("path", text("docs")),
        &fixture.context(),
    );
    assert!(opened.is_success(), "{}", opened.message);
    assert_eq!(
        PathBuf::from(&opened.data["path"]),
        fixture.dir.join("docs")
    );

    let readme = run(
        &file,
        ActionParameters::new()
            .with("location", role("repository"))
            .with("path", text("docs/README.md"))
            .with("application", fixed("tools.editor")),
        &fixture.context(),
    );
    assert!(readme.is_success(), "{}", readme.message);
    assert_eq!(launcher.plans().len(), 2);

    for (path, expected) in [
        ("../outside", crate::codes::INVALID_PATH),
        ("docs/missing.md", crate::codes::NOT_FOUND),
    ] {
        let result = run(
            &file,
            ActionParameters::new()
                .with("location", role("repository"))
                .with("path", text(path)),
            &fixture.context(),
        );
        assert_eq!(code(&result), Some(expected), "{path}");
    }
    // Without an active project, a project role cannot be resolved.
    let no_project = ExecutionContext::new(&fixture.profile, None, &fixture.machine);
    let result = run(
        &folder,
        ActionParameters::new().with("location", role("repository")),
        &no_project,
    );
    assert_eq!(code(&result), Some("no-active-project"));
    assert_eq!(launcher.plans().len(), 2);
}

fn command(arguments: &str) -> ActionParameters {
    ActionParameters::new()
        .with("program", fixed("tools.shell"))
        .with("arguments", text(arguments))
        .with("workingDirectory", role("repository"))
}

#[test]
fn commands_are_off_until_allowed() {
    let fixture = fixture();
    let result = run(
        &RunCommand::new(false),
        command(&script("echo hi", "echo hi")),
        &fixture.context(),
    );
    assert_eq!(code(&result), Some(crate::codes::COMMANDS_DISABLED));
}

#[test]
fn commands_run_without_a_shell_in_their_folder() {
    let fixture = fixture();
    let handler = RunCommand::new(true);
    // The whole line is one argument: it reaches the script unsplit.
    let result = run(
        &handler,
        command(&script("pwd; printf '%s' \"$0\"", "cd")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["exitCode"], "0");
    let folder = fixture.dir.canonicalize().unwrap();
    let reported = PathBuf::from(result.data["stdout"].lines().next().unwrap().trim());
    assert_eq!(reported.canonicalize().unwrap(), folder);

    let failed = run(
        &handler,
        command(&script("exit 3", "exit 3")),
        &fixture.context(),
    );
    assert_eq!(code(&failed), Some(crate::codes::EXIT_NONZERO));
    assert_eq!(failed.data["exitCode"], "3");
    let tolerated = run(
        &handler,
        command(&script("exit 3", "exit 3")).with("failOnNonZero", ParameterValue::Boolean(false)),
        &fixture.context(),
    );
    assert!(tolerated.is_success());
}

#[test]
fn commands_do_not_see_the_agents_secrets() {
    let fixture = fixture();
    // Only names outside the minimal environment are removed; use one the
    // test process has (cargo sets it when running tests).
    assert!(std::env::var_os("CARGO_PKG_NAME").is_some());
    let result = run(
        &RunCommand::new(true),
        command(&script(
            "printf '[%s]' \"${CARGO_PKG_NAME:-}\"",
            "echo [%CARGO_PKG_NAME%]",
        )),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert!(
        !result.data["stdout"].contains("deckpilot"),
        "{}",
        result.data["stdout"]
    );
}

#[test]
fn commands_stop_when_cancelled_or_out_of_time() {
    let fixture = fixture();
    let handler = RunCommand::new(true);
    let long = command(&script("sleep 30", "ping -n 30 127.0.0.1"));

    let token = CancellationToken::new();
    let progress = |_: Progress| {};
    let control = ExecutionControl::new(&token, &progress);
    let context = fixture.context().with_control(&control);
    let canceller = {
        let token = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            token.cancel();
        })
    };
    let begun = Instant::now();
    let result = run(&handler, long.clone(), &context);
    canceller.join().unwrap();
    assert_eq!(code(&result), Some("cancelled"), "{}", result.message);
    assert!(begun.elapsed() < Duration::from_secs(10));

    let begun = Instant::now();
    let result = run(
        &handler,
        long.with("timeoutSeconds", ParameterValue::Integer(1)),
        &fixture.context(),
    );
    assert_eq!(code(&result), Some("timed-out"), "{}", result.message);
    assert!(begun.elapsed() < Duration::from_secs(10));
}

#[test]
fn programs_must_be_bound_and_valid() {
    let mut fixture = fixture();
    fixture.machine.set_binding(MachineResourceBinding {
        key: key("tools.relative"),
        kind: ResourceKind::Other,
        value: "bin/tool".into(),
    });
    let handler = RunCommand::new(true);
    for (program, expected) in [
        ("tools.unbound", "resource-unbound"),
        ("tools.relative", crate::codes::INVALID_PATH),
    ] {
        let result = run(
            &handler,
            ActionParameters::new().with("program", fixed(program)),
            &fixture.context(),
        );
        assert_eq!(code(&result), Some(expected), "{program}");
    }
    let granted = BTreeSet::from([Capability::MachineResources]);
    let restricted = fixture.context().restricted(&granted);
    let denied = run(&handler, command(&script("true", "exit 0")), &restricted);
    assert_eq!(code(&denied), Some("not-permitted"));
}

#[test]
fn process_status_reports_by_name() {
    let fixture = fixture();
    let handler = ProcessStatus::new();
    let result = run(
        &handler,
        ActionParameters::new().with("name", text("deckpilot-no-such-program")),
        &fixture.context(),
    );
    assert!(result.is_success());
    assert_eq!(result.data["running"], "false");
    assert_eq!(result.data["count"], "0");
    assert_eq!(normalize_process_name(" Slack.EXE "), "slack");

    let granted = BTreeSet::from([Capability::MachineResources]);
    let restricted = fixture.context().restricted(&granted);
    let denied = run(
        &handler,
        ActionParameters::new().with("name", text("x")),
        &restricted,
    );
    assert_eq!(code(&denied), Some("not-permitted"));
}

#[cfg(unix)]
#[test]
fn process_status_finds_running_programs() {
    let fixture = fixture();
    let mut child = std::process::Command::new("sleep")
        .arg("10")
        .spawn()
        .unwrap();
    let result = run(
        &ProcessStatus::new(),
        ActionParameters::new().with("name", text("sleep")),
        &fixture.context(),
    );
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(result.data["running"], "true", "{}", result.message);
    assert!(
        result.data["pids"]
            .split(',')
            .any(|pid| pid == child.id().to_string())
    );
}

/// A link at `link` to `target`; false where links cannot be made
/// (Windows without the privilege).
fn make_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    made.is_ok()
}

#[test]
fn links_out_of_the_resource_are_not_opened() {
    let fixture = fixture();
    let outside = fixture.dir.with_extension("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "out").unwrap();
    let made = make_link(&outside.join("secret.txt"), &fixture.dir.join("leak.txt"))
        && make_link(&outside, &fixture.dir.join("leak-dir"))
        && make_link(&fixture.dir.join("docs"), &fixture.dir.join("manual"));
    if !made {
        eprintln!("links cannot be made here; skipped");
        let _ = std::fs::remove_dir_all(&outside);
        return;
    }
    let launcher = Arc::new(RecordingLauncher::new());
    let folder = OpenDirectory::new(launcher.clone());
    let file = OpenFile::new(launcher.clone());
    let at = |path: &str| {
        ActionParameters::new()
            .with("location", role("repository"))
            .with("path", text(path))
    };
    for (handler, path) in [
        (&file as &dyn ActionHandler, "leak.txt"),
        (&file, "leak-dir/secret.txt"),
        (&folder, "leak-dir"),
    ] {
        let result = run(handler, at(path), &fixture.context());
        assert_eq!(
            code(&result),
            Some(crate::codes::INVALID_PATH),
            "{path}: {}",
            result.message
        );
    }
    assert_eq!(launcher.plans().len(), 0);
    // A link that stays inside opens.
    let inside = run(&file, at("manual/README.md"), &fixture.context());
    assert!(inside.is_success(), "{}", inside.message);
    let inside = run(&folder, at("manual"), &fixture.context());
    assert!(inside.is_success(), "{}", inside.message);
    assert_eq!(launcher.plans().len(), 2);
    let _ = std::fs::remove_dir_all(&outside);
}
