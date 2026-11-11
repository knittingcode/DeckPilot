//! Nothing is opened: a recording launcher keeps the plans.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{
    FeatureModule, MachineConfig, MachineResourceBinding, Project, ProjectKey, ResourceKey,
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

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.into())
}

fn choice(value: &str) -> ParameterValue {
    ParameterValue::Choice(value.into())
}

fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-workspace-test-{}",
        deckpilot_core::ProfileId::new()
    ));
    let repo = dir.join("shop");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("handbook")).unwrap();
    std::fs::write(repo.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(dir.join("secret.txt"), "outside").unwrap();

    let mut profile = Profile::new("Work");
    let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    for (role, resource) in [
        ("repository", "shop.repository"),
        ("editor", "tools.editor"),
        ("docs", "shop.docs"),
        ("dashboard", "shop.dashboard"),
        ("ci", "shop.ci"),
        ("bad", "shop.bad"),
    ] {
        shop.resources.insert(role.into(), key(resource));
    }
    profile.projects.push(shop);
    let mut empty = Project::new(ProjectKey::new("EMPTY").unwrap(), "Empty");
    empty.resources.insert("docs".into(), key("empty.docs"));
    profile.projects.push(empty);
    let mut handbook = Project::new(ProjectKey::new("BOOK").unwrap(), "Book");
    handbook.resources.insert("docs".into(), key("book.docs"));
    profile.projects.push(handbook);

    let mut machine = MachineConfig::new(Os::current());
    for (k, kind, value) in [
        (
            "shop.repository",
            ResourceKind::Directory,
            repo.display().to_string(),
        ),
        (
            "tools.editor",
            ResourceKind::Application,
            "Editor".to_owned(),
        ),
        (
            "shop.docs",
            ResourceKind::Url,
            "https://docs.example.com/shop".to_owned(),
        ),
        (
            "shop.dashboard",
            ResourceKind::Url,
            "https://grafana.example.com/d/shop".to_owned(),
        ),
        // shop.ci is not bound on this computer.
        (
            "shop.bad",
            ResourceKind::Url,
            "javascript:alert(1)".to_owned(),
        ),
        (
            "book.docs",
            ResourceKind::Directory,
            dir.join("handbook").display().to_string(),
        ),
        (
            "tools.browser",
            ResourceKind::Application,
            "Firefox".to_owned(),
        ),
        (
            "tools.code-cli",
            ResourceKind::Application,
            dir.join("bin/code").display().to_string(),
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
        self.context_for(0)
    }

    fn context_for(&self, project: usize) -> ExecutionContext<'_> {
        ExecutionContext::new(
            &self.profile,
            Some(&self.profile.projects[project]),
            &self.machine,
        )
    }

    fn repo(&self) -> PathBuf {
        self.dir.join("shop")
    }
}

fn handler(kind: &str, launcher: &Arc<RecordingLauncher>) -> Box<dyn ActionHandler> {
    handlers(&(launcher.clone() as Arc<dyn Launcher>))
        .into_iter()
        .find(|h| h.definition().id.as_str() == kind)
        .unwrap()
}

fn run_with(
    kind: &str,
    launcher: &Arc<RecordingLauncher>,
    parameters: &ActionParameters,
    context: &ExecutionContext<'_>,
) -> ActionResult {
    let handler = handler(kind, launcher);
    let normalized = handler.definition().normalize(parameters).unwrap();
    handler.execute(&normalized, context)
}

fn editor_parameters() -> ActionParameters {
    ActionParameters::new().with("application", fixed("tools.editor"))
}

fn expected(
    application: &str,
    path: &Path,
    editor: Editor,
    position: Option<Position>,
) -> LaunchPlan {
    editor::editor_plan(Os::current(), editor, application, path, position)
        .unwrap()
        .0
}

