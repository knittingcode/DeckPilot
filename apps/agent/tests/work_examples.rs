//! The development seed's "Start Work" and "End Work" graphs, run end to
//! end in the agent against fake `git` and `docker` programs. The graphs
//! are the seed's own (`tools/dev/src/seed/examples.rs`, included as is).
//!
//! The editor is the project's `editor` resource, left unbound here: the
//! "Open editor" node fails harmlessly (no application is started) and the
//! run continues, as its policy says.
//!
//! The fake `git` prints NUL-separated porcelain output, which a Windows
//! batch file cannot; these tests run on Unix.
#![cfg(unix)]

mod support;

use deckpilot_core::{
    ActionParameters, ExecutionState, MachineConfig, MachineResourceBinding, Os, ParameterValue,
    Profile, Project, ProjectKey, ResourceKey, ResourceKind,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_modules::projects::ProjectCatalog;
use deckpilot_modules::{docker, git};
use deckpilot_storage::ConfigStore;
use support::{TempDir, TestAgent, fakes};

/// The seed's builder interface, for `examples.rs`.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Builder<'a> {
    registry: &'a deckpilot_core::ActionRegistry,
    profile: Profile,
}

impl Builder<'_> {
    fn action(
        &mut self,
        name: &str,
        action_type: deckpilot_core::ActionTypeId,
        parameters: &ActionParameters,
        icon: &str,
        label: Option<&str>,
    ) -> Result<deckpilot_core::ActionInstanceId> {
        let mut instance = self
            .registry
            .create_instance(name, action_type, parameters)?;
        instance.icon = Some(icon.to_owned());
        instance.label = label.map(str::to_owned);
        let id = instance.id;
        self.profile.actions.push(instance);
        Ok(id)
    }
}

#[path = "../../../tools/dev/src/seed/examples.rs"]
mod examples;

struct Fixture {
    _dir: TempDir,
    agent: TestAgent,
    profile: Profile,
    docker_log: std::path::PathBuf,
}

/// A Work profile with SHOP (repository, Compose project, unbound editor),
/// the seed's workflows, fake programs and an unlocked agent.
async fn fixture() -> Fixture {
    let dir = TempDir::new("deckpilot-work-examples");
    let bin = dir.path().join("bin");
    let shop = dir.path().join("workspace/shop");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&shop).unwrap();
    std::fs::write(shop.join("compose.yaml"), "services: {}\n").unwrap();
    let git = fakes::git(&bin, "feature/e2e");
    let docker = fakes::docker(&bin);

    let registry = support::registry();
    let mut b = Builder {
        registry: &registry,
        profile: Profile::new("Work"),
    };
    let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Online Shop");
    for (role, key) in [
        ("repository", "shop.repository"),
        ("compose", "shop.compose"),
        ("editor", "dev.editor"),
    ] {
        project
            .resources
            .insert(role.into(), ResourceKey::new(key).unwrap());
    }
    b.profile.add_project(project).unwrap();
    b.profile.default_project = Some(b.profile.projects[0].id);
    let actions = examples::actions(&mut b).unwrap();
    let (start, end) = (examples::start_work(&actions), examples::end_work(&actions));
    b.profile.workflows.extend([start, end]);
    let profile = b.profile;

    let text = |path: &std::path::Path| ParameterValue::Text(path.display().to_string());
    let (paths, key) = support::data_dir(
        &dir,
        &profile,
        &[
            (
                git::module_id(),
                ActionParameters::new().with(git::EXECUTABLE, text(&git)),
            ),
            (
                docker::module_id(),
                ActionParameters::new()
                    .with(docker::EXECUTABLE, text(&docker))
                    .with(docker::ALLOW_STATE_CHANGES, ParameterValue::Boolean(true)),
            ),
        ],
    );
    let mut machine = MachineConfig::new(Os::current());
    for (key, kind, value) in [
        ("shop.repository", ResourceKind::Directory, &shop),
        ("shop.compose", ResourceKind::DockerEnvironment, &shop),
    ] {
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new(key).unwrap(),
            kind,
            value: value.display().to_string(),
        });
    }
    // `dev.editor` stays unbound: nothing is ever opened.
    support::open_store(&paths, &key)
        .save_machine_config(&machine)
        .unwrap();
    let agent = TestAgent::start_unlocked(&paths, &key, &MockBackend::new(), None).await;
    Fixture {
        _dir: dir,
        agent,
        profile,
        docker_log: bin.join("docker.log"),
    }
}