#[test]
fn the_module_and_its_contract() {
    let module = crate::WorkspaceModule::new();
    let manifest = module.manifest();
    assert_eq!(manifest.id.as_str(), "workspace");
    assert_eq!(
        manifest.capabilities,
        [
            Capability::ProfileRead,
            Capability::MachineResources,
            Capability::OpenExternal
        ]
    );
    let all = module.action_handlers(&ActionParameters::new());
    let ids: Vec<_> = all.iter().map(|h| h.definition().id.to_string()).collect();
    assert_eq!(ids, [OPEN_IN_EDITOR, OPEN_URLS, OPEN_PROJECT]);
    for handler in &all {
        assert_eq!(handler.definition().node.effect, ActionEffect::Opens);
    }
}

#[test]
fn the_repository_opens_in_the_editor() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let result = run_with(
        OPEN_IN_EDITOR,
        &launcher,
        &editor_parameters(),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["located"], "false");
    assert_eq!(
        launcher.plans(),
        [expected("Editor", &fixture.repo(), Editor::Other, None)]
    );
}

#[test]
fn a_file_opens_at_a_line_with_the_editor_s_convention() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let file = fixture.repo().join("src/main.rs");
    for (editor, kind) in [
        ("vscode", Editor::VsCode),
        ("cursor", Editor::Cursor),
        ("jetbrains", Editor::JetBrains),
        ("zed", Editor::Zed),
        ("sublime", Editor::Sublime),
    ] {
        let result = run_with(
            OPEN_IN_EDITOR,
            &launcher,
            &editor_parameters()
                .with("editor", choice(editor))
                .with("path", text("src/main.rs"))
                .with("line", ParameterValue::Integer(12))
                .with("column", ParameterValue::Integer(4)),
            &fixture.context(),
        );
        assert!(result.is_success(), "{editor}: {}", result.message);
        assert_eq!(result.data["located"], "true", "{editor}");
        let position = Some(Position {
            line: 12,
            column: Some(4),
        });
        assert_eq!(
            launcher.plans().pop().unwrap(),
            expected("Editor", &file, kind, position),
            "{editor}"
        );
    }
    // Other editors open the file without the line.
    let other = run_with(
        OPEN_IN_EDITOR,
        &launcher,
        &editor_parameters()
            .with("path", text("src/main.rs"))
            .with("line", ParameterValue::Integer(3)),
        &fixture.context(),
    );
    assert!(other.is_success(), "{}", other.message);
    assert_eq!(other.data["located"], "false");
}

#[test]
fn paths_and_positions_are_checked() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let run = |parameters: ActionParameters| {
        run_with(OPEN_IN_EDITOR, &launcher, &parameters, &fixture.context())
    };
    for outside in [
        "../secret.txt",
        "/etc/passwd",
        "src/../../secret.txt",
        r"C:\x",
    ] {
        let result = run(editor_parameters().with("path", text(outside)));
        assert_eq!(
            result.code.as_deref(),
            Some(system_codes::INVALID_PATH),
            "{outside}"
        );
    }
    let missing = run(editor_parameters().with("path", text("src/nope.rs")));
    assert_eq!(missing.code.as_deref(), Some(system_codes::NOT_FOUND));
    let folder_line = run(editor_parameters()
        .with("path", text("src"))
        .with("line", ParameterValue::Integer(1)));
    assert_eq!(folder_line.code.as_deref(), Some(codes::INVALID_POSITION));
    let column_only = run(editor_parameters()
        .with("path", text("src/main.rs"))
        .with("column", ParameterValue::Integer(1)));
    assert_eq!(column_only.code.as_deref(), Some(codes::INVALID_POSITION));
    // A command-line tool that is not there.
    let cli = run(ActionParameters::new().with("application", fixed("tools.code-cli")));
    assert_eq!(cli.code.as_deref(), Some(system_codes::NOT_FOUND));
    // Without a project there is no repository.
    let no_project = ExecutionContext::new(&fixture.profile, None, &fixture.machine);
    let result = run_with(OPEN_IN_EDITOR, &launcher, &editor_parameters(), &no_project);
    assert_eq!(result.code.as_deref(), Some("no-active-project"));
    assert_eq!(launcher.plans().len(), 0);

    let handler = handler(OPEN_IN_EDITOR, &launcher);
    let bad = editor_parameters()
        .with("path", text("../x"))
        .with("column", ParameterValue::Integer(2));
    assert_eq!(handler.check(&bad, &fixture.profile).len(), 2);
}