fn node<'a>(
    record: &'a deckpilot_core::ExecutionRecord,
    id: &str,
) -> &'a deckpilot_core::graph::NodeReport {
    record
        .nodes
        .iter()
        .find(|n| n.node.as_str() == id)
        .unwrap_or_else(|| panic!("no node {id} in {record:#?}"))
}

fn status(record: &deckpilot_core::ExecutionRecord, id: &str) -> deckpilot_core::graph::NodeStatus {
    node(record, id).status
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_work_and_end_work_run_end_to_end_against_fakes() {
    use deckpilot_core::graph::NodeStatus::{Failed, Succeeded};

    let f = fixture().await;
    let [start, end] = [f.profile.workflows[0].id, f.profile.workflows[1].id];

    // Start Work: Git and Docker are checked, the services start, the
    // editor cannot open (unbound), the run continues to the summary.
    let run = f.agent.start_workflow(start).await.unwrap();
    let record = f.agent.wait_finished(run.id).await;
    assert_eq!(
        record.summary.state,
        ExecutionState::Succeeded,
        "{record:#?}"
    );
    for id in [
        "project",
        "git",
        "docker",
        "available",
        "services",
        "ready",
        "summary",
    ] {
        assert_eq!(status(&record, id), Succeeded, "{id}: {record:#?}");
    }
    assert_eq!(node(&record, "git").output["branch"], "feature/e2e");
    assert_eq!(node(&record, "docker").output["available"], "true");
    let editor = node(&record, "editor");
    assert_eq!(editor.status, Failed);
    assert!(
        editor
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("dev.editor")),
        "{editor:#?}"
    );
    let summary = &node(&record, "summary").output["summary"];
    assert!(
        summary.starts_with("4 succeeded, 1 failed (editor)"),
        "{summary}"
    );
    let calls = fakes::calls(&f.docker_log);
    assert!(
        calls
            .iter()
            .any(|c| c.iter().any(|a| a == "compose") && c.iter().any(|a| a == "up")),
        "{calls:?}"
    );

    // End Work: the changes are listed, the services stopped, summarized.
    let run = f.agent.start_workflow(end).await.unwrap();
    let record = f.agent.wait_finished(run.id).await;
    assert_eq!(
        record.summary.state,
        ExecutionState::Succeeded,
        "{record:#?}"
    );
    for id in [
        "project",
        "changes",
        "services",
        "configured",
        "stop",
        "done",
        "summary",
    ] {
        assert_eq!(status(&record, id), Succeeded, "{id}: {record:#?}");
    }
    assert_eq!(node(&record, "changes").output["changed"], "1");
    let summary = &node(&record, "summary").output["summary"];
    assert!(summary.starts_with("4 succeeded ·"), "{summary}");
    let calls = fakes::calls(&f.docker_log);
    assert!(
        calls
            .iter()
            .any(|c| c.iter().any(|a| a == "compose") && c.iter().any(|a| a == "down")),
        "{calls:?}"
    );

    // Both runs are in the history, newest first, and stored.
    let runs = f.agent.runs(start).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].trigger, deckpilot_core::ExecutionTrigger::App);
    let stored = f
        .agent
        .store()
        .execution(record.summary.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.summary.state, ExecutionState::Succeeded);
    assert_eq!(stored.nodes.len(), record.nodes.len());
}