#[test]
fn pages_open_in_the_default_or_a_chosen_browser() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let pages = "https://ci.example.com/shop\n\n  http://localhost:3000/  \n";
    let result = run_with(
        OPEN_URLS,
        &launcher,
        &ActionParameters::new().with("urls", text(pages)),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["opened"], "2");
    assert_eq!(result.data["failed"], "0");
    assert!(!result.message.contains("example.com"));
    assert_eq!(
        launcher.plans(),
        [
            editor::url_plan(Os::current(), "https://ci.example.com/shop", None).unwrap(),
            editor::url_plan(Os::current(), "http://localhost:3000/", None).unwrap(),
        ]
    );

    let chosen = Arc::new(RecordingLauncher::new());
    let result = run_with(
        OPEN_URLS,
        &chosen,
        &ActionParameters::new()
            .with("urls", text("https://ci.example.com/shop"))
            .with("browser", fixed("tools.browser")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(
        chosen.plans(),
        [editor::url_plan(
            Os::current(),
            "https://ci.example.com/shop",
            Some("Firefox")
        )
        .unwrap()]
    );
}

#[test]
fn page_lists_are_checked_before_anything_opens() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let run = |urls: &str| {
        run_with(
            OPEN_URLS,
            &launcher,
            &ActionParameters::new().with("urls", text(urls)),
            &fixture.context(),
        )
    };
    let mixed = run("https://ok.example.com\nfile:///etc/passwd");
    assert_eq!(mixed.code.as_deref(), Some(system_codes::INVALID_URL));
    for bad in ["mailto:a@example.com", "javascript:alert(1)", "vscode://x"] {
        assert_eq!(
            run(bad).code.as_deref(),
            Some(system_codes::INVALID_URL),
            "{bad}"
        );
    }
    // Blank lists are refused when saved (required) and when run.
    assert_eq!(
        urls(Some(" \n ")).unwrap_err().code.as_deref(),
        Some(codes::NO_URLS)
    );
    // Too many are refused when saved (a list of at most ten) and when run.
    let eleven = "https://example.com/\n".repeat(MAX_URLS + 1);
    let open = handler(OPEN_URLS, &launcher);
    let parameters = ActionParameters::new().with("urls", text(&eleven));
    assert!(open.definition().normalize(&parameters).is_err());
    assert_eq!(
        open.execute(&parameters, &fixture.context())
            .code
            .as_deref(),
        Some(codes::TOO_MANY_URLS)
    );
    assert_eq!(launcher.plans().len(), 0);

    let failing = Arc::new(RecordingLauncher::failing());
    let failed = run_with(
        OPEN_URLS,
        &failing,
        &ActionParameters::new().with("urls", text("https://example.com/")),
        &fixture.context(),
    );
    assert!(!failed.is_success());

    let handler = handler(OPEN_URLS, &launcher);
    let saved = ActionParameters::new().with("urls", text("ftp://x"));
    assert_eq!(handler.check(&saved, &fixture.profile).len(), 1);
    let templated = ActionParameters::new().with("urls", text("{{ steps.a.output.url }}"));
    assert_eq!(
        handler.check(&templated, &fixture.profile),
        Vec::<String>::new()
    );
}

#[test]
fn the_project_workspace_opens_what_is_bound() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let result = run_with(
        OPEN_PROJECT,
        &launcher,
        &ActionParameters::new()
            .with("editor", choice("vscode"))
            .with("urlRoles", text("dashboard\nci\nbad\nunknown")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["projectKey"], "SHOP");
    assert_eq!(result.data["opened"], "3");
    assert_eq!(result.data["openedRoles"], "repository\ndocs\ndashboard");
    assert_eq!(result.data["missingCount"], "3");
    let missing = &result.data["missing"];
    assert!(missing.contains("ci: "), "{missing}");
    assert!(missing.contains("bad: "), "{missing}");
    assert!(missing.contains("unknown: "), "{missing}");
    assert_eq!(
        launcher.plans(),
        [
            expected("Editor", &fixture.repo(), Editor::VsCode, None),
            editor::url_plan(Os::current(), "https://docs.example.com/shop", None).unwrap(),
            editor::url_plan(Os::current(), "https://grafana.example.com/d/shop", None).unwrap(),
        ]
    );

    // A documentation folder opens in the file manager.
    let book = Arc::new(RecordingLauncher::new());
    let result = run_with(
        OPEN_PROJECT,
        &book,
        &ActionParameters::new(),
        &fixture.context_for(2),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["openedRoles"], "docs");
    assert!(result.data["missing"].contains("repository: "));
    assert_eq!(
        book.plans(),
        [launch::plan(
            Os::current(),
            &OpenTarget::Path(fixture.dir.join("handbook"))
        )
        .unwrap()]
    );
}

#[test]
fn nothing_to_open_is_a_failure() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let result = run_with(
        OPEN_PROJECT,
        &launcher,
        &ActionParameters::new().with("urlRoles", text("dashboard")),
        &fixture.context_for(1),
    );
    assert_eq!(result.code.as_deref(), Some(codes::NOTHING_OPENED));
    assert_eq!(result.data["opened"], "0");
    assert_eq!(result.data["missingCount"], "3");
    assert_eq!(launcher.plans().len(), 0);

    let no_project = ExecutionContext::new(&fixture.profile, None, &fixture.machine);
    let none = run_with(
        OPEN_PROJECT,
        &launcher,
        &ActionParameters::new(),
        &no_project,
    );
    assert_eq!(none.code.as_deref(), Some("no-active-project"));

    let too_many = "a\n".repeat(MAX_URLS + 1);
    let handler = handler(OPEN_PROJECT, &launcher);
    let parameters = ActionParameters::new().with("urlRoles", text(&too_many));
    assert_eq!(handler.check(&parameters, &fixture.profile).len(), 1);
}

#[test]
fn capabilities_are_required() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let without_open = BTreeSet::from([Capability::ProfileRead, Capability::MachineResources]);
    for (kind, parameters) in [
        (OPEN_IN_EDITOR, editor_parameters()),
        (
            OPEN_URLS,
            ActionParameters::new().with("urls", text("https://example.com/")),
        ),
        (OPEN_PROJECT, ActionParameters::new()),
    ] {
        let result = run_with(
            kind,
            &launcher,
            &parameters,
            &fixture.context().restricted(&without_open),
        );
        assert_eq!(result.code.as_deref(), Some("not-permitted"), "{kind}");
    }
    let without_resources = BTreeSet::from([Capability::ProfileRead, Capability::OpenExternal]);
    let editor = run_with(
        OPEN_IN_EDITOR,
        &launcher,
        &editor_parameters(),
        &fixture.context().restricted(&without_resources),
    );
    assert_eq!(editor.code.as_deref(), Some("not-permitted"));
    let project = run_with(
        OPEN_PROJECT,
        &launcher,
        &ActionParameters::new(),
        &fixture.context().restricted(&without_resources),
    );
    assert_eq!(project.code.as_deref(), Some("not-permitted"));
    let without_profile = BTreeSet::from([Capability::MachineResources, Capability::OpenExternal]);
    let project = run_with(
        OPEN_PROJECT,
        &launcher,
        &ActionParameters::new(),
        &fixture.context().restricted(&without_profile),
    );
    assert_eq!(project.code.as_deref(), Some("not-permitted"));
    assert_eq!(launcher.plans().len(), 0);
}
